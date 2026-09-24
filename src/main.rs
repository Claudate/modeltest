//! 桌面入口：egui 原生窗口（不是网页）。
//!
//! ## 上一版为什么整块空白
//! 旧写法把「连接卡片 / 统计条 / 模型表格 / 已保存卡片」分别塞进会做高度测量
//! 的子面板，日志底栏一展开就把中央剩余高度吃光，中间三块被压成 0 高度——窗口
//! 里只剩顶栏和日志，看起来像“什么都没渲染”。
//!
//! ## 这一版的布局契约（不要再改回多层面板）
//! 只有三层，且中央不做任何高度测量：
//!   - `TopBottomPanel::top`：标题栏，`exact_height` 固定高。
//!   - `CentralPanel` + **一个** `ScrollArea`：所有业务内容都在这条滚动流里，
//!     内容多就滚动，永远不会被压成 0。
//!   - `TopBottomPanel::bottom`：日志，展开时用固定内容高度，不靠内容撑高。
//!
//! ## 借用规则（Rust 侧的坑，别踩回去）
//! `card()` 收两个闭包（标题行 + 内容）。Rust 不允许两个闭包同时可变借用
//! `self`（E0524），所以约定：**标题行闭包只返回一个 action，不碰 self**；
//! 内容闭包可以改 self；`card` 把 action 原样交回，由调用方在闭包全部结束后
//! 才落地。这也符合 egui「本帧记录、下帧生效」的写法。
//!
//! ## 数据流
//! 填 Base URL + Token → `GET {base}/models` → 勾选（不勾 = 全部，产品约定）→
//! 极短 chat completion 判可用。**落盘的只有「请求地址 + Token」这一组连接配置**
//! （`endpoints.json`，0600），模型清单一律不写盘——模型属于这次请求的结果，
//! 换个 Token 就不成立了。网络跑在
//! `model_service` 自己的 tokio 运行时里，UI 线程只 `try_recv` 收消息；只有任务
//! 进行中才申请重绘，空闲时不吃 CPU。

use eframe::egui;
use egui::{
    vec2, Align, Color32, FontData, FontDefinitions, FontFamily, FontId, Grid, Layout, Margin,
    RichText, ScrollArea, Sense, Stroke, Ui,
};
use std::time::Duration;

mod model_service;
mod store;
use model_service::{
    ids_to_test, normalize_base_url, ModelRecord, ModelService, ServiceMsg, TestStatus,
};
use store::{display_time, mask_token, now_iso_utc, EndpointStore, SavedEndpoint};

// ---------------------------------------------------------------- 语义色 token
// 近白画布 + 白卡片 + 锌灰边框 + 单一蓝色主操作；状态色只出现在徽标和统计数字。
// 裸 hex 只允许出现在这一段 token 定义里，业务代码一律消费这些常量。
const CANVAS: Color32 = Color32::from_rgb(0xF6, 0xF7, 0xF9);
const CARD: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const INPUT_BG: Color32 = Color32::from_rgb(0xFC, 0xFC, 0xFD);
const BORDER: Color32 = Color32::from_rgb(0xE4, 0xE4, 0xE7);
const FAINT: Color32 = Color32::from_rgb(0xFA, 0xFA, 0xFA);
const FG: Color32 = Color32::from_rgb(0x11, 0x11, 0x13);
const MUTED: Color32 = Color32::from_rgb(0x71, 0x71, 0x7A);
const NEUTRAL_FG: Color32 = Color32::from_rgb(0x52, 0x52, 0x5B);
const NEUTRAL_SOFT: Color32 = Color32::from_rgb(0xF4, 0xF4, 0xF5);
const ACCENT: Color32 = Color32::from_rgb(0x25, 0x63, 0xEB);
const ACCENT_HOVER: Color32 = Color32::from_rgb(0x1D, 0x4E, 0xD8);
const ACCENT_ACTIVE: Color32 = Color32::from_rgb(0x1E, 0x40, 0xAF);
const ACCENT_SOFT: Color32 = Color32::from_rgb(0xEF, 0xF6, 0xFF);
const OK: Color32 = Color32::from_rgb(0x05, 0x96, 0x69);
const OK_SOFT: Color32 = Color32::from_rgb(0xEC, 0xFD, 0xF5);
const BAD: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);
const BAD_SOFT: Color32 = Color32::from_rgb(0xFE, 0xF2, 0xF2);

