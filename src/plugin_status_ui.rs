//! 用岛仔画布绘制状态详情。
#[path = "plugin_status_controls.rs"]
mod controls;
pub(crate) use controls::{extra_fields_height, ClickTarget, DEFAULT_FORM_HEIGHT, Interaction, InteractionResponse};

#[path = "plugin_status_components.rs"]
mod components;
#[path = "plugin_status_private.rs"]
mod private;

pub(crate) use private::{clear_sensitive_reveal, sensitive_values_blocked, MASKED_VALUE};
use private::SensitiveValues;

#[cfg(test)]
#[path = "plugin_status_realtime_ui_tests.rs"]
mod realtime_ui_tests;

use components::{
    badge_count, draw_badges, draw_chart, draw_icon, draw_list, draw_note, list_rows, note_lines,
    BADGE_ROW_HEIGHT, NOTE_LINE_HEIGHT,
};

use crate::{
    paint::{self, Card, H},
    plugin_status::{
        PluginStatus, ProgressVariant, StatusComponent, StatusDensity, StatusEmphasis,
        StatusHealth, StatusLayout, StatusState, StatusStyle, StatusSurface, MAX_ITEMS,
    },
    skin_paint::{self, Palette},
    theme::{self, size, W},
};
use eframe::egui::{self, Color32};
use crate::{plugin_canvas::Canvas, plugin_canvas_ui::CanvasTheme};
use std::{
    f32::consts::{FRAC_PI_2, TAU},
    sync::Arc,
    time::Duration,
};

pub(crate) const STATUS_PAGE_H: f32 = 236.0;
#[cfg(windows)]
pub(crate) const ATTRIBUTION_HEIGHT: f32 = 22.0;

const LEFT: f32 = 42.0;
const WIDTH: f32 = 335.0;
const TEXT_RIGHT: f32 = 363.0;
const CONTENT_TOP: f32 = 42.0;
const PAGE_BOTTOM_SPACE: f32 = 16.0;
const MIN_LAYOUT_HEIGHT: f32 = 156.0;
const MAX_METRICS: usize = 6;
const MAX_PAGE_HEIGHT: f32 = 348.0;
const MAX_CARD_HEIGHT: f32 = MAX_PAGE_HEIGHT - CONTENT_TOP - PAGE_BOTTOM_SPACE;
const CANVAS_LEFT: f32 = 16.0;
const CANVAS_WIDTH: f32 = crate::app::PW - CANVAS_LEFT * 2.0;
const MAX_CANVAS_CONTENT_HEIGHT: f32 = 360.0;
pub(crate) type CanvasPaint<'a> = dyn FnMut(&Card<'_>, &Arc<Canvas>, egui::Rect, CanvasTheme) + 'a;

#[derive(Clone, Copy)]
pub(crate) struct StatusBackdrop {
    pub palette: Option<Palette>,
    pub island: Color32,
}

impl StatusBackdrop {
    fn card_color(self, surface: StatusSurface) -> Color32 {
        let [r, g, b, _] = self.island.to_srgba_unmultiplied();
        let island = Color32::from_rgb(r, g, b);
        if surface == StatusSurface::Plain {
            return island;
        }
        let card = self.palette.map_or(theme::CARD, |palette| {
            skin_paint::map_color(theme::CARD, &palette)
        });
        let [r, g, b, alpha] = card.to_srgba_unmultiplied();
        paint::mix(island, Color32::from_rgb(r, g, b), f32::from(alpha) / 255.0)
    }
}

#[derive(Clone, Copy)]
struct Accent {
    color: Color32,
    fixed: bool,
}

impl Accent {
    fn native(color: Color32) -> Self {
        Self {
            color,
            fixed: false,
        }
    }

    fn draw(self, c: &Card, draw: impl FnOnce(Color32)) {
        let layer = c.painter.layer_id();
        let from = self
            .fixed
            .then(|| skin_paint::shape_count(c.ui.ctx(), layer));
        draw(self.color);
        if let Some(from) = from {
            // 强调色已经根据最终卡片底色校正，后续皮肤换色保留此色。
            skin_paint::tone_since(c.ui.ctx(), layer, from, |color, _| color);
        }
    }
}

