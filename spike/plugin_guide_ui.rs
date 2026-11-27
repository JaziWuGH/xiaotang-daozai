//! 集市顶部的「做插件」入口和新手教程抽屉。
//!
//! 教程分「还没做好」「已经做好了」两种情况，同一位置、同一结构：已开放能力、
//! 一句话交给 AI 的卡片、可展开的完整步骤，底部固定开发者工具和投稿指南。

use super::*;
use std::path::{Path, PathBuf};

/// 岛仔插件投稿仓，AI 助手从这里读规范和投稿要求。
const REPO: &str = "https://github.com/Renly1994/xiaotang-daozai-plugins";

fn guide_id() -> egui::Id {
    egui::Id::new("plugin-guide")
}

/// 「从模板创建」生成的插件文件夹，教程里的那段话会自动填上。
pub(super) fn template_dir_id() -> egui::Id {
    guide_id().with("template")
}

#[derive(Clone, Default)]
struct GuideState {
    have: bool,
    idea: String,
    existing: String,
    copied: Option<f64>,
}

const IDEA_EXAMPLE: &str = "在待办里显示离下次还信用卡还有几天";
const EXISTING_EXAMPLE: &str = "我写的番茄钟统计脚本，代码在 D:\\我的工具\\pomodoro";

pub(super) fn open_guide(ui: &egui::Ui) {
    let now = ui.input(|input| input.time);
    ui.data_mut(|data| data.insert_temp(guide_id().with("opened"), now));
    ui.ctx().request_repaint();
}

fn close_guide(ui: &egui::Ui) {
    ui.data_mut(|data| data.remove::<f64>(guide_id().with("opened")));
}

/// 发给 AI 助手的那段话。`idea` 为空时用示例占位。
pub(super) fn prompt(have: bool, idea: &str, template: Option<&Path>, exe: Option<&Path>) -> (String, std::ops::Range<usize>) {
    let idea = if idea.trim().is_empty() {
        if have { EXISTING_EXAMPLE } else { IDEA_EXAMPLE }
    } else {
        idea.trim()
    };
    let mut text = if have {
        "帮我把已有的功能改造成岛仔（Windows 灵动岛工具）插件：".to_owned()
    } else {
        "帮我做一个岛仔（Windows 灵动岛工具）插件：".to_owned()
    };
    let start = text.len();
    text.push_str(idea);
    let range = start..text.len();
    text.push('。');
    if have {
        text.push_str("先判断它需要的能力岛仔现在支不支持，不支持的部分先告诉我。");
    }
    text.push('\n');
    match template {
        Some(dir) => text.push_str(&format!(
            "插件模板在 {}（岛仔「从模板创建」生成）。",
            dir.display()
        )),
        None => text.push_str(
            "插件模板我会在岛仔「设置 › 插件 › 开发者工具 › 从模板创建」生成，再告诉你文件夹位置。",
        ),
    }
    text.push_str(&format!("规范和投稿要求看 {REPO} 。\n要求：\n"));
    text.push_str("- 只改模板里的 main.js、plugin.json 和 cover.png，用 JavaScript\n");
    text.push_str("- refresh 每次从头运行，需要记住的内容放进返回的 storage\n");
    text.push_str("- 让用户填的名字、日期写成 plugin.json 里的设置项\n");
    let exe = exe.map_or_else(|| "daozai.exe".to_owned(), |exe| exe.display().to_string());
    text.push_str(&format!(
        "- 改完运行 \"{exe}\" --plugin-test plugin.json，直到校验通过\n"
    ));
    text.push_str("- 可追加 --input mock.json，模拟设置、私有数据、当前时间和网络响应\n");
    text.push_str("- 最后告诉我怎么在岛仔里安装试用");
    (text, range)
}