/// 顶栏高度（固定，避免标题文字撑高）。
const HEADER_HEIGHT: f32 = 52.0;
/// 日志展开时正文区高度。底栏不能靠内容撑高，否则抢中央空间 → 上一版的空白事故。
const LOG_BODY_HEIGHT: f32 = 88.0;

/// 常用中转站预设：只给地址，Token 必须用户自己填（不写死任何密钥）。
/// 这是可扩展的数据表——加站点只改这里，不改界面逻辑。
const PRESETS: [(&str, &str); 5] = [
    ("OpenAI", "https://api.openai.com/v1"),
    ("OpenRouter", "https://openrouter.ai/api/v1"),
    ("DeepSeek", "https://api.deepseek.com/v1"),
    ("Moonshot", "https://api.moonshot.cn/v1"),
    ("智谱 GLM", "https://open.bigmodel.cn/api/paas/v4"),
];

/// 模型列表标题行的批量操作意图。由标题行闭包返回，调用方稍后落地（见文件头借用规则）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bulk {
    None,
    SelectVisible,
    ClearVisible,
}

/// 已保存连接行上的操作。表格闭包里持有 `&self.endpoints`，不能同时增删，
/// 所以只记录意图，等 Grid 画完再落地。
#[derive(Clone, Copy, PartialEq, Eq)]
enum EndpointAction {
    /// 把这条地址 + Token 填回输入框。
    Use,
    /// 从本机删除这条连接。
    Remove,
}

// ---------------------------------------------------------------- 字体 / 主题

/// egui 自带字体只有拉丁字形，中文必须挂系统 CJK 字体，否则整页方框。
/// 一个候选都没找到时不 panic：继续用默认字体，界面至少不是空白。
fn install_cjk_fonts(ctx: &egui::Context) {
    // (路径, ttc 内的字面序号)。Hiragino Sans GB 是 macOS 上最稳的中文 UI 字体。
    let candidates: [(&str, u32); 4] = [
        ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
        ("/System/Library/Fonts/STHeiti Light.ttc", 0),
        ("/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0),
        ("/Library/Fonts/Arial Unicode.ttf", 0),
    ];
    for &(path, index) in candidates.iter() {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert("cjk".to_owned(), data);
        // 两个族都要挂：正文走 Proportional，模型 ID / 日志走 Monospace。
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push("cjk".to_owned());
            }
        }
        ctx.set_fonts(fonts);
        return;
    }
    eprintln!("warning: 未找到系统中文字体，中文可能显示为方框");
}

/// 统一视觉 token：白卡、细边框、蓝色选中环，替掉 egui 默认的灰底观感。
fn install_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::light();
    v.panel_fill = CANVAS;
    v.extreme_bg_color = INPUT_BG;
    v.window_fill = CARD;
    v.faint_bg_color = FAINT;
    v.override_text_color = Some(FG);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = Stroke::new(1.0_f32,ACCENT);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = CARD;
    w.noninteractive.weak_bg_fill = CARD;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32,BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32,FG);
    w.inactive.weak_bg_fill = NEUTRAL_SOFT;
    w.inactive.bg_stroke = Stroke::new(1.0_f32,BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0_f32,NEUTRAL_FG);
    w.hovered.weak_bg_fill = CARD;
    w.hovered.bg_stroke = Stroke::new(1.0_f32,ACCENT);
    w.hovered.fg_stroke = Stroke::new(1.0_f32,FG);
    w.active.weak_bg_fill = ACCENT_SOFT;
    w.active.bg_stroke = Stroke::new(1.0_f32,ACCENT);
    w.active.fg_stroke = Stroke::new(1.0_f32,ACCENT_ACTIVE);
    w.open.weak_bg_fill = ACCENT_SOFT;
    w.open.bg_stroke = Stroke::new(1.0_f32,ACCENT);
    w.open.fg_stroke = Stroke::new(1.0_f32,ACCENT_ACTIVE);
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.spacing.item_spacing = vec2(10.0, 8.0);
        s.spacing.button_padding = vec2(11.0, 5.0);
    });
}

