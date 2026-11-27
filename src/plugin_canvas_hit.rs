//! 画布点按、悬停与滚轮都在宿主内处理。

use super::{
    geometry::{self, Clip, Transform},
    motion::{group, mix_color, num, LiveOptions, LiveResult},
    CanvasTheme,
};
use crate::{
    paint::{Card, H},
    plugin_canvas::{
        live::{Hit, Hover, Tip},
        Kind, Node, Paint, Token,
    },
    skin_paint, theme,
};
use eframe::egui::{self, pos2, Color32, Pos2, Rect};
use std::collections::BTreeMap;

pub(super) type NodeKey = egui::Id;

pub(super) fn key(parent: NodeKey, index: usize, node: &Node) -> NodeKey {
    node.id.as_ref().map_or_else(
        || parent.with(index),
        |id| egui::Id::new(("canvas-node", id)),
    )
}

#[derive(Clone, Default)]
pub(super) struct InputState {
    scroll: BTreeMap<u64, f32>,
    hover: BTreeMap<u64, HoverTrack>,
}

#[derive(Clone, Copy)]
struct HoverTrack {
    from: f32,
    entered: bool,
    at: f64,
}

impl HoverTrack {
    fn value(self, time: f64, duration: f64) -> f32 {
        let t = ((time - self.at) / duration).clamp(0.0, 1.0) as f32;
        self.from + (f32::from(u8::from(self.entered)) - self.from) * t
    }
}

pub(super) struct Input<'a, 'ui> {
    pub c: &'a Card<'ui>,
    pub area: Rect,
    pub theme: CanvasTheme,
    pub options: LiveOptions,
    pub id: NodeKey,
}

struct Region<'a> {
    key: NodeKey,
    bounds: Rect,
    clip: Clip,
    hit: Option<&'a Hit>,
    hover: Option<&'a Hover>,
    scroll_end: Option<f32>,
    scroll_scale: f32,
    tip: Option<TipAt>,
}

pub(super) enum HoverOverlay {
    Chart(TipAt),
    Hint { text: String, anchor: Rect },
}

impl HoverOverlay {
    pub fn draw(self, input: &Input<'_, '_>) {
        let (text, anchor) = match self {
            Self::Chart(tip) => return tip.draw(input),
            Self::Hint { text, anchor } => (text, anchor),
        };
        let c = input.c;
        let area = input.area.intersect(c.painter.clip_rect());
        let min = c.to_local(area.min);
        let max = c.to_local(area.max);
        let height = 22.0;
        let available = max.x - min.x;
        if available < 24.0 || max.y - min.y < height {
            return;
        }
        let width = (c.text_w(&text, 11.0, theme::W::Semibold) + 16.0).min(available);
        let x = (c.to_local(anchor.center()).x - width / 2.0).clamp(min.x, (max.x - width).max(min.x));
        let above = c.to_local(anchor.min).y - height - 4.0;
        let below = c.to_local(anchor.max).y + 4.0;
        let y = if above >= min.y { above } else { below }.clamp(min.y, (max.y - height).max(min.y));
        let layer = c.painter.layer_id();
        let first = skin_paint::shape_count(c.ui.ctx(), layer);
        let clipped = c.clipped(min.x, min.y, available, max.y - min.y);
        let background = crate::paint::mix(input.theme.background, input.theme.token(&Token::Card), 0.75);
        clipped.fill(x, y, width, height, 6.0, background);
        clipped.stroke(x, y, width, height, 6.0, 1.0, input.theme.token(&Token::Dim).gamma_multiply(0.5));
        clipped.centered_text(
            x + width / 2.0, y + height / 2.0,
            &clipped.fit(&text, 11.0, theme::W::Semibold, width - 16.0),
            11.0, theme::W::Semibold, input.theme.token(&Token::Text), H::Center,
        );
        skin_paint::tone_since(c.ui.ctx(), layer, first, |color, _| color);
    }
}

#[derive(Clone)]
pub(super) struct TipAt {
    tip: Tip,
    x: f32,
    chart: Rect,
}

