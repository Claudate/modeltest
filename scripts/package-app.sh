#!/usr/bin/env bash
# 打 macOS .app。先读 docs/t54-desktop-package-anti-stale.md，任何一门失败都不得产出 dist/。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

APP_NAME="ModelTest"
BIN_NAME="model-test"
DIST="$ROOT/dist"
APP="$DIST/${APP_NAME}.app"
MACOS_DIR="$APP/Contents/MacOS"
CONTENTS="$APP/Contents"
ZIP="$DIST/${APP_NAME}-macos-arm64.zip"
RULES="docs/t54-desktop-package-anti-stale.md"

fail() {
  echo "ERROR: $1" >&2
  exit 1
}

# R1 — 规则文件必须在场，改规则立刻能阻断打包
[[ -f "$RULES" ]] || fail "ANTI_STALE_RULES_MISSING"
grep -q "# T54" "$RULES" || fail "ANTI_STALE_RULES_MISSING"
echo "==> T54 rules present: $RULES"

# R2 — 清上一轮 dist，避免把旧 .app 当成功产物
rm -rf "$DIST"
mkdir -p "$DIST"

echo "==> cargo clean -p model-test"
cargo clean -p model-test || fail "CLEAN_FAIL"

echo "==> cargo build --release"
cargo build --release || fail "BUILD_FAIL"

SRC_BIN="$ROOT/target/release/$BIN_NAME"
[[ -x "$SRC_BIN" ]] || fail "BUILD_FAIL: missing $SRC_BIN"

SRC_SHA="$(shasum -a 256 "$SRC_BIN" | awk '{print $1}')"
echo "==> release binary sha256=$SRC_SHA"

mkdir -p "$MACOS_DIR" "$CONTENTS/Resources"
cp "$SRC_BIN" "$MACOS_DIR/$BIN_NAME"
chmod +x "$MACOS_DIR/$BIN_NAME"

# R7 — codesign 前逐字节比对，挡住拷错/拷旧
BUNDLE_SHA="$(shasum -a 256 "$MACOS_DIR/$BIN_NAME" | awk '{print $1}')"
if [[ "$BUNDLE_SHA" != "$SRC_SHA" ]]; then
  fail "STALE_BINARY: bundle $BUNDLE_SHA != release $SRC_SHA"
fi
echo "==> R7 copy match ok"

VERSION="$(awk -F '"' '/^version *=/ {print $2; exit}' Cargo.toml)"
[[ -n "$VERSION" ]] || VERSION="0.0.0"
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  GIT_SHA="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
  GIT_SHORT="$(git describe --tags --always 2>/dev/null || git rev-parse --short HEAD)"
  if [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
    GIT_DIRTY=true
  else
    GIT_DIRTY=false
  fi
else
  GIT_SHA="unknown"
  GIT_SHORT="unknown"
  GIT_DIRTY=true
fi

# R6
cat > "$CONTENTS/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>zh_CN</string>
  <key>CFBundleExecutable</key>
  <string>${BIN_NAME}</string>
  <key>CFBundleIdentifier</key>
  <string>local.model-test.app</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>中转站模型验证工具</string>
  <key>CFBundleDisplayName</key>
  <string>中转站模型验证工具</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${GIT_SHORT}</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

SOURCE_SHA="$(cat Cargo.toml Cargo.lock src/*.rs | shasum -a 256 | awk '{print $1}')"
PACKAGED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

# R4 — 先写 manifest，再签；签名不改这份 JSON
cat > "$CONTENTS/Pack-manifest.json" <<EOF
{
  "packaged_at": "${PACKAGED_AT}",
  "git_sha": "${GIT_SHA}",
  "git_dirty": ${GIT_DIRTY},
  "source_sha256": "${SOURCE_SHA}",
  "binary_sha256": "${SRC_SHA}",
  "version": "${VERSION}",
  "bundle_version": "${GIT_SHORT}"
}
EOF
[[ -f "$CONTENTS/Pack-manifest.json" ]] || fail "MANIFEST_MISSING"

# ad-hoc 签名，避免未封印 bundle 直接打不开；签名后不再用 R7 比哈希
if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep -s - "$APP" >/dev/null
  echo "==> ad-hoc codesign done"
fi

# R3 — 产物必须严格新于源
NEWEST_SRC=0
while IFS= read -r f; do
  mt="$(stat -f %m "$f")"
  if (( mt > NEWEST_SRC )); then
    NEWEST_SRC=$mt
  fi
done < <(find src -name '*.rs' -print; printf '%s\n' Cargo.toml Cargo.lock)

BIN_MT="$(stat -f %m "$MACOS_DIR/$BIN_NAME")"
if (( BIN_MT <= NEWEST_SRC )); then
  fail "STALE_ARTIFACT: binary mtime $BIN_MT <= source mtime $NEWEST_SRC"
fi
echo "==> R3 mtime ok (bin=$BIN_MT source=$NEWEST_SRC)"

# R5 — 不用 grep -q：pipefail 下 unzip 会被 SIGPIPE 打成假失败
rm -f "$ZIP"
COPYFILE_DISABLE=1 ditto -c -k --norsrc --keepParent "$APP" "$ZIP"
ZIP_LIST="$(unzip -Z1 "$ZIP")"
echo "$ZIP_LIST" | grep -F "ModelTest.app/Contents/Pack-manifest.json" >/dev/null \
  || fail "MANIFEST_MISSING: zip 未包含 Pack-manifest.json"

echo "==> packed $APP"
echo "==> zip    $ZIP"
echo "==> manifest:"
cat "$CONTENTS/Pack-manifest.json"
echo
echo "open with: open \"$APP\""