/// 集市顶部的教程入口：虚线框横幅，整块可点。
pub(super) fn guide_banner(ui: &mut egui::Ui) -> bool {
    let ctx = ui.ctx().clone();
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 60.0), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "看教程"));
    let accent = plugin_accent(ui);
    let painter = ui.painter_at(rect.expand(1.0));
    let left = color(&ctx, Color32::from_rgb(42, 36, 56), Color32::from_rgb(243, 239, 253));
    let right = color(&ctx, Color32::from_rgb(36, 36, 42), Color32::from_rgb(250, 250, 252));
    let mut mesh = egui::Mesh::default();
    let mid = egui::pos2(rect.left() + rect.width() * 0.7, rect.top());
    for (a, b, ca, cb) in [
        (rect.left_top(), egui::pos2(mid.x, rect.bottom()), left, right),
        (mid, rect.right_bottom(), right, right),
    ] {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(a, ca);
        mesh.colored_vertex(egui::pos2(b.x, a.y), cb);
        mesh.colored_vertex(egui::pos2(a.x, b.y), ca);
        mesh.colored_vertex(b, cb);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 2, base + 3);
    }
    painter.add(egui::Shape::mesh(mesh));
    let border = if response.hovered() || response.has_focus() {
        accent
    } else {
        color(&ctx, Color32::from_rgb(74, 65, 96), Color32::from_rgb(205, 196, 238))
    };
    let r = rect.shrink(0.5);
    let outline = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(egui::Shape::dashed_line(&outline, egui::Stroke::new(1.0, border), 5.0, 3.0));

    let icon = egui::Rect::from_min_size(egui::pos2(rect.left() + 14.0, rect.center().y - 18.0), egui::vec2(36.0, 36.0));
    let tag_fg = color(&ctx, Color32::from_rgb(215, 201, 252), Color32::from_rgb(91, 80, 140));
    painter.rect_filled(icon, 9.0, color(&ctx, Color32::from_rgb(58, 49, 81), Color32::from_rgb(232, 225, 250)));
    painter.text(icon.center(), egui::Align2::CENTER_CENTER, "</>", egui::FontId::monospace(13.0), tag_fg);

    let button_text = ui.fonts(|fonts| {
        fonts.layout_no_wrap("看教程".to_owned(), egui::FontId::proportional(12.5), Color32::WHITE)
    });
    let button = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 14.0 - button_text.size().x - 24.0, rect.center().y - 14.0),
        egui::vec2(button_text.size().x + 24.0, 28.0),
    );
    painter.rect_filled(button, 7.0, accent);
    painter.galley(
        button.center() - button_text.size() / 2.0,
        button_text,
        Color32::WHITE,
    );

    let text_left = icon.right() + 14.0;
    let text_w = (button.left() - 14.0 - text_left).max(40.0);
    let strong = ui.visuals().strong_text_color();
    let muted = muted_color(ui);
    let line = |text: &str, size: f32, tint: Color32| {
        let mut job = egui::text::LayoutJob::simple_singleline(
            text.to_owned(),
            egui::FontId::proportional(size),
            tint,
        );
        job.wrap.max_width = text_w;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        ui.fonts(|fonts| fonts.layout_job(job))
    };
    let title = line("把你做的功能变成插件，放到集市给大家用", 13.5, strong);
    let sub = line("不会写代码也能跟着做，已经做好的功能也能搬进来", 12.0, muted);
    let total = title.size().y + 3.0 + sub.size().y;
    let top = rect.center().y - total / 2.0;
    painter.galley(egui::pos2(text_left, top), title.clone(), strong);
    painter.galley(egui::pos2(text_left + 0.3, top), title.clone(), strong);
    painter.galley(egui::pos2(text_left, top + title.size().y + 3.0), sub, muted);
    response.clicked()
}

