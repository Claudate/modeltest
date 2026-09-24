# T54 — 桌面打包防旧（anti-stale）

> 真源：本仓库本地唯一。`scripts/package-app.sh`（macOS）与 `scripts/package-win.sh`（Windows）每次执行**必须先读此文件并执行其中的硬规则**，否则打包失败。
> 本项目是 egui/eframe 原生桌面程序，**禁止打成网页、禁止用浏览器打开交付**。

## 问题

打包脚本 exit 0 但产物仍是旧 app：`cargo build` 吃 cache 未重编，`target/release/model-test` mtime 未变，用户双击看到旧 UI / 旧逻辑。

## 规则（`package-app.sh` / `package-win.sh` 必须执行的硬门槛）

同一套门槛两平台脚本平行实现；平台差异只在命令细节（`shasum`→`sha256sum`、`ditto`→`tar -a`、`stat -f %m`→`stat -c %Y`），检查语义逐条对齐。

### R1 — 每次必须先读本文件

- 脚本开头 `grep -q "# T54" docs/t54-desktop-package-anti-stale.md`，缺失则 `exit 1`（`ANTI_STALE_RULES_MISSING`）。
- 规则变更能立即阻断打包，不靠人记。

### R2 — 全量重建本 crate（不靠 cargo cache 判断是否要重编）

- 每次打包先清 `dist/`，再 `cargo clean -p model-test`，然后 `cargo build --release`。
- 保证主二进制 mtime 必然刷新。
- macOS 产物取 `target/release/model-test`，Windows 取 `target/release/model-test.exe`；都在各自 runner 的原生 target 上构建，不加 `--target`。

### R3 — 产物 mtime 必须晚于仓库源 mtime

- 打包完成后收集 `src/**/*.rs`、`Cargo.toml`、`Cargo.lock` 中最新的 mtime，与产物二进制 mtime 比较（mac：`dist/ModelTest.app/Contents/MacOS/model-test`；win：`dist/ModelTest-win/model-test.exe`）。
- 若产物 mtime ≤ 源 mtime（含同秒），`STALE_ARTIFACT`，exit 1。

### R4 — 写入打包元数据

- macOS：`ModelTest.app/Contents/Pack-manifest.json`；Windows：`ModelTest-win/Pack-manifest.json`（win 附 `platform: "windows-x86_64"`）。两者必须包含：
  - `packaged_at`：ISO 时间戳
  - `git_sha`：`git rev-parse HEAD`（无 git 则 `unknown`）
  - `git_dirty`：是否有未提交改动
  - `source_sha256`：对 `Cargo.toml` + `Cargo.lock` + 全部 `src/**/*.rs` 做 sha256
  - `binary_sha256`：签名前、从 `target/release/model-test` 拷进 bundle 时的哈希
- 用户可 `cat dist/ModelTest.app/Contents/Pack-manifest.json` 自检是否最新。

### R5 — ZIP 内嵌 manifest

- `dist/ModelTest-macos-arm64.zip` 内必须含 `ModelTest.app/Contents/Pack-manifest.json`。
- `dist/ModelTest-windows-x64.zip` 内必须含 `ModelTest-win/Pack-manifest.json`。

### R6 — Info.plist 版本可溯源

- `CFBundleVersion` 用 `git describe --tags --always`（无 tag 则 short sha）。
- `CFBundleShortVersionString` 与 `Cargo.toml` 的 `version` 一致。

### R7 — 逐字节比对（拷贝后、codesign 前）

- `dist/ModelTest.app/Contents/MacOS/model-test` 的 SHA-256 必须等于本轮 `target/release/model-test`。
- 不一致即 `STALE_BINARY`，exit 1。codesign 会改写 Mach-O，比对只能在签名之前。

## 失败模式

| 失败信号 | 含义 |
|----------|------|
| `ANTI_STALE_RULES_MISSING` | 本文件缺失或损坏 |
| `STALE_ARTIFACT` | 产物 mtime ≤ 源 mtime |
| `STALE_BINARY` | bundle 内二进制 ≠ 本轮 release 产物 |
| `MANIFEST_MISSING` | pack manifest 未写入 |
| `CLEAN_FAIL` | cargo clean 失败 |
| `BUILD_FAIL` | cargo build --release 失败 |

## 验证

```bash
cat dist/ModelTest.app/Contents/Pack-manifest.json
git rev-parse HEAD
shasum -a 256 dist/ModelTest.app/Contents/MacOS/model-test
```