fn readable_accent(color: Color32, background: Color32) -> Color32 {
    let contrast = |color: Color32| {
        let rgb = |c: Color32| [c.r(), c.g(), c.b()].map(|v| f32::from(v) / 255.0);
        crate::glass::contrast(rgb(color), rgb(background))
    };
    if contrast(color) >= 3.0 {
        return color;
    }
    let target = if contrast(Color32::BLACK) > contrast(Color32::WHITE) {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..8 {
        let mid = (low + high) * 0.5;
        if contrast(paint::mix(color, target, mid)) >= 3.0 {
            high = mid;
        } else {
            low = mid;
        }
    }
    paint::mix(color, target, high)
}

pub(crate) fn draw_title(
    c: &Card,
    name: &str,
    accent: Option<(&str, Option<[u8; 3]>)>,
    background: Color32,
) {
    let title = c.fit(name, size::TITLE, W::Bold, 142.0);
    if let Some((suffix, rgb)) = accent {
        if let Some(prefix) = name.strip_suffix(suffix) {
            if let Some(tail) = title.strip_prefix(prefix).filter(|tail| !tail.is_empty()) {
                let width = c.text(15.0, 25.0, prefix, size::TITLE, W::Bold, theme::WHITE, H::Left);
                let accent = rgb.map_or_else(
                    || Accent::native(theme::BLUE),
                    |[r, g, b]| Accent {
                        color: readable_accent(Color32::from_rgb(r, g, b), background),
                        fixed: true,
                    },
                );
                accent.draw(c, |color| {
                    c.text(15.0 + width, 25.0, tail, size::TITLE, W::Bold, color, H::Left);
                });
                return;
            }
        }
    }
    c.text(15.0, 25.0, &title, size::TITLE, W::Bold, theme::WHITE, H::Left);
}

pub(crate) fn status_page_height(layout: Option<&StatusLayout>, status: Option<&PluginStatus>) -> f32 {
    let canvas = status.and_then(|status| status.canvas.as_ref());
    if let Some(canvas) = canvas.filter(|_| layout.is_none_or(canvas_only)) {
        return CONTENT_TOP + canvas.height + PAGE_BOTTOM_SPACE;
    }
    let Some(layout) = layout else {
        return STATUS_PAGE_H;
    };
    if canvas.is_some() {
        return CONTENT_TOP + layout_card_height(layout, status).min(MAX_CANVAS_CONTENT_HEIGHT)
            + PAGE_BOTTOM_SPACE;
    }
    // 给可出现的组件留足空间，读数缺失或断源时保留页面高度。
    let extra = status.filter(|_| layout.components.contains(&StatusComponent::Fields {}))
        .map_or(0.0, controls::extra_fields_height);
    (CONTENT_TOP + layout_card_height(layout, None) + extra + PAGE_BOTTOM_SPACE)
        .clamp(MIN_LAYOUT_HEIGHT, MAX_PAGE_HEIGHT)
}

pub(crate) fn status_scroll_area(
    status: &PluginStatus,
    layout: &StatusLayout,
) -> Option<egui::Rect> {
    let (x, width, height) = layout_viewport(status.canvas.is_some());
    (layout_card_height(layout, Some(status)) > height).then(|| {
        egui::Rect::from_min_size(
            egui::pos2(x, CONTENT_TOP),
            egui::vec2(width, height),
        )
    })
}

pub(crate) fn draw_status_page(
    c: &Card,
    id: &str,
    status: Option<&PluginStatus>,
    health: StatusHealth,
    layout: Option<&StatusLayout>,
    backdrop: StatusBackdrop,
    privacy_blocked: bool,
    paint_canvas: Option<&mut CanvasPaint<'_>>,
) {
    let Some(status) = status.filter(|_| health == StatusHealth::Ready) else {
        draw_unavailable(c, health);
        return;
    };
    let privacy = SensitiveValues::new(id, privacy_blocked);
    if layout.is_none_or(canvas_only) && status.canvas.is_some() {
        draw_full_canvas(c, status, layout, backdrop, paint_canvas, None);
    } else if let Some(layout) = layout {
        draw_layout(c, id, status, layout, backdrop, &privacy, None, paint_canvas);
    } else {
        draw_default(c, status, &privacy);
    }
}

/// 带按钮、输入框或可点列表的状态页。点击结果由 `interaction` 带回插件宿主。
pub(crate) fn draw_interactive_status_page(
    c: &Card,
    id: &str,
    status: &PluginStatus,
    layout: Option<&StatusLayout>,
    backdrop: StatusBackdrop,
    privacy_blocked: bool,
    interaction: &mut Interaction<'_>,
    paint_canvas: Option<&mut CanvasPaint<'_>>,
) {
    let privacy = SensitiveValues::new(id, privacy_blocked);
    if layout.is_none_or(canvas_only) && status.canvas.is_some() {
        draw_full_canvas(c, status, layout, backdrop, paint_canvas, Some(interaction));
    } else if let Some(layout) = layout {
        draw_layout(c, id, status, layout, backdrop, &privacy, Some(interaction), paint_canvas);
    } else {
        draw_default(c, status, &privacy);
        let y = STATUS_PAGE_H - PAGE_BOTTOM_SPACE;
        interaction.fields(c, status, LEFT + 14.0, y, TEXT_RIGHT - LEFT - 14.0, None);
        interaction.actions(
            c,
            status,
            LEFT + 14.0,
            y + controls::fields_height(Some(status), None),
            TEXT_RIGHT - LEFT - 14.0,
            None,
        );
    }
}

fn canvas_only(layout: &StatusLayout) -> bool {
    layout.components == [StatusComponent::Canvas {}]
}

fn draw_full_canvas(
    c: &Card,
    status: &PluginStatus,
    layout: Option<&StatusLayout>,
    backdrop: StatusBackdrop,
    paint_canvas: Option<&mut CanvasPaint<'_>>,
    interaction: Option<&mut Interaction<'_>>,
) {
    if let (Some(draw), Some(canvas)) = (paint_canvas, &status.canvas) {
        draw(c, canvas, c.rect(CANVAS_LEFT, CONTENT_TOP, CANVAS_WIDTH, canvas.height),
            canvas_theme(layout, backdrop));
        if let (Some(layout), Some(interaction)) = (layout, interaction) {
            interaction.canvas_fields(c, status, layout, CANVAS_LEFT, CONTENT_TOP);
        }
    }
}

fn canvas_theme(layout: Option<&StatusLayout>, backdrop: StatusBackdrop) -> CanvasTheme {
    let accent = layout.and_then(|layout| {
        let [r, g, b] = layout.style.accent_rgb()?;
        Some(readable_accent(Color32::from_rgb(r, g, b), backdrop.card_color(layout.style.surface)))
    });
    CanvasTheme { palette: backdrop.palette, accent, background: backdrop.island }
}

fn layout_viewport(canvas: bool) -> (f32, f32, f32) {
    if canvas { (CANVAS_LEFT, CANVAS_WIDTH, MAX_CANVAS_CONTENT_HEIGHT) }
    else { (LEFT, WIDTH, MAX_CARD_HEIGHT) }
}

#[cfg(windows)]
pub(crate) fn draw_attribution(
    c: &Card,
    id: &str,
    top: f32,
    label: &str,
    url: Option<&str>,
    time: Option<&chrono::DateTime<chrono::Local>>,
) {
    let mut width = WIDTH;
    if let Some(time) = time {
        let time = format!("最近取数 {}", time.format("%m-%d %H:%M"));
        width -= c.text_w(&time, size::TINY, W::Regular) + 8.0;
        c.centered_text(LEFT + WIDTH, top + 3.0, &time, size::TINY, W::Regular, theme::DIM, H::Right);
    }
    let label = c.fit(&format!("数据来源 {label}"), size::TINY, W::Regular, width);
    let color = if url.is_some() { theme::BLUE } else { theme::DIM };
    c.centered_text(LEFT, top + 3.0, &label, size::TINY, W::Regular, color, H::Left);
    if let Some(url) = url {
        let width = c.text_w(&label, size::TINY, W::Regular);
        if c.hit(LEFT, top - 6.0, width, 18.0, ("plugin-source", id)).clicked() {
            c.ui.ctx().open_url(eframe::egui::OpenUrl::new_tab(url));
        }
    }
}

fn draw_default(c: &Card, status: &PluginStatus, privacy: &SensitiveValues) {
    let color = state_color(status.state);
    c.fill(LEFT, 42.0, WIDTH, 78.0, 9.0, theme::CARD);
    let head_x = if let Some(progress) = status.progress_pct {
        draw_progress_ring(c, progress, Accent::native(color), (84.0, 81.0), 27.0);
        128.0
    } else {
        56.0
    };
    let device_offline = status.state == StatusState::Offline && status.label != "设备离线";
    let title_width = TEXT_RIGHT - head_x - if device_offline { 73.0 } else { 12.0 };
    c.circle(head_x + 3.0, 61.0, 2.8, color);
    let label = c.fit(&status.label, size::TITLE, W::Semibold, title_width);
    c.centered_text(
        head_x + 12.0,
        61.0,
        &label,
        size::TITLE,
        W::Semibold,
        theme::WHITE,
        H::Left,
    );
    if device_offline {
        c.centered_text(
            TEXT_RIGHT,
            61.0,
            "设备离线",
            size::TINY,
            W::Regular,
            theme::GRAY,
            H::Right,
        );
    }
    if let Some(summary) = status.summary.as_deref() {
        draw_lines(c, summary, head_x, 81.0, TEXT_RIGHT - head_x, theme::GRAY);
    }
    if let Some(at) = status
        .observed_at
        .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
    {
        let time = at.with_timezone(&chrono::Local).format("%m-%d %H:%M");
        c.centered_text(
            head_x,
            110.0,
            &format!("更新于 {time}"),
            size::TINY,
            W::Regular,
            theme::DIM,
            H::Left,
        );
    }

    for (i, metric) in status.metrics.iter().enumerate() {
        let x = 51.0 + (i % 2) as f32 * 165.0;
        let y = 133.0 + (i / 2) as f32 * 18.0;
        let revealed = privacy.revealed(
            c, metric.sensitive, ("metric", i, &metric.label, &metric.value),
            [x, y - 9.0, 151.0, 18.0],
        );
        let raw = if revealed { &metric.value } else { MASKED_VALUE };
        let value = c.fit(raw, size::BODY, W::Semibold, 87.0);
        let label_width = 151.0 - c.text_w(&value, size::BODY, W::Semibold) - 8.0;
        let label = c.fit(&metric.label, size::SMALL, W::Regular, label_width);
        c.centered_text(x, y, &label, size::SMALL, W::Regular, theme::GRAY, H::Left);
        c.centered_text(
            x + 151.0,
            y,
            &value,
            size::BODY,
            W::Semibold,
            theme::WHITE,
            H::Right,
        );
    }
    if let Some(alert) = status.alert.as_deref() {
        let y = if status.metrics.is_empty() {
            131.0
        } else {
            128.0 + status.metrics.len().div_ceil(2) as f32 * 18.0
        };
        let text_width = TEXT_RIGHT - 63.0;
        let one_line = c.text_w(alert, size::SMALL, W::Regular) <= text_width;
        let (height, lead) = if one_line { (24.0, 12.0) } else { (30.0, 9.0) };
        c.fill(
            LEFT,
            y,
            WIDTH,
            height,
            7.0,
            paint::alpha(theme::ORANGE, 0.14),
        );
        c.circle(54.0, y + lead, 2.4, theme::ORANGE);
        draw_lines(c, alert, 63.0, y + lead, text_width, theme::ORANGE);
    }
}

fn state_color(state: StatusState) -> Color32 {
    match state {
        StatusState::Running => theme::SOFT_GREEN,
        StatusState::Paused => theme::YELLOW,
        StatusState::Succeeded => theme::CYAN,
        StatusState::Error => theme::RED_TOP,
        StatusState::Offline | StatusState::Idle => theme::DIM,
    }
}

fn spacing(density: StatusDensity) -> (f32, f32) {
    match density {
        StatusDensity::Comfortable => (8.0, 6.0),
        StatusDensity::Compact => (6.0, 3.0),
    }
}

fn metric_row_height(density: StatusDensity) -> f32 {
    match density {
        StatusDensity::Comfortable => 18.0,
        StatusDensity::Compact => 15.0,
    }
}

fn component_height(
    component: StatusComponent,
    layout: &StatusLayout,
    status: Option<&PluginStatus>,
) -> f32 {
    let density = layout.style.density;
    match component {
        StatusComponent::State {} => 18.0,
        StatusComponent::Summary {} => 26.0,
        StatusComponent::Progress { variant } => match (variant, density) {
            (ProgressVariant::Ring, StatusDensity::Comfortable) => 60.0,
            (ProgressVariant::Ring, StatusDensity::Compact) => 52.0,
            (ProgressVariant::Bar, StatusDensity::Comfortable) => 26.0,
            (ProgressVariant::Bar, StatusDensity::Compact) => 22.0,
        },
        StatusComponent::Metrics { columns } => {
            let metrics = status.map_or(MAX_METRICS, |status| status.metrics.len());
            metrics.div_ceil(usize::from(columns)) as f32 * metric_row_height(density)
        }
        StatusComponent::Alert {} => 30.0,
        StatusComponent::UpdatedAt {} => 14.0,
        StatusComponent::Actions {} => controls::ACTION_HEIGHT,
        StatusComponent::Fields {} => controls::fields_height(status, Some(layout)),
        StatusComponent::List {} => {
            let count = status.map_or(MAX_ITEMS, |status| status.items.len());
            list_rows(density, count) as f32 * metric_row_height(density)
        }
        StatusComponent::Chart {} => {
            if density == StatusDensity::Compact {
                30.0
            } else {
                44.0
            }
        }
        StatusComponent::Text {} => {
            note_lines(density, status.and_then(|status| status.note.as_deref())) as f32
                * NOTE_LINE_HEIGHT
        }
        StatusComponent::Badge {} => {
            let count = badge_count(density, status.map_or(4, |status| status.tags.len()));
            let columns = if density == StatusDensity::Compact {
                3
            } else {
                2
            };
            count.div_ceil(columns) as f32 * BADGE_ROW_HEIGHT
        }
        StatusComponent::Counter {} | StatusComponent::Clock {} | StatusComponent::Countdown {} => {
            metric_row_height(density)
        }
        StatusComponent::Canvas {} => status.and_then(|status| status.canvas.as_ref())
            .map_or(40.0, |canvas| canvas.height),
    }
}

fn components_with_data<'a>(
    layout: &'a StatusLayout,
    status: Option<&'a PluginStatus>,
) -> impl Iterator<Item = (StatusComponent, usize)> + 'a {
    layout
        .components
        .iter()
        .copied()
        .enumerate()
        .filter_map(move |(index, component)| {
            let occurrence = layout.occurrence(index);
            let visible = status.is_none_or(|status| match component {
                StatusComponent::State {} => true,
                StatusComponent::Summary {} => status.summary.is_some(),
                StatusComponent::Progress { .. } => status.progress_pct.is_some(),
                StatusComponent::Metrics { .. } => !status.metrics.is_empty(),
                StatusComponent::Alert {} => status.alert.is_some(),
                StatusComponent::UpdatedAt {} => status.observed_at.is_some(),
                StatusComponent::Actions {} => controls::standalone_actions(status, Some(layout)).next().is_some(),
                StatusComponent::Fields {} => controls::standalone_fields(status, Some(layout)).next().is_some(),
                StatusComponent::List {} => !status.items.is_empty(),
                StatusComponent::Chart {} => status.series.is_some(),
                StatusComponent::Text {} => status.note.is_some(),
                StatusComponent::Badge {} => !status.tags.is_empty(),
                StatusComponent::Counter {} => occurrence < status.counter.len(),
                StatusComponent::Clock {} => occurrence < status.clock.len(),
                StatusComponent::Countdown {} => occurrence < status.countdown.len(),
                StatusComponent::Canvas {} => status.canvas.is_some(),
            });
            visible.then_some((component, occurrence))
        })
}

