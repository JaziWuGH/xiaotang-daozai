//! 状态页扩展组件和宿主图标。

use super::{metric_row_height, state_color, Accent, SensitiveValues, MASKED_VALUE, TEXT_RIGHT};
use crate::{
    paint::{self, Card, H},
    plugin_status::{PluginStatus, StatusDensity, StatusEmphasis, StatusIcon, MAX_ITEMS},
    theme::{self, size, W},
};
use eframe::egui::{self, Color32};

pub(super) const NOTE_LINE_HEIGHT: f32 = 13.0;
pub(super) const BADGE_ROW_HEIGHT: f32 = 20.0;

pub(super) fn list_rows(density: StatusDensity, count: usize) -> usize {
    count.min(if density == StatusDensity::Compact {
        3
    } else {
        MAX_ITEMS
    })
}

pub(super) fn note_lines(density: StatusDensity, note: Option<&str>) -> usize {
    let limit = if density == StatusDensity::Compact {
        2
    } else {
        14
    };
    note.map_or(limit, |note| {
        note.lines()
            .map(|line| line.chars().count().max(1).div_ceil(20))
            .sum::<usize>()
            .clamp(1, limit)
    })
}

pub(super) fn badge_count(density: StatusDensity, count: usize) -> usize {
    count.min(if density == StatusDensity::Compact {
        3
    } else {
        4
    })
}

pub(super) fn draw_icon(c: &Card, icon: StatusIcon, x: f32, y: f32) {
    match icon {
        StatusIcon::Battery => paint::battery(c, x, y, None, false, 0.72),
        StatusIcon::Cup => paint::coffee(c, x, y, theme::WHITE),
        StatusIcon::Clock => paint::stopwatch(&c.scaled(x, y, 0.60), x, y, 8.0, 0.5, theme::WHITE),
        StatusIcon::Cloud => crate::weather_ui::icon(c, x, y, 14.0, crate::weather::Kind::Cloud),
        StatusIcon::Chart | StatusIcon::Calendar => {
            let rect = c.rect(x - 7.0, y - 7.0, 14.0, 14.0);
            let image = egui::Image::new(match icon {
                StatusIcon::Chart => egui::include_image!("../assets/settings-icons/system/performance.svg"),
                _ => egui::include_image!("../assets/settings-icons/calendar-days.svg"),
            });
            if let Ok(egui::load::TexturePoll::Ready { texture }) = image
                .fit_to_exact_size(rect.size())
                .load_for_size(c.ui.ctx(), rect.size())
            {
                let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                c.painter.image(texture.id, rect, uv, Color32::WHITE);
            }
        }
    }
}

pub(super) fn draw_list(
    c: &Card,
    status: &PluginStatus,
    density: StatusDensity,
    x: f32,
    y: f32,
    privacy: &SensitiveValues,
    mut interaction: Option<&mut super::Interaction<'_>>,
) {
    let row_height = metric_row_height(density);
    let count = list_rows(density, status.items.len());
    for (index, item) in status.items.iter().take(count).enumerate() {
        let top = y + index as f32 * row_height;
        let center_y = top + row_height / 2.0;
        let has_actions = interaction.is_some()
            && (item.action.is_some() || item.open.is_some() || item.copy.is_some());
        let identity = (
            "item", index, &item.label, &item.detail, &item.action, &item.open, &item.copy,
        );
        let bounds = [x, top, TEXT_RIGHT - x, row_height];
        let (revealed, actions_allowed) = if has_actions {
            privacy.revealed_for_actions(c, item.sensitive, identity, bounds)
        } else {
            (privacy.revealed(c, item.sensitive, identity, bounds), false)
        };
        let right = if let Some(interaction) = interaction.as_deref_mut().filter(|_| actions_allowed) {
            x + interaction.list_target(c, item, index, x, top, TEXT_RIGHT - x, row_height) - 4.0
        } else {
            TEXT_RIGHT
        };
        let label_x = x + 12.0;
        if let Some(state) = item.state.filter(|_| revealed) {
            c.circle(x + 3.0, center_y, 2.5, state_color(state));
        }
        let detail = item.detail.as_deref().filter(|_| revealed).map_or_else(
            String::new,
            |detail| c.fit(detail, size::SMALL, W::Regular, (right - label_x) * 0.55),
        );
        let detail_width = c.text_w(&detail, size::SMALL, W::Regular);
        let label_width = right - label_x - detail_width - 8.0;
        let raw = if revealed { &item.label } else { MASKED_VALUE };
        let label = c.fit(raw, size::SMALL, W::Semibold, label_width);
        c.centered_text(
            label_x,
            center_y,
            &label,
            size::SMALL,
            W::Semibold,
            theme::WHITE,
            H::Left,
        );
        c.centered_text(
            right,
            center_y,
            &detail,
            size::SMALL,
            W::Regular,
            theme::GRAY,
            H::Right,
        );
    }
}