// ---------------------------------------------------------------- 小部件

/// 带标题行的白卡片。卡片自身不限高——高度由内容决定，交给外层 ScrollArea。
/// 返回 `(标题行 action, 内容闭包返回值)`，见文件头的借用规则。
fn card<T, R>(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    header: impl FnOnce(&mut Ui) -> T,
    body: impl FnOnce(&mut Ui) -> R,
) -> (T, R) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32,BORDER))
        .rounding(10.0)
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            let head = ui
                .horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        ui.label(RichText::new(title).size(13.0).strong().color(FG));
                        if !subtitle.is_empty() {
                            ui.label(RichText::new(subtitle).size(11.0).color(MUTED));
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(14.0);
                        header(ui)
                    })
                    .inner
                })
                .inner;
            ui.add_space(8.0);
            ui.separator();
            let inner = egui::Frame::none()
                .fill(CARD)
                .inner_margin(Margin::symmetric(14.0, 12.0))
                .show(ui, body)
                .inner;
            (head, inner)
        })
        .inner
}

/// 状态徽标：小胶囊。底色与文字色成对传入，保证正文级对比度。
/// 返回 Response，调用方可以挂 hover 说明。
fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) -> egui::Response {
    egui::Frame::none()
        .fill(bg)
        .rounding(6.0)
        .inner_margin(Margin::symmetric(8.0, 3.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ui.label(RichText::new(text).size(11.0).strong().color(fg));
            });
        })
        .response
}

fn status_pill(ui: &mut Ui, status: TestStatus) {
    let (text, fg, bg) = match status {
        TestStatus::Pending => ("待测", NEUTRAL_FG, NEUTRAL_SOFT),
        TestStatus::Testing => ("测试中", ACCENT, ACCENT_SOFT),
        TestStatus::Pass => ("可用", OK, OK_SOFT),
        TestStatus::Fail => ("失败", BAD, BAD_SOFT),
    };
    let _ = pill(ui, text, fg, bg);
}

/// 次要按钮：白底细边框，用于「清空选择 / 删除 / 清空日志」这类不抢眼的操作。
fn ghost_button(ui: &mut Ui, text: &str) -> bool {
    ui.add(
        egui::Button::new(RichText::new(text).size(12.0).color(NEUTRAL_FG))
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32,BORDER))
            .rounding(8.0),
    )
    .clicked()
}

/// 轻量按钮：浅蓝底，用于预设站点、全选/取消这类批量操作。
fn soft_button(ui: &mut Ui, text: &str) -> bool {
    ui.add(
        egui::Button::new(RichText::new(text).size(11.0).color(ACCENT))
            .fill(ACCENT_SOFT)
            .stroke(Stroke::new(1.0_f32,ACCENT_SOFT))
            .rounding(6.0),
    )
    .clicked()
}

/// 主操作按钮（实心蓝）。
///
/// 禁用态走中性灰并把 sense 收成 hover，避免“看着能点其实没反应”。
/// 启用态临时改写 widget 配色再交回 egui 自己的 Button，从而保留原生
/// hovered / clicked / focus 行为——手写矩形覆盖会慢一帧，是上一版的观感问题之一。
fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> bool {
    if !enabled {
        return ui
            .add(
                egui::Button::new(
                    RichText::new(text).size(13.0).strong().color(NEUTRAL_FG),
                )
                .fill(NEUTRAL_SOFT)
                .stroke(Stroke::new(1.0_f32,BORDER))
                .rounding(8.0)
                .min_size(vec2(104.0, 30.0))
                .sense(Sense::hover()),
            )
            .clicked();
    }
    let prev = ui.visuals().widgets.clone();
    {
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.weak_bg_fill = ACCENT;
        w.inactive.bg_stroke = Stroke::new(1.0_f32,ACCENT);
        w.inactive.fg_stroke = Stroke::new(1.0_f32,Color32::WHITE);
        w.hovered.weak_bg_fill = ACCENT_HOVER;
        w.hovered.bg_stroke = Stroke::new(1.0_f32,ACCENT_HOVER);
        w.hovered.fg_stroke = Stroke::new(1.0_f32,Color32::WHITE);
        w.active.weak_bg_fill = ACCENT_ACTIVE;
        w.active.bg_stroke = Stroke::new(1.0_f32,ACCENT_ACTIVE);
        w.active.fg_stroke = Stroke::new(1.0_f32,Color32::WHITE);
    }
    let clicked = ui
        .add(
            egui::Button::new(RichText::new(text).size(13.0).strong().color(Color32::WHITE))
                .min_size(vec2(104.0, 30.0))
                .rounding(8.0),
        )
        .clicked();
    ui.visuals_mut().widgets = prev;
    clicked
}