/// 右侧整窗抽屉外壳：盖住整个设置窗口并置灰背后，从右侧滑入。Esc 和点背景都会关；
/// 上面另有弹窗时交给弹窗处理，免得一次 Esc 关掉两层。`content` 返回真也表示关闭。
pub(super) fn side_drawer(
    ui: &mut egui::Ui,
    salt: &str,
    opened: f64,
    content: impl FnOnce(&mut egui::Ui) -> bool,
) -> bool {
    let ctx = ui.ctx().clone();
    let clip = ctx.screen_rect();
    let elapsed = (ui.input(|input| input.time) - opened) as f32;
    let t = if crate::bar_layout::animations_enabled() {
        let t = (elapsed / 0.2).clamp(0.0, 1.0);
        if t < 1.0 {
            ctx.request_repaint();
        }
        1.0 - (1.0 - t).powi(3)
    } else {
        1.0
    };
    let width = (clip.width() * 0.92).min(440.0);
    let drawer_id = egui::Id::new((salt, "drawer"));
    let layer = egui::LayerId::new(egui::Order::Foreground, drawer_id);
    let on_top = ctx.memory(|memory| memory.is_above_modal_layer(layer));
    let mut close = on_top && ctx.input(|input| input.key_pressed(egui::Key::Escape));
    let scrim = egui::Area::new(egui::Id::new((salt, "scrim")))
        .order(egui::Order::Middle)
        .fixed_pos(clip.min)
        .show(&ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(clip.size(), egui::Sense::click());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_black_alpha((120.0 * t) as u8));
            response.clicked()
        });
    close |= on_top && scrim.inner;
    let left = clip.right() - width * t;
    egui::Area::new(drawer_id)
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(left, clip.top()))
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(color(&ctx, Color32::from_rgb(30, 30, 34), Color32::WHITE))
                .stroke(egui::Stroke::new(1.0, line_color(ui)))
                .inner_margin(egui::Margin::same(18))
                .show(ui, |ui| {
                    ui.set_width(width - 36.0);
                    ui.set_height(clip.height() - 36.0);
                    settings_apply_style(ui);
                    close |= content(ui);
                });
        });
    close
}

/// 抽屉右上角的 ×。
pub(super) fn drawer_close(ui: &mut egui::Ui) -> bool {
    ui.add(egui::Button::new(egui::RichText::new("×").size(18.0)).frame(false))
        .on_hover_text("关闭")
        .clicked()
}

fn sdk_capabilities(ui: &mut egui::Ui) {
    let groups: &[(&str, &[(&str, &str)])] = &[
        ("岛上显示", &[
            ("卡片与标注", "待办信息卡、日历日期标注，可在各自的功能页显示。"),
            ("功能页", "可做独立状态页，支持指标、进度、列表、折线、标签、告警和双色标题。"),
            ("自绘画布", "用文字、图形和图片自由排版，可铺满状态页，也可与标准组件混排。"),
            ("岛上常驻", "开启常驻的插件在岛收起时一起显示，可在「页面管理 › 岛上显示」排序。同一插件可提供多张轮播画布。"),
        ]),
        ("交互与数据", &[
            ("设置与刷新", "支持文本、日期、开关、选项、数字、时刻、星期、时区与列表，可保存私有数据、读取包内 JSON 并定时刷新。"),
            ("页面交互", "支持按钮、单行或多行输入、可点列表和画布交互，可打开 HTTPS 链接或复制文字。"),
            ("实时组件", "岛仔计算时钟、倒计时和跳动数字，可将敏感值默认打码。"),
            ("专注事件", "可订阅专注计时结束事件，由岛仔调用插件脚本。"),
            #[cfg(windows)]
            ("一次性提醒", "可发出提醒，由岛仔按用户的通知和正文设置显示。"),
            #[cfg(windows)]
            ("授权取数", "只读用户选定的文件、本机 HTTP 或白名单内的 HTTPS。HTTPS 最多 3 个 GET 来源，可调刷新间隔，密钥由 Windows 托管。"),
            ("AI 额度", "为现有 AI 额度面板添加自定义数据来源。"),
        ]),
        ("资源与接入", &[
            ("外观资源", "可提供皮肤、功能图标、吉祥物表情和画布字体。"),
            ("快捷导航", "为已启用的功能页提供底部快捷入口，可配置选中颜色。"),
            ("无线接收器", "可为兼容的 USB 无线接收器提供识别配置和电量读取规则。"),
            #[cfg(all(windows, feature = "sdk-material"))]
            ("GPU 材质", "可安装自定义材质，需开启 DX12 兼容渲染。"),
            #[cfg(windows)]
            ("本机接入", "独立接入端可发通知、更新状态卡，并接收用户点击发出的操作。"),
        ]),
        ("开发调试", &[
            ("本地工具", "可从模板创建、安装、监听文件变化和查看脚本日志，并用模拟数据校验及本地打包。"),
        ]),
    ];
    egui::Frame::new()
        .fill(card_color(ui))
        .stroke(egui::Stroke::new(1.0, line_color(ui)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let spacing = ui.spacing_mut();
            spacing.item_spacing.y = 4.0;
            spacing.button_padding.y = 2.0;
            spacing.interact_size.y = 24.0;
            egui::CollapsingHeader::new(egui::RichText::new("当前版本已开放的 SDK 能力").size(13.5).strong())
                .id_salt("plugin-guide-sdk-capabilities")
                .show_background(false)
                .show(ui, |ui| {
                    let muted = muted_color(ui);
                    let accent = plugin_accent(ui);
                    for (index, (group, capabilities)) in groups.iter().enumerate() {
                        if index > 0 {
                            ui.separator();
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(*group).size(11.5).strong().color(accent));
                        for (label, description) in *capabilities {
                            ui.scope(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.add(egui::Label::new(egui::RichText::new(*label).size(12.5).strong()).wrap());
                                ui.add(egui::Label::new(egui::RichText::new(*description).size(12.0).color(muted)).wrap());
                            });
                        }
                    }
                    ui.add_space(4.0);
                });
        });
}