impl TipAt {
    pub fn draw(self, input: &Input<'_, '_>) {
        let c = input.c;
        let area = input.area.intersect(c.painter.clip_rect());
        let p = c.to_local(pos2(self.x, self.chart.bottom()));
        let y = c.to_local(self.chart.min).y;
        let theme = input.theme;
        let layer = c.painter.layer_id();
        let first = skin_paint::shape_count(c.ui.ctx(), layer);
        let ink = theme.token(&crate::plugin_canvas::Token::Text);
        let card = theme.token(&crate::plugin_canvas::Token::Card);
        let dim = theme.token(&crate::plugin_canvas::Token::Dim);
        let width = (c
            .text_w(&self.tip.label, 10.0, theme::W::Regular)
            .max(c.text_w(&self.tip.value, 11.0, theme::W::Semibold))
            + 16.0)
            .min(area.width() / c.s);
        let left = c.to_local(area.min).x;
        let right = c.to_local(area.max).x - width;
        let x = (p.x + 7.0).clamp(left, right.max(left));
        let top = (y + 4.0)
            .min(c.to_local(area.max).y - 37.0)
            .max(c.to_local(area.min).y);
        let clipped = c.clipped(
            left,
            c.to_local(area.min).y,
            area.width() / c.s,
            area.height() / c.s,
        );
        clipped.seg((p.x, y), (p.x, p.y), 1.0, dim);
        clipped.fill(x, top, width, 37.0, 6.0, card);
        clipped.stroke(x, top, width, 37.0, 6.0, 1.0, dim.gamma_multiply(0.4));
        clipped.centered_text(
            x + 8.0,
            top + 10.0,
            &clipped.fit(&self.tip.label, 10.0, theme::W::Regular, width - 16.0),
            10.0,
            theme::W::Regular,
            dim,
            H::Left,
        );
        clipped.centered_text(
            x + 8.0,
            top + 25.0,
            &clipped.fit(&self.tip.value, 11.0, theme::W::Semibold, width - 16.0),
            11.0,
            theme::W::Semibold,
            ink,
            H::Left,
        );
        skin_paint::tone_since(c.ui.ctx(), layer, first, |color, _| color);
    }
}