/// 统计格：小标签 + 大号数字。宽度靠 min_col_width 稳定，数值变化不抖布局。
fn stat_tile(ui: &mut Ui, label: &str, value: usize, color: Color32) {
    egui::Frame::none()
        .fill(FAINT)
        .stroke(Stroke::new(1.0_f32,BORDER))
        .rounding(8.0)
        .inner_margin(Margin::symmetric(10.0, 6.0))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                ui.label(RichText::new(label).size(10.0).color(MUTED));
                ui.label(
                    RichText::new(value.to_string())
                        .size(17.0)
                        .strong()
                        .color(color),
                );
            });
        });
}

/// 空状态 / 提示用的浅底信息块。
fn info_block(ui: &mut Ui, title: &str, lines: &str) {
    egui::Frame::none()
        .fill(FAINT)
        .stroke(Stroke::new(1.0_f32,BORDER))
        .rounding(8.0)
        .inner_margin(Margin::symmetric(14.0, 14.0))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
                ui.label(RichText::new(title).size(13.0).strong().color(FG));
                ui.label(RichText::new(lines).size(12.0).color(MUTED));
            });
        });
}

// ---------------------------------------------------------------- 应用状态

struct ModelTestApp {
    /// 地址与 Token 存在 service 里，UI 直接绑定它的 pub 字段。
    service: ModelService,
    /// 本机保存的连接（请求地址 + Token），不含任何模型信息。
    endpoints: EndpointStore,
    models: Vec<ModelRecord>,
    filter: String,
    log_open: bool,
    endpoints_open: bool,
    last_log: String,
}

impl ModelTestApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_cjk_fonts(&cc.egui_ctx);
        install_theme(&cc.egui_ctx);
        let mut app = Self {
            service: ModelService::new(),
            endpoints: EndpointStore::load(),
            models: Vec::new(),
            filter: String::new(),
            log_open: true,
            endpoints_open: false,
            last_log: String::new(),
        };
        // 启动即回填上次用的连接：最新一条排在最前，省得每次重粘 Token。
        if let Some(last) = app.endpoints.entries.first() {
            if app.service.base_url.is_empty() {
                app.service.base_url = last.base_url.clone();
            }
            if app.service.api_key.is_empty() {
                app.service.api_key = last.token.clone();
            }
        }
        let n = app.endpoints.entries.len();
        if n > 0 {
            app.last_log
                .push_str(&format!("已载入 {n} 个保存的连接，并填入最近一个。\n"));
        }
        app.last_log
            .push_str("就绪：填写请求地址与 Token，点「获取模型」。\n");
        app
    }

    /// 当前地址的规范化形式；地址还没填对时返回空串（只用于展示，不弹错）。
    fn current_base(&self) -> String {
        normalize_base_url(&self.service.base_url).unwrap_or_default()
    }

    /// 收网络消息，返回“任务是否仍在进行”（决定要不要继续申请重绘）。
    /// 过期 job 的消息一律丢弃：连点两次时旧响应不得覆盖新列表。
    fn poll(&mut self) -> bool {
        let mut running = self.service.is_running();
        while let Some(msg) = self.service.try_recv() {
            match msg {
                ServiceMsg::ListDone { job, models, log } => {
                    if job == self.service.current_job() {
                        self.models = models;
                        self.last_log.push_str(&log);
                        // 能拉到列表 = 地址与 Token 这一对是可用的，此刻就值得存。
                        self.persist_endpoint();
                        self.service.set_running(false);
                        running = false;
                    }
                }
                ServiceMsg::TestProgress { job, models } => {
                    if job == self.service.current_job() {
                        self.merge_test_results(models);
                    }
                }
                ServiceMsg::TestDone { job, models, log } => {
                    if job == self.service.current_job() {
                        self.merge_test_results(models);
                        // 验证跑完同样刷一次连接：地址 + Token 能打通就该留下。
                        self.persist_endpoint();
                        self.last_log.push_str(&log);
                        self.service.set_running(false);
                        running = false;
                    }
                }
            }
            if !self.service.is_running() {
                break;
            }
        }
        running
    }

    fn merge_test_results(&mut self, results: Vec<ModelRecord>) {
        for rec in results {
            match self.models.iter_mut().find(|m| m.id == rec.id) {
                Some(target) => {
                    target.status = rec.status;
                    target.latency_ms = rec.latency_ms;
                    target.detail = rec.detail;
                }
                // 验证途中列表被换掉：补一行，保证结果不丢。
                None => self.models.push(rec),
            }
        }
    }

    /// 把当前这组「请求地址 + Token」写盘。模型不落盘。
    ///
    /// 地址或 Token 为空（地址规范化失败也算空）时不写，避免存下一条打不开的记录；
    /// 已经是最新一条时也不重写，免得每次验证都刷时间戳、把列表顺序打乱。
    fn persist_endpoint(&mut self) {
        let base = self.current_base();
        let token = self.service.api_key.trim().to_owned();
        if base.is_empty() || token.is_empty() {
            return;
        }
        if self.endpoints.is_current(&base, &token) {
            return;
        }
        self.endpoints.upsert(SavedEndpoint {
            base_url: base.clone(),
            token,
            saved_at: now_iso_utc(),
        });
        if let Err(e) = self.endpoints.save() {
            self.last_log.push_str(&format!("保存连接失败: {e}\n"));
        } else {
            self.last_log.push_str(&format!("已保存连接 {base}\n"));
        }
    }

    /// 搜索框只过滤模型 ID；空查询 = 全部可见。
    fn visible_indices(&self) -> Vec<usize> {
        let q = self.filter.trim().to_ascii_lowercase();
        self.models
            .iter()
            .enumerate()
            .filter(|(_, m)| q.is_empty() || m.id.to_ascii_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }
}