fn layout_card_height(layout: &StatusLayout, status: Option<&PluginStatus>) -> f32 {
    let density = layout.style.density;
    let (padding, gap) = spacing(density);
    let height: f32 = components_with_data(layout, status)
        .map(|(component, _)| component_height(component, layout, status) + gap)
        .sum();
    if height > 0.0 {
        height - gap + padding * 2.0
    } else {
        0.0
    }
}

fn draw_layout(
    c: &Card,
    id: &str,
    status: &PluginStatus,
    layout: &StatusLayout,
    backdrop: StatusBackdrop,
    privacy: &SensitiveValues,
    mut interaction: Option<&mut Interaction<'_>>,
    paint_canvas: Option<&mut CanvasPaint<'_>>,
) {
    let height = layout_card_height(layout, Some(status));
    if height == 0.0 {
        draw_message(c, "当前没有可显示的数据", None, theme::DIM);
        return;
    }
    let (viewport_x, viewport_width, viewport_height) = layout_viewport(status.canvas.is_some());
    if layout.style.surface == StatusSurface::Card {
        c.fill(
            LEFT,
            CONTENT_TOP,
            WIDTH,
            height.min(viewport_height),
            9.0,
            theme::CARD,
        );
    }
    let accent = layout.style.accent_rgb().map_or_else(
        || Accent::native(state_color(status.state)),
        |[r, g, b]| Accent {
            color: readable_accent(
                Color32::from_rgb(r, g, b),
                backdrop.card_color(layout.style.surface),
            ),
            fixed: backdrop.palette.is_some(),
        },
    );
    let canvas_theme = canvas_theme(Some(layout), backdrop);
    if height <= viewport_height {
        draw_components(c, status, layout, accent, privacy, interaction, canvas_theme, paint_canvas);
        return;
    }

    let viewport = c.rect(viewport_x, CONTENT_TOP, viewport_width, viewport_height);
    let mut content = egui::Ui::new(
        c.ui.ctx().clone(),
        egui::Id::new(("plugin-status-scroll", id, c.live)),
        egui::UiBuilder::new()
            .max_rect(viewport)
            .layer_id(c.painter.layer_id())
            .style(c.ui.style().clone()),
    );
    content.set_clip_rect(viewport.intersect(c.painter.clip_rect()));
    content.set_opacity(c.painter.opacity());
    if !c.live {
        content.disable();
    }
    content.spacing_mut().scroll.bar_width = 4.0 * c.s;
    egui::ScrollArea::vertical()
        .max_height(viewport.height())
        .auto_shrink([false, false])
        .show(&mut content, |ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(viewport_width * c.s, height * c.s), egui::Sense::hover());
            let origin = rect.min - egui::vec2(viewport_x, CONTENT_TOP) * c.s;
            let mut inner = Card::new(ui, origin, ui.clip_rect());
            inner.s = c.s;
            inner.live = c.live;
            draw_components(&inner, status, layout, accent, privacy, interaction.as_deref_mut(),
                canvas_theme, paint_canvas);
        });
}

