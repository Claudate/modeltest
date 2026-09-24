#!/usr/bin/env bash
# 打 Windows 绿色目录版。先读 docs/t54-desktop-package-anti-stale.md，任何一门失败都不得产出 dist/。
# 与 scripts/package-app.sh 平行对齐 T54 R1-R7，防旧硬门槛不因平台放松。
# 设计为在 Windows runner（Git Bash，MSVC host）上执行；不要在本机 macOS 上跑（无 MSVC 工具链）。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DIST="$ROOT/dist"
WIN_DIR="$DIST/ModelTest-win"
BIN_EXE="$WIN_DIR/model-test.exe"
ZIP="$DIST/ModelTest-windows-x64.zip"
RULES="docs/t54-desktop-package-anti-stale.md"

fail() {
  echo "ERROR: $1" >&2
  exit 1
}

# R1 — 规则文件必须在场，改规则立刻能阻断打包
[[ -f "$RULES" ]] || fail "ANTI_STALE_RULES_MISSING"
grep -q "# T54" "$RULES" || fail "ANTI_STALE_RULES_MISSING"
echo "==> T54 rules present: $RULES"

# 平台守卫：本脚本针对 Windows（MSVC, Git Bash）构建,没有 mingw 的机器即使
# 是 Linux/mac 也因为缺少 x86_64-pc-windows-gnu 工具链而无法产出可用 exe。
# cargo build 默认 target 即 host：windows-latest = x86_64-pc-windows-msvc，
# 产物在 target/release/model-test.exe，无需 --target。
case "$(uname -s)" in
  MINGW*|MSYS*) : ;;   # Windows Git Bash / MSYS2
  *) fail "package-win.sh 只在 Windows Git Bash(MINGW/MSYS)上运行,当前 $(uname -s)" ;;
esac

# R2 — 清上一轮 dist，避免把旧 exe 当成功产物
rm -rf "$DIST"
mkdir -p "$DIST"

echo "==> cargo clean -p model-test"
cargo clean -p model-test || fail "CLEAN_FAIL"

echo "==> cargo build --release"
cargo build --release || fail "BUILD_FAIL"

SRC_EXE="$ROOT/target/release/model-test.exe"
[[ -f "$SRC_EXE" ]] || fail "BUILD_FAIL: missing $SRC_EXE"

SRC_SHA="$(sha256sum "$SRC_EXE" | awk '{print $1}')"
echo "==> release exe sha256=$SRC_SHA"

mkdir -p "$WIN_DIR"
cp "$SRC_EXE" "$BIN_EXE"

# R7 — 拷贝后逐字节比对，挡住拷错/拷旧（Windows 无 codesign 改写，比对就是对最终产物）
BUNDLE_SHA="$(sha256sum "$BIN_EXE" | awk '{print $1}')"
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

SOURCE_SHA="$(cat Cargo.toml Cargo.lock src/*.rs | sha256sum | awk '{print $1}')"
PACKAGED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

# R4 — 写入打包元数据；Windows 没有 Info.plist，版本与 git 溯源都落在这一个文件里
cat > "$WIN_DIR/Pack-manifest.json" <<EOF
{
  "platform": "windows-x86_64",
  "packaged_at": "${PACKAGED_AT}",
  "git_sha": "${GIT_SHA}",
  "git_dirty": ${GIT_DIRTY},
  "source_sha256": "${SOURCE_SHA}",
  "binary_sha256": "${SRC_SHA}",
  "version": "${VERSION}",
  "bundle_version": "${GIT_SHORT}"
}
EOF
[[ -f "$WIN_DIR/Pack-manifest.json" ]] || fail "MANIFEST_MISSING"

# R3 — 产物必须严格新于源。Git Bash 的 stat 是 GNU coreutils，用 -c %Y 而非 mac 的 -f %m。
NEWEST_SRC=0
while IFS= read -r f; do
  mt="$(stat -c %Y "$f")"
  if (( mt > NEWEST_SRC )); then
    NEWEST_SRC=$mt
  fi
done < <(find src -name '*.rs' -print; printf '%s\n' Cargo.toml Cargo.lock)

BIN_MT="$(stat -c %Y "$BIN_EXE")"
if (( BIN_MT <= NEWEST_SRC )); then
  fail "STALE_ARTIFACT: exe mtime $BIN_MT <= source mtime $NEWEST_SRC"
fi
echo "==> R3 mtime ok (exe=$BIN_MT source=$NEWEST_SRC)"

# R5 — zip 内嵌 manifest。Git Bash 可能没有 bsdtar 的 zip 支持（tar -a 在 Windows
# 上不一定按扩展名推断 zip），优先用 zip 命令；没有则退回 PowerShell Compress-Archive。
# zip 内条目以 ModelTest-win/ 为根，解压即得绿色目录。
rm -f "$ZIP"
if command -v zip >/dev/null 2>&1; then
  (cd "$DIST" && zip -qr "$(basename "$ZIP")" ModelTest-win)
elif command -v powershell >/dev/null 2>&1; then
  powershell -NoProfile -Command "Compress-Archive -Path '$WIN_DIR' -DestinationPath '$ZIP' -Force"
else
  fail "ZIP_TOOL_MISSING: 无 zip 或 powershell"
fi
if ! unzip -Z1 "$ZIP" | grep -F "ModelTest-win/Pack-manifest.json" >/dev/null; then
  fail "MANIFEST_MISSING: zip 未包含 Pack-manifest.json"
fi

echo "==> packed $WIN_DIR"
echo "==> zip    $ZIP"
echo "==> manifest:"
cat "$WIN_DIR/Pack-manifest.json"
echo
echo "解压即用：unzip $ZIP"