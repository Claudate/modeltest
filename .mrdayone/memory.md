# Mr. Day One 项目记忆

<!-- 由 remember 工具维护。你可以直接编辑这个文件：本地记忆为空时（换机器/清数据/重装）会从这里读回。 -->

## 核心记忆（每轮常驻）

- [你记的] model-test 产品约定：填 Base URL + Token 拉 /v1/models；勾选指定模型验证，一个都不选则验证全部。normalize_base_url 会剥掉 /models 和 /chat/completions 尾巴。

## 项目目标与约定（锁定，防偏航）

- [据本轮归纳] 用户认为当前UI设计很差，要求按正式软件标准优化界面与交互，不只修字体。
- [运行中记下] model-test 产品约定：填 Base URL + Token 拉 /v1/models；勾选指定模型验证，一个都不选则验证全部。normalize_base_url 会剥掉 /models 和 /chat/completions 尾巴。

## 项目事实（实时扶正）

- [据本轮归纳] model-test 是 egui 原生桌面程序，唯一入口 ./scripts/package-app.sh。每次打包前必读 docs/t54-desktop-package-anti-stale.md (R1-R7)。禁止浏览器打开。测试
- [据本轮归纳] 测过能用的模型要持久化并支持搜索，保存路径为 Application Support/model-test/verified.json。
- 项目档案: Rust 项目「model-test」（cargo build / run / test）。

## 踩过的坑（不再犯）

- [运行中记下] model-test 是 egui 原生桌面程序，不是网页。唯一打包入口是 ./scripts/package-app.sh，每次必须先读 docs/t54-desktop-package-anti-stale.md（R1–R7：清 dist、cargo clean -p model-test、mtime/SHA 比对、Pack-manifest.json）。禁止用浏览器打开交付。