fn draw_components(
    c: &Card,
    status: &PluginStatus,
    layout: &StatusLayout,
    accent: Accent,
    privacy: &SensitiveValues,
    mut interaction: Option<&mut Interaction<'_>>,
    canvas_theme: CanvasTheme,
    mut paint_canvas: Option<&mut CanvasPaint<'_>>,
) {
    let density = layout.style.density;
    let (padding, gap) = spacing(density);
    let mut y = CONTENT_TOP + padding;
    let clock = chrono::Utc::now();
    let now = clock.timestamp();
    for (component, index) in components_with_data(layout, Some(status)) {
        let height = component_height(component, layout, Some(status));
        let center_y = y + height / 2.0;
        let x = LEFT + 14.0;
        match component {
            StatusComponent::State {} => draw_state(c, status, &layout.style, accent, x, center_y),
            StatusComponent::Summary {} => {
                if let Some(summary) = status.summary.as_deref() {
                    draw_lines(c, summary, x, y + 7.0, TEXT_RIGHT - x, theme::GRAY);
                }
            }
            StatusComponent::Progress { variant } => {
                if let Some(progress) = status.progress_pct {
                    match variant {
                        ProgressVariant::Ring => draw_progress_ring(
                            c,
                            progress,
                            accent,
                            (LEFT + WIDTH / 2.0, center_y),
                            height / 2.0 - 4.0,
                        ),
                        ProgressVariant::Bar => draw_progress_bar(c, progress, accent, x, center_y),
                    }
                }
            }
            StatusComponent::Metrics { columns } => {
                draw_metrics(c, status, usize::from(columns), density, x, y, privacy);
            }
            StatusComponent::Alert {} => {
                if let Some(alert) = status.alert.as_deref() {
                    c.fill(
                        x,
                        y,
                        TEXT_RIGHT - x,
                        height,
                        7.0,
                        paint::alpha(theme::ORANGE, 0.14),
                    );
                    let text_width = TEXT_RIGHT - x - 24.0;
                    let lead = if c.text_w(alert, size::SMALL, W::Regular) <= text_width {
                        center_y
                    } else {
                        y + 9.0
                    };
                    c.circle(x + 9.0, lead, 2.4, theme::ORANGE);
                    draw_lines(c, alert, x + 16.0, lead, text_width, theme::ORANGE);
                }
            }
            StatusComponent::UpdatedAt {} => {
                if let Some(at) = status
                    .observed_at
                    .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
                {
                    let time = at.with_timezone(&chrono::Local).format("%m-%d %H:%M");
                    c.centered_text(
                        x,
                        center_y,
                        &format!("更新于 {time}"),
                        size::TINY,
                        W::Regular,
                        theme::DIM,
                        H::Left,
                    );
                }
            }
            StatusComponent::List {} => {
                draw_list(c, status, density, x, y, privacy, interaction.as_deref_mut());
            }
            StatusComponent::Actions {} => {
                if let Some(interaction) = interaction.as_deref_mut() {
                    interaction.actions(c, status, x, y, TEXT_RIGHT - x, Some(layout));
                }
            }
            StatusComponent::Fields {} => {
                if let Some(interaction) = interaction.as_deref_mut() {
                    interaction.fields(c, status, x, y, TEXT_RIGHT - x, Some(layout));
                }
            }
            StatusComponent::Chart {} => {
                if let Some(series) = &status.series {
                    let label_height = if let Some(label) = status.series_label.as_deref() {
                        let label = c.fit(label, size::TINY, W::Regular, TEXT_RIGHT - x);
                        c.centered_text(
                            x, y + NOTE_LINE_HEIGHT / 2.0, &label, size::TINY,
                            W::Regular, theme::GRAY, H::Left,
                        );
                        NOTE_LINE_HEIGHT
                    } else {
                        0.0
                    };
                    draw_chart(c, series, layout.style.emphasis, accent, x, y + label_height, height - label_height);
                }
            }
            StatusComponent::Text {} => {
                if let Some(note) = status.note.as_deref() {
                    draw_note(c, note, density, x, y);
                }
            }
            StatusComponent::Badge {} => draw_badges(c, status, density, accent, x, y),
            StatusComponent::Counter {} => {
                let value = &status.counter[index];
                let revealed = privacy.revealed(
                    c, value.sensitive, ("counter", index, value.reveal_identity()),
                    [x, y, TEXT_RIGHT - x, height],
                );
                let text = if revealed { value.value_at(now) } else { MASKED_VALUE.into() };
                draw_realtime(c, x, center_y, None, &text);
                if revealed {
                    repaint_realtime(c, [x, y, TEXT_RIGHT - x, height], clock, value.repaint_at(now));
                }
            }
            StatusComponent::Clock {} => {
                let value = &status.clock[index];
                draw_realtime(c, x, center_y, Some(&value.label), &value.value_at(now));
                repaint_realtime(c, [x, y, TEXT_RIGHT - x, height], clock, Some(value.repaint_at(now)));
            }
            StatusComponent::Countdown {} => {
                let value = &status.countdown[index];
                draw_realtime(c, x, center_y, Some(&value.label), &value.value_at(now));
                repaint_realtime(c, [x, y, TEXT_RIGHT - x, height], clock, value.repaint_at(now));
            }
            StatusComponent::Canvas {} => {
                if let (Some(draw), Some(canvas)) = (paint_canvas.as_deref_mut(), &status.canvas) {
                    draw(c, canvas, c.rect(CANVAS_LEFT, y, CANVAS_WIDTH, height), canvas_theme);
                    if let Some(interaction) = interaction.as_deref_mut() {
                        interaction.canvas_fields(c, status, layout, CANVAS_LEFT, y);
                    }
                }
            }
        }
        y += height + gap;
    }
}