// ---------------------------------------------------------------- 绘制

impl eframe::App for ModelTestApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.poll() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }

        let running = self.service.is_running();
        let log_lines = self.last_log.lines().count();
        let saved_count = self.endpoints.entries.len();

        // ---------------- 顶栏：固定高，不参与中央测量 ----------------
        egui::TopBottomPanel::top("header")
            .exact_height(HEADER_HEIGHT)
            .frame(
                egui::Frame::none()
                    .fill(CARD)
                    .inner_margin(Margin::symmetric(16.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                    ui.label(RichText::new("●").size(14.0).color(ACCENT));
                    ui.label(
                        RichText::new("中转站模型验证工具")
                            .size(15.0)
                            .strong()
                            .color(FG),
                    );
                    ui.label(
                        RichText::new("连通性与模型可用性检查")
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if running {
                            ui.spinner();
                        }
                        let _ = pill(
                            ui,
                            if running { "运行中" } else { "就绪" },
                            if running { ACCENT } else { NEUTRAL_FG },
                            if running { ACCENT_SOFT } else { NEUTRAL_SOFT },
                        );
                        pill(
                            ui,
                            &format!("已存连接 {saved_count}"),
                            NEUTRAL_FG,
                            NEUTRAL_SOFT,
                        )
                        .on_hover_text("本机只保存请求地址与 Token，不保存模型清单");
                    });
                });
            });

        // ---------------- 日志底栏：展开时用固定内容高度 ----------------
        egui::TopBottomPanel::bottom("log_bar")
            .frame(
                egui::Frame::none()
                    .fill(CARD)
                    .inner_margin(Margin::symmetric(16.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                    let arrow = if self.log_open { "▾" } else { "▸" };
                    let title = if log_lines == 0 {
                        format!("{arrow} 日志")
                    } else {
                        format!("{arrow} 日志（{log_lines} 行）")
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(title).size(12.0).strong().color(FG),
                            )
                            .fill(CARD)
                            .stroke(Stroke::new(1.0_f32,CARD))
                            .rounding(6.0),
                        )
                        .clicked()
                    {
                        self.log_open = !self.log_open;
                    }
                    if !self.log_open {
                        // 收起时把最后一行摊在标题后：滚动日志不额外占高度。
                        let last: String = self
                            .last_log
                            .lines()
                            .last()
                            .unwrap_or("暂无输出")
                            .chars()
                            .take(80)
                            .collect();
                        ui.label(RichText::new(last).size(11.0).color(MUTED));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ghost_button(ui, "清空") {
                            self.last_log.clear();
                        }
                    });
                });
                if self.log_open {
                    ui.add_space(4.0);
                    egui::Frame::none()
                        .fill(FAINT)
                        .stroke(Stroke::new(1.0_f32,BORDER))
                        .rounding(8.0)
                        .inner_margin(Margin::symmetric(10.0, 6.0))
                        .show(ui, |ui| {
                            ScrollArea::vertical()
                                .max_height(LOG_BODY_HEIGHT)
                                .auto_shrink([false, false])
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    let body = if self.last_log.is_empty() {
                                        "— 暂无输出 —".to_owned()
                                    } else {
                                        self.last_log.clone()
                                    };
                                    ui.label(
                                        RichText::new(body)
                                            .size(11.0)
                                            .monospace()
                                            .color(NEUTRAL_FG),
                                    );
                                });
                        });
                }
            });

        // ---------------- 中央：一条滚动流装下全部内容 ----------------
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(CANVAS)
                    .inner_margin(Margin::symmetric(16.0, 14.0)),
            )
            .show(ctx, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.draw_connection(ui, running);
                        ui.add_space(12.0);
                        self.draw_models(ui);
                        ui.add_space(12.0);
                        self.draw_endpoints(ui);
                        ui.add_space(4.0);
                    });
            });
    }
}