/// 步骤说明里用 `反引号` 包起来的路径和文件名，显示成小底色块。
fn step_text(ui: &egui::Ui, text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let muted = muted_color(ui);
    let code_bg = color(ui.ctx(), Color32::from_rgb(52, 52, 59), Color32::from_rgb(236, 237, 242));
    let strong = ui.visuals().strong_text_color();
    for (index, part) in text.split('`').enumerate() {
        if part.is_empty() {
            continue;
        }
        let format = if index % 2 == 1 {
            egui::TextFormat {
                font_id: egui::FontId::proportional(11.5),
                color: strong,
                background: code_bg,
                ..Default::default()
            }
        } else {
            egui::TextFormat {
                font_id: egui::FontId::proportional(12.0),
                color: muted,
                line_height: Some(19.0),
                ..Default::default()
            }
        };
        let part = if index % 2 == 1 { format!(" {part} ") } else { part.to_owned() };
        job.append(&part, 0.0, format);
    }
    job
}

fn steps(ui: &mut egui::Ui, items: &[(&str, &str)]) {
    let accent_bg = color(ui.ctx(), Color32::from_rgb(58, 49, 81), Color32::from_rgb(232, 225, 250));
    let accent_fg = color(ui.ctx(), Color32::from_rgb(215, 201, 252), Color32::from_rgb(91, 80, 140));
    for (index, (title, text)) in items.iter().enumerate() {
        egui::Frame::new()
            .fill(card_color(ui))
            .stroke(egui::Stroke::new(1.0, line_color(ui)))
            .corner_radius(9.0)
            .inner_margin(egui::Margin::symmetric(12, 11))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 11.0, accent_bg);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        (index + 1).to_string(),
                        egui::FontId::proportional(12.0),
                        accent_fg,
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.label(egui::RichText::new(*title).size(13.0).strong());
                        let job = step_text(ui, text);
                        ui.add(egui::Label::new(job).wrap());
                    });
                });
            });
        ui.add_space(2.0);
    }
}

const NEW_STEPS: [(&str, &str); 5] = [
    ("想好要显示什么", "用一句话写下来，比如「收起时显示距离发薪还有几天」。先选好内容放在哪个功能页，或者是否要在岛上常驻。"),
    ("用模板生成插件", "到 `开发者工具 › 从模板创建`，填插件 ID、名称和作者，选一个文件夹。岛仔会生成可直接运行的样例。"),
    ("改成你的内容", "用记事本或 VS Code 打开文件夹。`main.js` 写显示逻辑，`plugin.json` 声明要用的能力与设置。把封面换成你的 `cover.png`（16:9，不超过 256 KB）。"),
    ("装到自己的岛仔上试", "在开发者工具点「安装本地插件…」，选 plugin.json，打开插件开关。勾选「监视文件变化」后保存本地文件即可重载，运行信息可在「脚本日志」里查看。"),
    ("校验并投稿到集市", "点「本地校验」通过后，按投稿指南提交到岛仔插件仓库。审核通过后，大家就能在集市里搜到并安装。"),
];

