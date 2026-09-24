## ModelTest __VERSION__

中转站模型验证工具：填写 OpenAI 兼容的 Base URL + Token，获取模型列表并一键校验可用性。

### 下载

| 平台 | 产物 | 说明 |
|------|------|------|
| macOS (Apple Silicon) | `ModelTest-macos-arm64.zip` | 解压后打开 `ModelTest.app`；App 未签名，macOS 若拦截请右键 → 打开 |
| Windows (x64) | `ModelTest-windows-x64.zip` | 解压 `ModelTest-win` 目录，运行 `model-test.exe` |

### 校验

```sh
# macOS
shasum -a 256 ModelTest-macos-arm64.zip
# Windows / PowerShell
Get-FileHash ModelTest-windows-x64.zip -Algorithm SHA256
```

压缩包内含 `Pack-manifest.json`，记录 build 时的 git commit、是否含未提交改动、源文件与二进制的 SHA-256，可对照本仓库对应 tag 自证来源。

### 使用

1. 填请求地址（可点「常用」预设，或直接贴完整 `/chat/completions` 地址，程序自动剥尾）
2. 填 Token（仅保存在本机，界面只显示掩码）
3. 「获取模型」拉取列表
4. 勾选要测的模型（不勾选 = 全部），「验证模型」出结果