impl ModelTestApp {
    /// 卡片 1：地址 / 预设 / Token / 动作按钮。
    fn draw_connection(&mut self, ui: &mut Ui, running: bool) {
        card(
            ui,
            "连接配置",
            "OpenAI 兼容地址；贴完整 /chat/completions 也会自动剥尾",
            move |ui| {
                if running {
                    ui.label(RichText::new("任务进行中").size(11.0).color(ACCENT));
                }
            },
            |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
                ui.label(
                    RichText::new("请求地址（Base URL）")
                        .size(11.0)
                        .strong()
                        .color(NEUTRAL_FG),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.service.base_url)
                        .hint_text("https://your-gateway/v1")
                        .font(FontId::proportional(13.0))
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                    ui.label(RichText::new("常用").size(11.0).color(MUTED));
                    for (name, url) in PRESETS.iter() {
                        if soft_button(ui, name) {
                            self.service.base_url = (*url).to_owned();
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Token").size(11.0).strong().color(NEUTRAL_FG));
                ui.add(
                    egui::TextEdit::singleline(&mut self.service.api_key)
                        .hint_text("sk-...")
                        .password(true)
                        .font(FontId::proportional(13.0))
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                    let target = ids_to_test(&self.models).len();
                    let has_list = !self.models.is_empty();
                    if primary_button(ui, "获取模型", !running) {
                        self.last_log.push_str("开始获取模型列表\n");
                        self.service.list_models();
                    }
                    // 不勾选 = 验证全部，按钮可用性按这个口径算。
                    if primary_button(ui, "验证模型", !running && target > 0) {
                        self.last_log
                            .push_str(&format!("开始验证 {target} 个模型\n"));
                        let snapshot = self.models.clone();
                        self.service.test_models(&snapshot);
                    }
                    if ghost_button(ui, "清空选择") {
                        for m in self.models.iter_mut() {
                            m.selected = false;
                        }
                    }
                    let hint = if running {
                        "网络任务进行中，请稍候"
                    } else if !has_list {
                        "先获取模型列表"
                    } else if target == self.models.len() {
                        "未勾选 = 验证全部"
                    } else {
                        "只验证已勾选的模型"
                    };
                    ui.label(RichText::new(hint).size(11.0).color(MUTED));
                });
            },
        );
    }