pub(super) fn draw_chart(
    c: &Card,
    series: &[f64],
    emphasis: StatusEmphasis,
    accent: Accent,
    x: f32,
    y: f32,
    height: f32,
) {
    let left = x + 4.0;
    let right = TEXT_RIGHT - 4.0;
    let top = y + 4.0;
    let bottom = y + height - 4.0;
    let grid = paint::alpha(theme::DIM, 0.18);
    for row in [top, (top + bottom) * 0.5, bottom] {
        c.seg((x, row), (TEXT_RIGHT, row), 1.0, grid);
    }
    for fraction in [0.25, 0.5, 0.75] {
        let column = x + (TEXT_RIGHT - x) * fraction;
        c.seg((column, top), (column, bottom), 1.0, grid);
    }

    let (min, max) = series
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), &value| {
            (min.min(value), max.max(value))
        });
    // 有限采样值相减也可能溢出。两端同时缩小一半就能保留比例。
    let factor = if (max - min).is_finite() { 1.0 } else { 0.5 };
    let span = max * factor - min * factor;
    let steps = (series.len() - 1) as f32;
    let points: Vec<(f32, f32)> = series
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let height_ratio = if span == 0.0 {
                0.5
            } else {
                (value * factor - min * factor) / span
            } as f32;
            (
                left + (right - left) * i as f32 / steps,
                bottom - (bottom - top) * height_ratio,
            )
        })
        .collect();
    accent.draw(c, |color| {
        let mut mesh = egui::Mesh::default();
        let fill = paint::alpha(color, 0.24);
        for &(px, py) in &points {
            mesh.colored_vertex(c.pos(px, py), fill);
            mesh.colored_vertex(c.pos(px, bottom), Color32::TRANSPARENT);
        }
        for i in 0..points.len() as u32 - 1 {
            let k = i * 2;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 3, k + 2);
        }
        c.painter.add(egui::Shape::mesh(mesh));
        let width = if emphasis == StatusEmphasis::Strong {
            2.3
        } else {
            1.6
        };
        c.path(&points, width, color);
        if let Some(&(px, py)) = points.last() {
            c.circle(px, py, 3.0, color);
        }
    });
}

pub(super) fn draw_note(c: &Card, note: &str, density: StatusDensity, x: f32, y: f32) {
    let width = TEXT_RIGHT - x;
    let max_lines = note_lines(density, Some(note));
    let note = note.replace("\r\n", "\n");
    let mut lines = c.wrap(&note, size::SMALL, W::Regular, width, max_lines);
    let visible_chars = lines.iter().map(|line| line.chars().count()).sum::<usize>();
    let note_chars = note.chars().filter(|&ch| ch != '\n').count();
    if visible_chars < note_chars {
        if let Some(last) = lines.last_mut() {
            *last = c.fit(
                &format!("{}…", last.trim_end_matches('…')),
                size::SMALL,
                W::Regular,
                width,
            );
        }
    }
    for (index, line) in lines.iter().enumerate() {
        let center_y = y + (index as f32 + 0.5) * NOTE_LINE_HEIGHT;
        c.centered_text(
            x,
            center_y,
            line,
            size::SMALL,
            W::Regular,
            theme::GRAY,
            H::Left,
        );
    }
}