fn draw_realtime(c: &Card, x: f32, y: f32, label: Option<&str>, value: &str) {
    let width = TEXT_RIGHT - x;
    let value = c.fit(value, size::BODY, W::Semibold, if label.is_some() { width * 0.6 } else { width });
    if let Some(label) = label {
        let label = c.fit(label, size::SMALL, W::Regular, width - c.text_w(&value, size::BODY, W::Semibold) - 8.0);
        c.centered_text(x, y, &label, size::SMALL, W::Regular, theme::GRAY, H::Left);
    }
    c.centered_text(TEXT_RIGHT, y, &value, size::BODY, W::Semibold, theme::WHITE, H::Right);
}

fn repaint_realtime(
    c: &Card,
    [x, y, width, height]: [f32; 4],
    now: chrono::DateTime<chrono::Utc>,
    at: Option<i64>,
) {
    if c.live && c.rect(x, y, width, height).intersects(c.painter.clip_rect()) {
        if let Some(at) = at {
            let seconds = Duration::from_secs((at - now.timestamp()) as u64);
            let fraction = Duration::from_nanos(u64::from(now.timestamp_subsec_nanos()));
            c.ui.ctx().request_repaint_after(seconds.saturating_sub(fraction));
        }
    }
}

fn draw_state(
    c: &Card,
    status: &PluginStatus,
    style: &StatusStyle,
    accent: Accent,
    x: f32,
    y: f32,
) {
    let device_offline = status.state == StatusState::Offline && status.label != "设备离线";
    let indicator = match status.state {
        StatusState::Paused | StatusState::Error | StatusState::Offline => {
            Accent::native(state_color(status.state))
        }
        _ => accent,
    };
    indicator.draw(c, |color| c.circle(x + 3.0, y, 2.8, color));
    let mut x = x + 12.0;
    if let Some(icon) = style.icon {
        draw_icon(c, icon, x + 7.0, y);
        x += 20.0;
    }
    let reserved = if device_offline { 58.0 } else { 0.0 };
    let (size, weight) = if style.emphasis == StatusEmphasis::Strong {
        (size::TITLE + 2.0, W::Bold)
    } else {
        (size::TITLE, W::Semibold)
    };
    let label = c.fit(&status.label, size, weight, TEXT_RIGHT - x - reserved);
    c.centered_text(x, y, &label, size, weight, theme::WHITE, H::Left);
    if device_offline {
        c.centered_text(
            TEXT_RIGHT,
            y,
            "设备离线",
            size::TINY,
            W::Regular,
            theme::GRAY,
            H::Right,
        );
    }
}