const HAVE_STEPS: [(&str, &str); 5] = [
    ("用模板生成插件外壳", "到 `开发者工具 › 从模板创建` 生成样例，里面包含清单、入口和封面。"),
    ("把逻辑搬进 refresh", "打开 `main.js`，把你原来计算结果的部分放进 `refresh`：它拿到设置、今天日期和上次保存的数据，返回要显示的文字和要保存的数据。其他语言写的，可以让 AI 助手改写成 JavaScript。"),
    ("把要用户填的东西变成设置", "原来写死在代码里的名字、日期，改成 `plugin.json` 里的设置项。用户在设置页填，插件刷新时读到。"),
    ("配一张封面", "放一张 16:9 的 `cover.png`，不超过 256 KB、1920×1080。集市卡片和详情会原样显示它。"),
    ("试装、校验、投稿", "「安装本地插件…」装上试用，「本地校验」通过后按投稿指南提交。之后发新版本只要改版本号再提交一次，用户会看到「更新」。"),
];

/// 打开开发者工具；`template` 为真时直接进入「从模板创建」表单。
fn open_developer_tools(ui: &egui::Ui, template: bool) {
    let key = egui::Id::new("plugin-developer-tools");
    ui.data_mut(|data| {
        data.insert_temp(key, true);
        if template {
            data.insert_temp(key.with("author"), plugin_author_ui::AuthorTools::template());
        }
    });
    ui.ctx().request_repaint();
}