pub(super) fn draw_badges(
    c: &Card,
    status: &PluginStatus,
    density: StatusDensity,
    accent: Accent,
    x: f32,
    y: f32,
) {
    let columns = if density == StatusDensity::Compact {
        3
    } else {
        2
    };
    let count = badge_count(density, status.tags.len());
    let max_width = (TEXT_RIGHT - x - 8.0 * (columns - 1) as f32) / columns as f32;
    let mut left = x;
    for (index, tag) in status.tags.iter().take(count).enumerate() {
        if index % columns == 0 {
            left = x;
        }
        let top = y + (index / columns) as f32 * BADGE_ROW_HEIGHT;
        let text = c.fit(tag, size::TINY, W::Semibold, max_width - 14.0);
        let width = c.text_w(&text, size::TINY, W::Semibold) + 14.0;
        accent.draw(c, |color| {
            c.fill(left, top + 2.0, width, 16.0, 8.0, paint::alpha(color, 0.18))
        });
        c.centered_text(
            left + width * 0.5,
            top + 10.0,
            &text,
            size::TINY,
            W::Semibold,
            theme::WHITE,
            H::Center,
        );
        left += width + 8.0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use serde_json::json;

    fn contrast(color: Color32, background: Color32) -> f32 {
        let rgb = |c: Color32| [c.r(), c.g(), c.b()].map(|v| f32::from(v) / 255.0);
        crate::glass::contrast(rgb(color), rgb(background))
    }

    #[test]
    fn plugin_status_accent_preserves_readable_colors_and_corrects_low_contrast() {
        let blue = Color32::from_rgb(91, 173, 239);
        let dark = Color32::from_gray(24);
        assert_eq!(readable_accent(blue, dark), blue);
        for background in [
            dark,
            Color32::from_gray(118),
            Color32::from_rgb(250, 243, 228),
        ] {
            let accent = readable_accent(background, background);
            assert!(contrast(accent, background) >= 3.0);
            assert_ne!(accent, background);
        }
    }

    #[test]
    fn plugin_status_components_fit_at_both_densities_skins_and_scales() {
        let status: PluginStatus = serde_json::from_value(json!({
            "state":"running", "label":"最长测试标题已进入打印队列".repeat(3),
            "progress_pct":95,
            "items":[
                {"label":"任务","detail":"非常长的文件名称".repeat(8),"state":"running"},
                {"label":"列表第二条","state":"paused"},
                {"label":"列表第三条","state":"idle"},
                {"label":"列表第四条","state":"succeeded"}
            ],
            "series":[-f64::MAX, 0.0, f64::MAX],
            "series_label":"近期读数",
            "clock":{"label":"北京","utc_offset_minutes":480,"format":"HH:mm:ss"},
            "note":"冷却风扇运行正常\n准备打印下一件",
            "tags":["精细打印","模拟","深度观测","标签四"]
        }))
        .unwrap();
        assert!(status.valid());

        for (palette, island, dark) in [
            (None, Color32::BLACK, true),
            (
                Some(skin_paint::CREAM),
                Color32::from_rgb(255, 243, 228),
                false,
            ),
            (Some(skin_paint::GLASS_LIGHT), Color32::WHITE, false),
            (Some(skin_paint::GLASS_DARK), Color32::from_gray(16), true),
        ] {
            let backdrop = StatusBackdrop { palette, island };
            for density in ["comfortable", "compact"] {
                for scale in [1.0, 2.0] {
                    let ctx = egui::Context::default();
                    theme::load_fonts(&ctx);
                    ctx.set_visuals(if dark {
                        egui::Visuals::dark()
                    } else {
                        egui::Visuals::light()
                    });
                    let layout: StatusLayout = serde_json::from_value(json!({
                        "components":[{"type":"state"},{"type":"list"},{"type":"chart"},{"type":"clock"},{"type":"text"},{"type":"badge"}],
                        "style":{"density":density,"emphasis":"strong","icon":"clock","accent":"#F0F0F0"}
                    })).unwrap();
                    let background = backdrop.card_color(layout.style.surface);
                    let accent = readable_accent(Color32::from_gray(240), background);
                    let canvas = egui::Rect::from_min_max(
                        egui::Pos2::ZERO,
                        egui::pos2(419.0, MAX_PAGE_HEIGHT),
                    );
                    let input = egui::RawInput {
                        screen_rect: Some(canvas),
                        viewports: [(
                            egui::ViewportId::ROOT,
                            egui::ViewportInfo {
                                native_pixels_per_point: Some(scale),
                                ..Default::default()
                            },
                        )]
                        .into_iter()
                        .collect(),
                        ..Default::default()
                    };
                    let output = ctx.run(input, |ctx| {
                        assert_eq!(ctx.pixels_per_point(), scale);
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                let c = Card::new(ui, egui::Pos2::ZERO, canvas);
                                let layer = c.painter.layer_id();
                                let from = skin_paint::shape_count(ctx, layer);
                                draw_status_page(
                                    &c,
                                    "printer",
                                    Some(&status),
                                    StatusHealth::Ready,
                                    Some(&layout),
                                    backdrop,
                                    false,
                                    None,
                                );
                                let to = skin_paint::shape_count(ctx, layer);
                                if let Some(palette) = palette {
                                    skin_paint::recolor_layer(ctx, layer, &[(from, to)], &palette);
                                }
                            });
                    });

                    let card = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Rect(rect) if rect.rect.width() == WIDTH => {
                                Some(rect.rect)
                            }
                            _ => None,
                        })
                        .unwrap();
                    let texts: Vec<&str> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) => {
                                let bounds = text.visual_bounding_rect();
                                assert!(
                                    card.contains_rect(bounds),
                                    "{}: {bounds:?}",
                                    text.galley.text()
                                );
                                Some(text.galley.text())
                            }
                            _ => None,
                        })
                        .collect();
                    for label in [
                        "列表第二条",
                        "列表第三条",
                        "冷却风扇运行正常",
                        "准备打印下一件",
                        "精细打印",
                        "近期读数",
                        "北京",
                    ] {
                        assert!(texts.contains(&label), "{label}: {density}, {scale}");
                    }
                    assert_eq!(texts.contains(&"列表第四条"), density == "comfortable");
                    assert_eq!(texts.contains(&"标签四"), density == "comfortable");
                    assert!(contrast(accent, background) >= 3.0);
                    assert!(
                        output.shapes.iter().any(|shape| matches!(
                            &shape.shape,
                            egui::Shape::Circle(end) if end.fill == accent && end.radius == 3.0
                        )),
                        "折线末点应使用换肤后的校正色"
                    );
                    assert!(ctx
                        .tessellate(output.shapes, scale)
                        .iter()
                        .all(|primitive| {
                            match &primitive.primitive {
                                egui::epaint::Primitive::Mesh(mesh) => {
                                    mesh.vertices.iter().all(|v| v.pos.is_finite())
                                }
                                _ => true,
                            }
                        }));
                }
            }
        }
    }
}