fn draw_metrics(
    c: &Card,
    status: &PluginStatus,
    columns: usize,
    density: StatusDensity,
    x: f32,
    y: f32,
    privacy: &SensitiveValues,
) {
    let gap = 12.0;
    let width = (TEXT_RIGHT - x - gap * (columns - 1) as f32) / columns as f32;
    let row_height = metric_row_height(density);
    for (index, metric) in status.metrics.iter().enumerate() {
        let left = x + (index % columns) as f32 * (width + gap);
        let center_y = y + (index / columns) as f32 * row_height + row_height / 2.0;
        let revealed = privacy.revealed(
            c, metric.sensitive, ("metric", index, &metric.label, &metric.value),
            [left, center_y - row_height / 2.0, width, row_height],
        );
        let raw = if revealed { &metric.value } else { MASKED_VALUE };
        let value = c.fit(raw, size::BODY, W::Semibold, width * 0.55);
        let label_width = width - c.text_w(&value, size::BODY, W::Semibold) - 8.0;
        let label = c.fit(&metric.label, size::SMALL, W::Regular, label_width);
        c.centered_text(
            left,
            center_y,
            &label,
            size::SMALL,
            W::Regular,
            theme::GRAY,
            H::Left,
        );
        c.centered_text(
            left + width,
            center_y,
            &value,
            size::BODY,
            W::Semibold,
            theme::WHITE,
            H::Right,
        );
    }
}