impl InputState {
    pub fn apply(
        &mut self,
        input: &Input<'_, '_>,
        nodes: &mut [Node],
        time: f64,
        result: &mut LiveResult,
    ) -> Option<HoverOverlay> {
        if !input.options.interactive {
            return None;
        }
        let c = input.c;
        let ctx = c.ui.ctx();
        let pointer = ctx.input(|state| state.pointer.hover_pos());
        let root = Transform::at(c.s, input.area.min);
        let root_clip = Clip::new(input.area.intersect(c.painter.clip_rect()));
        let mut regions = Vec::new();
        self.regions(c, nodes, root, &root_clip, input.id, pointer, &mut regions);
        let under_pointer =
            |r: &&Region<'_>| pointer.is_some_and(|p| r.bounds.contains(p) && r.clip.contains(p));
        let hovered = regions
            .iter()
            .rev()
            .filter(under_pointer)
            .find(|r| r.hover.is_some())
            .map(|r| r.key);
        let tip = regions
            .iter()
            .rev()
            .filter(under_pointer)
            .find_map(|r| {
                r.hover.and_then(|hover| hover.hint.as_ref()).map(|hint| HoverOverlay::Hint {
                    text: hint.clone(),
                    anchor: r.bounds,
                }).or_else(|| r.tip.clone().map(HoverOverlay::Chart))
            });
        if let Some(scroll) = regions
            .iter()
            .rev()
            .filter(under_pointer)
            .find(|r| r.scroll_end.is_some())
        {
            result.scroll_area = Some(scroll.bounds.intersect(scroll.clip.rect));
            let dy = ctx.input_mut(|state| {
                let dy = state.smooth_scroll_delta.y;
                state.smooth_scroll_delta.y = 0.0;
                state.raw_scroll_delta.y = 0.0;
                dy
            });
            if dy != 0.0 {
                let offset = self.scroll.entry(scroll.key.value()).or_default();
                *offset = (*offset - dy / scroll.scroll_scale)
                    .clamp(0.0, scroll.scroll_end.unwrap_or_default());
                result.animating = true;
            }
        }
        for region in &regions {
            let Some(action) = region
                .hit
                .filter(|hit| input.options.allow_action || !matches!(hit, Hit::Action(_)))
            else {
                continue;
            };
            let rect = region.bounds.intersect(region.clip.rect);
            if !rect.is_positive() {
                continue;
            }
            let id = input.id.with((input.options.revision, region.key));
            let response = c.ui.interact(rect, id, egui::Sense::click());
            if response.hovered() && under_pointer(&region) {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() && (pointer.is_none() || under_pointer(&region)) {
                result.hit = Some(action.clone());
            }
        }
        self.scroll.retain(|id, _| {
            regions
                .iter()
                .any(|region| region.key.value() == *id && region.scroll_end.is_some())
        });
        self.visuals(input, nodes, input.id, hovered, time, &mut result.animating);
        tip
    }

    fn regions<'a>(
        &self,
        c: &Card<'_>,
        nodes: &'a [Node],
        transform: Transform,
        clip: &Clip,
        parent: NodeKey,
        pointer: Option<Pos2>,
        regions: &mut Vec<Region<'a>>,
    ) {
        if !clip.rect.is_positive() || !(0.0 < transform.scale && transform.scale <= 1024.0) {
            return;
        }
        for (index, node) in nodes.iter().enumerate() {
            if node.style.opacity.initial() <= 0.0 {
                continue;
            }
            let key = key(parent, index, node);
            let scroll = node.scroll.as_ref();
            if node.hit.is_some()
                || node.hover.is_some()
                || scroll.is_some()
                || !node.tips.is_empty()
            {
                let bounds = bounds(c, node, transform).intersect(clip.rect);
                if bounds.is_positive() {
                    let scale = match &node.kind {
                        Kind::Group { scale, .. } => transform.scale * scale.initial(),
                        _ => transform.scale,
                    };
                    let scroll_end = scroll
                        .map(|scroll| scroll.content - bounds.height() / scale)
                        .filter(|size| *size > 0.0);
                    let tip = pointer.and_then(|p| {
                        let local = transform.local(p);
                        let tip = node.tips.iter().min_by(|a, b| {
                            (a.x - local.x).abs().total_cmp(&(b.x - local.x).abs())
                        })?;
                        Some(TipAt {
                            tip: tip.clone(),
                            x: transform.xy(tip.x, local.y).x,
                            chart: bounds,
                        })
                    });
                    regions.push(Region {
                        key,
                        bounds,
                        clip: clip.clone(),
                        hit: node.hit.as_ref(),
                        hover: node.hover.as_ref(),
                        scroll_end,
                        scroll_scale: scale,
                        tip,
                    });
                }
            }
            if let Kind::Group {
                group,
                translate,
                rotate,
                scale,
                clip: inner,
            } = &node.kind
            {
                let mut child = transform.child(translate, rotate.initial(), scale.initial());
                let clipped = inner.as_ref().map(|bounds| clip.child(bounds, child));
                if let Some(offset) = self.scroll.get(&key.value()) {
                    child = child.child(&[num(0.0), num(-offset)], 0.0, 1.0);
                }
                self.regions(
                    c,
                    group,
                    child,
                    clipped.as_ref().unwrap_or(clip),
                    key,
                    pointer,
                    regions,
                );
            }
        }
    }

    fn visuals(
        &mut self,
        input: &Input<'_, '_>,
        nodes: &mut [Node],
        parent: NodeKey,
        hovered: Option<NodeKey>,
        time: f64,
        animating: &mut bool,
    ) {
        for (index, node) in nodes.iter_mut().enumerate() {
            let key = key(parent, index, node);
            if let Kind::Group {
                group: children, ..
            } = &mut node.kind
            {
                self.visuals(input, children, key, hovered, time, animating);
                if let Some(offset) = self
                    .scroll
                    .get(&key.value())
                    .copied()
                    .filter(|offset| *offset > 0.0)
                {
                    *children = vec![group(std::mem::take(children), [0.0, -offset], 0.0, None)];
                }
            }
            let Some(hover) = &node.hover else { continue };
            let entered = Some(key) == hovered;
            let duration = node
                .animate
                .as_ref()
                .map_or(0.0, |animation| f64::from(animation.ms) / 1000.0);
            let target = f32::from(u8::from(entered));
            let mut value = target;
            if duration > 0.0 {
                let track = self.hover.entry(key.value()).or_insert(HoverTrack {
                    from: 0.0,
                    entered: false,
                    at: time,
                });
                if entered != track.entered {
                    *track = HoverTrack {
                        from: track.value(time, duration),
                        entered,
                        at: time,
                    };
                }
                value = track.value(time, duration);
                *animating |= value != target;
            }
            if let Some(opacity) = &hover.opacity {
                let start = node.style.opacity.initial();
                node.style.opacity = num(start + (opacity.initial() - start) * value);
            }
            if let Some(fill) = &hover.fill {
                let before = match &node.style.fill {
                    Some(Paint::Solid(color)) => input.theme.color(color),
                    _ => Color32::TRANSPARENT,
                };
                if value > 0.0 {
                    node.style.fill = Some(Paint::Solid(mix_color(
                        before,
                        input.theme.color(fill),
                        value,
                    )));
                }
            }
            if let Some(lift) = hover
                .lift
                .as_ref()
                .map(|lift| lift.initial() * value)
                .filter(|lift| *lift != 0.0)
            {
                *node = group(vec![node.clone()], [0.0, -lift], 0.0, None);
            }
        }
    }
}