impl LocalPlugins {
    /// 教程抽屉。打开时盖住整个设置窗口。
    pub(super) fn guide_drawer(&mut self, ui: &mut egui::Ui) {
        let Some(opened) = ui.data(|data| data.get_temp::<f64>(guide_id().with("opened"))) else {
            return;
        };
        let state_id = guide_id().with("state");
        let mut state = ui.data(|data| data.get_temp::<GuideState>(state_id)).unwrap_or_default();
        let template: Option<PathBuf> = ui.data(|data| data.get_temp(template_dir_id()));
        let exe = std::env::current_exe().ok();
        let now = ui.input(|input| input.time);
        let close = side_drawer(ui, "plugin-guide", opened, |ui| {
            let mut close = false;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("做一个岛仔插件").size(17.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close |= drawer_close(ui);
                    let options = ["还没做好".to_owned(), "已经做好了".to_owned()];
                    if let Some(index) = segmented(ui, "plugin-guide-mode", &options, usize::from(state.have), &[false, false], 12.0) {
                        state.have = index == 1;
                        state.copied = None;
                    }
                });
            });
            ui.add_space(8.0);
            let footer_h = 50.0;
            egui::ScrollArea::vertical()
                .id_salt("plugin-guide-scroll")
                .max_height((ui.available_height() - footer_h).max(80.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    sdk_capabilities(ui);
                    ui.add_space(8.0);
                    self.guide_ai_card(ui, &mut state, template.as_deref(), exe.as_deref(), now);
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new(egui::RichText::new("想自己动手，看完整步骤").size(13.0))
                        .id_salt(("plugin-guide-manual", state.have))
                        .default_open(false)
                        .show(ui, |ui| {
                            steps(ui, if state.have { &HAVE_STEPS } else { &NEW_STEPS });
                        });
                });
            let top = ui.cursor().top() + 4.0;
            ui.painter().hline(
                ui.max_rect().x_range().expand(18.0),
                top,
                egui::Stroke::new(1.0, line_color(ui)),
            );
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(market_button(ui, "投稿指南")).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(plugin_author_ui::GUIDE));
                }
                if ui.button("打开开发者工具").clicked() {
                    open_developer_tools(ui, false);
                }
            });
            close
        });
        if close {
            close_guide(ui);
        }
        ui.data_mut(|data| data.insert_temp(state_id, state));
    }

    fn guide_ai_card(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut GuideState,
        template: Option<&Path>,
        exe: Option<&Path>,
        now: f64,
    ) {
        let ctx = ui.ctx().clone();
        let accent = plugin_accent(ui);
        egui::Frame::new()
            .fill(paint::mix(card_color(ui), accent, 0.12))
            .stroke(egui::Stroke::new(1.0, color(&ctx, Color32::from_rgb(74, 65, 96), Color32::from_rgb(205, 196, 238))))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let title = if state.have { "一句话交给 AI 改造" } else { "一句话交给 AI 做" };
                    ui.label(egui::RichText::new(title).size(13.5).strong());
                    small_tag(ui, "JS 卡片模板");
                });
                let hint = if state.have {
                    "把原代码的文件夹位置也告诉 AI，请它对照当前 SDK 判断哪些能力适合迁移到插件。"
                } else {
                    "先点下方「从模板创建」生成插件文件夹，再把这段话发给 Claude Code、Cursor 等 AI 编程助手。模板位置和校验命令岛仔已经帮你填好。"
                };
                ui.add(egui::Label::new(egui::RichText::new(hint).size(12.0).color(muted_color(ui))).wrap());
                let (value, example, salt) = if state.have {
                    (&mut state.existing, EXISTING_EXAMPLE, "plugin-guide-existing")
                } else {
                    (&mut state.idea, IDEA_EXAMPLE, "plugin-guide-idea")
                };
                ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(value)
                        .id_salt(salt)
                        .hint_text(example)
                        .char_limit(120),
                );
                let (text, range) = prompt(state.have, value, template, exe);
                egui::Frame::new()
                    .fill(color(&ctx, Color32::from_rgb(26, 26, 30), Color32::from_rgb(247, 247, 250)))
                    .stroke(egui::Stroke::new(1.0, line_color(ui)))
                    .corner_radius(7.0)
                    .inner_margin(egui::Margin::symmetric(10, 9))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt(("plugin-guide-prompt", state.have))
                            .max_height(170.0)
                            .show(ui, |ui| {
                                let base = egui::TextFormat {
                                    font_id: egui::FontId::proportional(11.5),
                                    color: color(&ctx, Color32::from_rgb(201, 201, 211), Color32::from_rgb(60, 64, 74)),
                                    line_height: Some(19.0),
                                    ..Default::default()
                                };
                                let mut job = egui::text::LayoutJob::default();
                                job.append(&text[..range.start], 0.0, base.clone());
                                job.append(
                                    &text[range.clone()],
                                    0.0,
                                    egui::TextFormat {
                                        color: color(&ctx, Color32::from_rgb(215, 201, 252), Color32::from_rgb(91, 80, 140)),
                                        ..base.clone()
                                    },
                                );
                                job.append(&text[range.end..], 0.0, base);
                                ui.add(egui::Label::new(job).wrap().selectable(true));
                            });
                    });
                ui.horizontal(|ui| {
                    let width = ui.available_width();
                    ui.allocate_ui_with_layout(
                        egui::vec2((width - 196.0).max(80.0), 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new("AI 改完后，回来点「安装本地插件…」试用")
                                        .size(11.5)
                                        .color(muted_color(ui)),
                                )
                                .wrap(),
                            );
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let copied = state.copied.is_some_and(|at| now - at < 1.5);
                        if copied {
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                        }
                        let label = if copied { "已复制" } else { "复制这段话" };
                        if ui.add(market_button(ui, label)).clicked() {
                            ui.ctx().copy_text(text.clone());
                            state.copied = Some(now);
                        }
                        if ui.button("从模板创建").clicked() {
                            open_developer_tools(ui, true);
                        }
                    });
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_fills_the_idea_template_and_validator() {
        let (text, range) = prompt(
            false,
            "  显示离周末还有几天 ",
            Some(Path::new(r"D:\插件\weekend")),
            Some(Path::new(r"C:\Daozai\daozai.exe")),
        );
        assert_eq!(&text[range], "显示离周末还有几天");
        assert!(text.contains(r"插件模板在 D:\插件\weekend"));
        assert!(text.contains(r#""C:\Daozai\daozai.exe" --plugin-test plugin.json"#));
        let (text, range) = prompt(true, "", None, None);
        assert_eq!(&text[range], EXISTING_EXAMPLE);
        assert!(text.contains("不支持的部分先告诉我"));
        assert!(text.contains("从模板创建」生成，再告诉你文件夹位置"));
    }
}