fn draw_progress_bar(c: &Card, progress: f64, accent: Accent, x: f32, y: f32) {
    let width = TEXT_RIGHT - x - 55.0;
    c.fill(x, y - 4.0, width, 8.0, 4.0, theme::TRACK);
    if progress > 0.0 {
        let filled = width * progress as f32 / 100.0;
        accent.draw(c, |color| {
            c.fill(x, y - 4.0, filled, 8.0, 4.0_f32.min(filled / 2.0), color)
        });
    }
    c.centered_text(
        TEXT_RIGHT - 12.0,
        y,
        &progress_text(progress),
        17.0,
        W::Bold,
        theme::WHITE,
        H::Right,
    );
    c.centered_text(
        TEXT_RIGHT,
        y + 2.0,
        "%",
        9.0,
        W::Semibold,
        theme::GRAY,
        H::Right,
    );
}

fn draw_progress_ring(c: &Card, progress: f64, accent: Accent, center: (f32, f32), radius: f32) {
    let (x, y) = center;
    c.track(x, y, radius, 4.0, theme::TRACK);
    accent.draw(c, |color| {
        if progress >= 100.0 {
            c.track(x, y, radius, 4.0, color);
        } else if progress > 0.0 {
            let start = -FRAC_PI_2;
            let end = start + TAU * progress as f32 / 100.0;
            c.arc(x, y, radius, start, end, 4.0, color);
            for angle in [start, end] {
                c.circle(
                    x + radius * angle.cos(),
                    y + radius * angle.sin(),
                    2.0,
                    color,
                );
            }
        }
    });
    let percent = progress_text(progress);
    let size = if percent.len() > 2 { 19.0 } else { 22.0 };
    let width = c.text_w(&percent, size, W::Bold);
    let left = x - (width + c.text_w("%", 9.0, W::Semibold) + 1.0) / 2.0;
    c.centered_text(left, y, &percent, size, W::Bold, theme::WHITE, H::Left);
    c.centered_text(
        left + width + 1.0,
        y + 3.0,
        "%",
        9.0,
        W::Semibold,
        theme::GRAY,
        H::Left,
    );
}

