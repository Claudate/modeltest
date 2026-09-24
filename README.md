# 中转站模型验证工具

原生 macOS 桌面程序（egui/eframe）。填中转站 Base URL 和 Token，拉取该 Token 下的模型，勾选要测的模型（不勾选 = 全部），用一次极短 chat completion 判断是否可用。

## 运行（开发）

```bash
cargo run
```

地址填 OpenAI 兼容前缀，例如 `https://your-gateway/v1`。贴完整 `/v1/chat/completions` 也可以，程序会剥掉尾巴。

本机只保存**请求地址 + Token** 这一组连接配置，写到
`~/Library/Application Support/model-test/endpoints.json`（权限 0600，Token 不以明文出现在界面上，只显示 `sk-…cdef` 掩码）。
模型清单不落盘：验证结果只在当前这一轮显示，换个 Token 就不成立了。
下次打开会自动填入最近一次用的地址和 Token；「已保存的连接」里每条都能「使用」或「删除」。
顶部搜索框按地址过滤已保存的连接。

## 打包（唯一入口）

```bash
./scripts/package-app.sh
```

脚本会先读 `docs/t54-desktop-package-anti-stale.md` 的硬门槛：清 `dist/`、`cargo clean -p model-test`、release 重建、拷贝后 SHA-256 比对、mtime 检查、写入 `Pack-manifest.json`、打 zip。

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