    /// 卡片 2：统计条 + 搜索 + 模型表格（含空状态与无匹配状态）。
    fn draw_models(&mut self, ui: &mut Ui) {
        // 先算好只读快照：闭包外借用结束，卡片里才能安全改 self。
        let visible = self.visible_indices();
        let header_visible = visible.clone();
        let total = self.models.len();

        let (bulk, _) = card(
            ui,
            "模型列表",
            "勾选要验证的模型，未勾选 = 全部；验证结果只在本轮显示，不落盘",
            move |ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                if soft_button(ui, "全选可见") {
                    Bulk::SelectVisible
                } else if ui
                    .add(
                        egui::Button::new(RichText::new("取消可见").size(11.0).color(NEUTRAL_FG))
                            .fill(NEUTRAL_SOFT)
                            .stroke(Stroke::new(1.0_f32,BORDER))
                            .rounding(6.0),
                    )
                    .clicked()
                {
                    Bulk::ClearVisible
                } else {
                    Bulk::None
                }
            },
            |ui| {
                let selected = self.models.iter().filter(|m| m.selected).count();
                let passed = self
                    .models
                    .iter()
                    .filter(|m| m.status == TestStatus::Pass)
                    .count();
                let failed = self
                    .models
                    .iter()
                    .filter(|m| m.status == TestStatus::Fail)
                    .count();

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                    stat_tile(ui, "模型", total, FG);
                    stat_tile(ui, "已选", selected, ACCENT);
                    stat_tile(ui, "可用", passed, OK);
                    stat_tile(ui, "失败", failed, BAD);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filter)
                                .hint_text("搜索模型 ID")
                                .font(FontId::proportional(12.0))
                                .desired_width(200.0),
                        );
                    });
                });
                ui.add_space(10.0);

                if total == 0 {
                    // 空状态必须画出来：上一版这里整块空白，看着像程序挂了。
                    info_block(
                        ui,
                        "还没有模型列表",
                        "① 填请求地址（可点「常用」里的预设）\n② 填 Token\n③ 点「获取模型」",
                    );
                } else if visible.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "没有匹配「{}」的模型，共 {total} 个",
                            self.filter.trim()
                        ))
                        .size(12.0)
                        .color(MUTED),
                    );
                } else {
                    // 勾选先记 (行号, 新值)：Grid 闭包里持有 &self.models，
                    // 不能同时改元素，等表格画完再写回。
                    let mut toggle: Option<(usize, bool)> = None;
                    Grid::new("model_table")
                        .num_columns(5)
                        .spacing(vec2(12.0, 6.0))
                        .min_col_width(28.0)
                        .striped(true)
                        .show(ui, |ui| {
                            for text in ["选", "模型 ID", "状态", "耗时", "详情"] {
                                ui.label(
                                    RichText::new(text).size(11.0).strong().color(MUTED),
                                );
                            }
                            ui.end_row();

                            for &i in visible.iter() {
                                let m = &self.models[i];
                                let mut sel = m.selected;
                                if ui.checkbox(&mut sel, "").changed() {
                                    toggle = Some((i, sel));
                                }
                                ui.label(RichText::new(&m.id).size(12.0).monospace().color(FG));
                                status_pill(ui, m.status);
                                ui.label(
                                    RichText::new(match m.latency_ms {
                                        Some(v) => format!("{v} ms"),
                                        None => "—".to_owned(),
                                    })
                                    .size(12.0)
                                    .monospace()
                                    .color(NEUTRAL_FG),
                                );
                                let detail: String = if m.detail.is_empty() {
                                    "—".to_owned()
                                } else {
                                    m.detail.chars().take(70).collect()
                                };
                                ui.label(RichText::new(detail).size(11.0).color(MUTED));
                                ui.end_row();
                            }
                        });
                    if let Some((i, v)) = toggle {
                        self.models[i].selected = v;
                    }
                }
            },
        );

        match bulk {
            Bulk::SelectVisible => {
                for &i in header_visible.iter() {
                    self.models[i].selected = true;
                }
            }
            Bulk::ClearVisible => {
                for &i in header_visible.iter() {
                    self.models[i].selected = false;
                }
            }
            Bulk::None => {}
        }
    }

    /// 卡片 3：本机保存的连接（请求地址 + Token）。默认折叠，长列表不挤掉主区。
    fn draw_endpoints(&mut self, ui: &mut Ui) {
        let total = self.endpoints.entries.len();
        let _ = card(
            ui,
            "已保存的连接",
            "只存请求地址与 Token（本机 0600），不存模型",
            move |ui| {
                ui.label(RichText::new(format!("{total} 条")).size(11.0).color(MUTED));
            },
            |ui| {
                if total == 0 {
                    ui.label(
                        RichText::new("获取或验证成功后，当前填写的请求地址与 Token 会写到这里，下次打开点「使用」即可。")
                            .size(12.0)
                            .color(MUTED),
                    );
                    return;
                }
                // 搜索词只匹配地址；Token 不参与匹配，也不在结果里露出明文。
                let matched = self.endpoints.matching(&self.filter);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                    if soft_button(ui, if self.endpoints_open { "收起列表" } else { "展开列表" }) {
                        self.endpoints_open = !self.endpoints_open;
                    }
                    ui.label(
                        RichText::new(format!("匹配地址 {} / 共 {}", matched.len(), total))
                            .size(11.0)
                            .color(MUTED),
                    );
                });
                if !self.endpoints_open {
                    return;
                }
                if matched.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("没有匹配当前搜索词的已保存地址")
                            .size(12.0)
                            .color(MUTED),
                    );
                    return;
                }
                ui.add_space(8.0);
                let mut act: Option<(usize, EndpointAction)> = None;
                Grid::new("endpoint_table")
                    .num_columns(4)
                    .spacing(vec2(12.0, 6.0))
                    .min_col_width(28.0)
                    .striped(true)
                    .show(ui, |ui| {
                        for text in ["请求地址", "Token", "保存时间", "操作"] {
                            ui.label(RichText::new(text).size(11.0).strong().color(MUTED));
                        }
                        ui.end_row();

                        for &i in matched.iter() {
                            let e = &self.endpoints.entries[i];
                            ui.label(
                                RichText::new(&e.base_url).size(11.0).monospace().color(FG),
                            );
                            ui.label(
                                RichText::new(mask_token(&e.token))
                                    .size(11.0)
                                    .monospace()
                                    .color(NEUTRAL_FG),
                            );
                            // 落盘是 UTC，界面显示本机时间：直接抛 ISO 串会让用户
                            // 以为记录比实际早 8 小时。
                            ui.label(
                                RichText::new(display_time(&e.saved_at))
                                    .size(11.0)
                                    .monospace()
                                    .color(MUTED),
                            );
                            // 嵌套闭包只返回布尔，不在内层写 act：外层 Grid 闭包
                            // 已持有 act 的可变借用，再借一次会撞 E0499。
                            let (use_it, drop_it) = ui
                                .horizontal(|ui| {
                                    ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                                    let used = soft_button(ui, "使用");
                                    let removed = ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("删除").size(11.0).color(BAD),
                                            )
                                            .fill(CARD)
                                            .stroke(Stroke::new(1.0_f32, BORDER))
                                            .rounding(6.0),
                                        )
                                        .clicked();
                                    (used, removed)
                                })
                                .inner;
                            if use_it {
                                act = Some((i, EndpointAction::Use));
                            } else if drop_it {
                                act = Some((i, EndpointAction::Remove));
                            }
                            ui.end_row();
                        }
                    });
                match act {
                    Some((i, EndpointAction::Use)) => {
                        // 先 cloned() 断开对 store 的借用，下面才能写 self 的其他字段。
                        let picked = self.endpoints.entries.get(i).cloned();
                        if let Some(e) = picked {
                            self.service.base_url = e.base_url.clone();
                            self.service.api_key = e.token.clone();
                            self.last_log
                                .push_str(&format!("已填入连接 {}\n", e.base_url));
                        }
                    }
                    Some((i, EndpointAction::Remove)) => {
                        self.endpoints.remove_at(i);
                        if let Err(e) = self.endpoints.save() {
                            self.last_log.push_str(&format!("删除连接失败: {e}\n"));
                        }
                    }
                    None => {}
                }
            },
        );
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 760.0])
            .with_min_inner_size([760.0, 560.0])
            .with_title("中转站模型验证工具"),
        ..Default::default()
    };
    eframe::run_native(
        "中转站模型验证工具",
        options,
        Box::new(|cc| Ok(Box::new(ModelTestApp::new(cc)))),
    )
}
