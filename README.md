# 中转站模型验证工具（ModelTest）

原生桌面程序（egui/eframe），支持 **macOS（Apple Silicon）** 和 **Windows（x64）**。
填中转站 Base URL + Token，拉取该 Token 下的模型，勾选要测的模型（不勾选 = 全部），
用一次极短的 chat completion 判断哪些模型当前可用、延迟多少。

## 特性

- **OPENAI 兼容**：地址填 `https://your-gateway/v1` 这类前缀即可；贴完整
  `/v1/chat/completions`、`/models` 尾巴程序也会自动剥掉。
- **一键探测**：`GET {base}/models` 拉列表，对每个模型发 `{"model": …,
  messages:[…], "max_tokens": 1, "stream": false}` 判可用；并发 4、超时 30s。
- **灵活勾选**：勾选要测的模型（不勾选 = 全部）。列表自带「全选/全不选」批量操作。
- **常用预设**：OpenAI / OpenRouter / DeepSeek / Moonshot / 智谱 GLM 一键填入。
- **Token 不裸奔**：界面只显示掩码（`sk-…cdef`，头 3 尾 4，短 token 整串打点）。
- **本地管理**：已保存连接按 `base_url` 去重、移到最前，顶部搜索框按地址过滤，
  每条都能「使用」「删除」；下次打开自动回填最近一条。
- **不存模型清单**：模型属于”这次请求”的结果，换个 Token 就不成立，一律不落盘。

## 下载安装

GitHub [Releases](https://github.com/Claudate/modeltest/releases) 页按平台下载对应 zip（打 tag 自动产出）：

| 平台 | 产物 | 说明 |
|------|------|------|
| macOS（Apple Silicon，M1 及以上） | `ModelTest-macos-arm64.zip` | 解压后打开 `ModelTest.app` |
| Windows（x64） | `ModelTest-windows-x64.zip` | 解压 `ModelTest-win` 目录，双击 `model-test.exe`（绿色版，不写注册表） |

每个包内都含 `Pack-manifest.json`，记录了构建时的 git commit、是否有未提交改动、源文件与二进制的 SHA-256，可对照仓库对应 tag 自证来源。

校验下载完整性（可选）：

```sh
# macOS
shasum -a 256 ModelTest-macos-arm64.zip
# Windows / PowerShell
Get-FileHash ModelTest-windows-x64.zip -Algorithm SHA256
```

> **未签名提示**：目前产物是 ad-hoc 签名 / 未签名，首次运行可能被系统拦截——
> macOS 报「无法验证开发者」时选 **右键 → 打开** → 再点「打开」；
> Windows SmartScreen 提示时点「更多信息 → 仍要运行」。

## 运行（开发）

```bash
cargo run        # 开发调试
cargo test       # 跑单元测试（地址规范化、模型解析、存储与权限、UTC 时间往返…）
```

本机只保存**请求地址 + Token** 这一组连接配置，写到
`endpoints.json`（权限 0600，Token 不以明文出现在界面上）。

配置文件位置随平台：

| 平台 | 路径 |
|------|------|
| macOS | `~/Library/Application Support/model-test/endpoints.json` |
| Windows | `%LOCALAPPDATA%\model-test\endpoints.json`（取不到时退回 `%APPDATA%`） |

开发/测试可用环境变量 **`MODEL_TEST_DATA_DIR`** 改数据目录（避免污染真实配置）。

## 使用流程

1. 填请求地址（常用预设 + 一键，或直接粘贴整行 `/chat/completions`）
2. 填 Token
3. 「获取模型」拉取列表
4. 勾选要测的模型（不勾选 = 全部），「验证模型」出结果

日志只在窗口打开期间积累；结果只在这一轮显示，换 Token 就不成立了。

## 目录结构

```
src/
  main.rs          桌面入口：三层布局（顶栏 / 滚动流 / 日志），egui 逻辑
  model_service.rs  HTTP 客户端：tokio 运行时跑网络，UI 线程只 try_recv 收消息
  store.rs          本地配置存取、Token 掩码、时区换算（不依赖 chrono）
scripts/
  package-app.sh    macOS .app + zip（唯一打包入口，走 T54 防旧门槛）
  package-win.sh    Windows 绿色目录 zip（与 mac 对齐 T54 R1-R7）
.github/workflows/
  release.yml       打 tag 自动双端打包并发布 GitHub Release
docs/
  t54-desktop-package-anti-stale.md  打包防旧硬规则（产品出入口）
```

## 打包（唯一入口）

```bash
./scripts/package-app.sh    # macOS
```

脚本会先读 `docs/t54-desktop-package-anti-stale.md` 的硬门槛：清 `dist/`、
`cargo clean -p model-test`、release 重建、拷贝后 SHA-256 比对、mtime 检查、
写入 `Pack-manifest.json`、打 zip。任何一门失败都不得产出 `dist/`。

产物：

- `dist/ModelTest.app`
- `dist/ModelTest-macos-arm64.zip`

打开：

```bash
open dist/ModelTest.app
```

核对本次是不是刚打的包：

```bash
cat dist/ModelTest.app/Contents/Pack-manifest.json
git rev-parse HEAD
```

Windows 打包在 Windows 构建机（Git Bash + MSVC）上跑，不在本机 macOS 打：

```bash
bash scripts/package-win.sh   # 产出 dist/ModelTest-win/model-test.exe + ModelTest-windows-x64.zip
```

## 发布（GitHub Actions）

打 tag（`v*`）即触发双端打包并发布 Release：

```bash
git tag v0.1.0 && git push origin v0.1.0
```

`release.yml` 并行构建 mac（arm64）/ win（x64）→ 聚合上传 GitHub Release，
Release 里会带着各平台产物 + SHA-256 校验和 + 使用说明。tag 与
`Cargo.toml` 的 `version` 不一致会被拒绝发布。

App 为 ad-hoc 签名：从网络下载解压后 macOS 可能拦截，右键 → 打开即可。

## 验证当前产物是否最新

```bash
cat dist/ModelTest.app/Contents/Pack-manifest.json   # packaged_at / git_sha / source_sha256 / binary_sha256
git rev-parse HEAD
shasum -a 256 dist/ModelTest.app/Contents/MacOS/model-test
```

`Pack-manifest.json` 记录了 build 时的 git commit、是否 dirty、源码与二进制的
SHA-256，可对照本仓库相应 tag 自证”这是哪次提交打的包”。

## 已知限制

- **Windows 时区显示**：程序通过 `/bin/date +%z` 获取本地时区偏移，Windows 上取不到，
  保存时间可能显示为 UTC（比本地早 8 小时等）。数据本身不受影响，读回来仍是本机时间。
- **未签名分发**：产物未接 Apple Developer / Windows 证书签名，见上方「签名提示」；
  适用于内部工具与个人分发，大规模对外分发建议接入正式签名。
- **只打包当前架构**：mac 仅 Apple Silicon（arm64），Windows 仅 x64；
  Intel Mac / Window ARM 用户暂未覆盖（换 X64 runner / 交叉编译即可扩展）。