fn progress_text(progress: f64) -> String {
    if progress == 0.0 {
        "0".into()
    } else if progress < 1.0 {
        "<1".into()
    } else {
        format!("{:.0}", progress.floor())
    }
}

fn draw_lines(c: &Card, text: &str, x: f32, y: f32, width: f32, color: Color32) {
    let mut lines = c.wrap(text, size::SMALL, W::Regular, width, 2);
    if lines.iter().map(|line| line.chars().count()).sum::<usize>() < text.chars().count() {
        if let Some(last) = lines.last_mut() {
            *last = c.fit(
                &format!("{}…", last.trim_end_matches('…')),
                size::SMALL,
                W::Regular,
                width,
            );
        }
    }
    for (i, line) in lines.iter().enumerate() {
        c.centered_text(
            x,
            y + i as f32 * 13.0,
            line,
            size::SMALL,
            W::Regular,
            color,
            H::Left,
        );
    }
}

fn draw_unavailable(c: &Card, health: StatusHealth) {
    let (name, hint, color) = match health {
        StatusHealth::Waiting | StatusHealth::Ready => ("等待数据", None, theme::DIM),
        StatusHealth::Unconfigured => {
            ("等待数据源授权", Some("可在插件设置中授权读取"), theme::DIM)
        }
        StatusHealth::Offline => (
            "数据源失联",
            Some("检查已授权的文件或本机接口"),
            theme::ORANGE,
        ),
        StatusHealth::Stale => ("状态已过期", None, theme::ORANGE),
        StatusHealth::Invalid => ("状态数据格式无效", None, theme::ORANGE),
        StatusHealth::CanvasInvalid => ("插件画面有误", None, theme::ORANGE),
    };
    draw_message(c, name, hint, color);
}

fn draw_message(c: &Card, name: &str, hint: Option<&str>, color: Color32) {
    c.fill(LEFT, 56.0, WIDTH, 78.0, 9.0, theme::CARD);
    let x = LEFT + WIDTH / 2.0;
    let y = if hint.is_some() { 86.0 } else { 95.0 };
    c.centered_text(x, y, name, size::TITLE, W::Semibold, color, H::Center);
    if let Some(hint) = hint {
        c.centered_text(
            x,
            107.0,
            hint,
            size::SMALL,
            W::Regular,
            theme::GRAY,
            H::Center,
        );
    }
}