pub(super) fn bounds(c: &Card<'_>, node: &Node, transform: Transform) -> Rect {
    if !(0.0 < transform.scale && transform.scale <= 1024.0) {
        return Rect::NOTHING;
    }
    let width = node
        .style
        .stroke
        .as_ref()
        .map_or(0.0, |_| node.style.width.initial() * transform.scale * 0.5);
    let box_rect = match &node.kind {
        Kind::Rect { rect, .. } | Kind::Image { rect, .. } | Kind::Icon { rect, .. } => transform.bounds(rect),
        Kind::Pet { at, size, .. } => {
            let center = transform.point(at);
            let size = size.initial() * transform.scale;
            Rect::from_center_size(center, egui::vec2(size, size * 0.7))
        }
        Kind::Circle([x, y, r]) => Rect::from_center_size(
            transform.xy(x.initial(), y.initial()),
            egui::Vec2::splat(2.0 * r.initial() * transform.scale),
        ),
        Kind::Arc {
            c: center,
            r,
            start,
            sweep,
        } => Rect::from_points(
            &geometry::arc(
                transform.point(center),
                r.initial() * transform.scale,
                start.initial() + transform.angle.to_degrees(),
                sweep.initial(),
                c.ui.ctx().pixels_per_point(),
            )
            .points,
        ),
        Kind::Line([x0, y0, x1, y1]) => Rect::from_two_pos(
            transform.xy(x0.initial(), y0.initial()),
            transform.xy(x1.initial(), y1.initial()),
        ),
        Kind::Polyline { points, area } => {
            let mut bounds = Rect::from_points(
                &points
                    .iter()
                    .map(|point| transform.point(point))
                    .collect::<Vec<_>>(),
            );
            if let (Some(first), Some(last), Some(area)) = (points.first(), points.last(), area) {
                bounds = bounds.union(Rect::from_two_pos(
                    transform.xy(first[0].initial(), area.initial()),
                    transform.xy(last[0].initial(), area.initial()),
                ));
            }
            bounds
        }
        Kind::Path(commands) => geometry::path(
            commands,
            transform,
            c.ui.ctx().pixels_per_point(),
            // 点击区域要算完整外框，不受绘制预算限制。
            None,
        )
        .into_iter()
        .flatten()
        .fold(Rect::NOTHING, |rect, contour| {
            rect.union(Rect::from_points(&contour.points))
        }),
        Kind::Text {
            text,
            at,
            size,
            weight,
            align,
            max_width,
            lines, font,
            ..
        } => {
            let pixels_per_point = c.ui.ctx().pixels_per_point();
            let galley = c.ui.fonts(|fonts| {
                fonts.layout_job(super::text_job(
                    text,
                    (size.initial() * transform.scale).min(256.0 / pixels_per_point),
                    *weight,
                    align,
                    max_width.as_ref().map(|w| w.initial() * transform.scale),
                    *lines,
                    font.as_deref(),
                    &fonts.families(),
                ))
            });
            let base = galley
                .rows
                .first()
                .and_then(|row| row.glyphs.first())
                .map_or(0.0, |glyph| glyph.pos.y);
            let rotation = egui::emath::Rot2::from_angle(transform.angle);
            galley
                .rect
                .translate(egui::vec2(0.0, -base))
                .rotate_bb(rotation)
                .translate(transform.point(at).to_vec2())
        }
        Kind::Group {
            group,
            clip,
            translate,
            rotate,
            scale,
        } => {
            let child = transform.child(translate, rotate.initial(), scale.initial());
            match clip {
                Some(clip) if node.scroll.is_some() => child.bounds(clip),
                _ => {
                    let bounds = group.iter().fold(Rect::NOTHING, |rect, node| {
                        rect.union(bounds(c, node, child))
                    });
                    clip.as_ref()
                        .map_or(bounds, |clip| bounds.intersect(child.bounds(clip)))
                }
            }
        }
    };
    box_rect.expand(width)
}
