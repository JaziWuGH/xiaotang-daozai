//! 画布声明动画的宿主状态和逐帧求值。

#[cfg(test)]
#[path = "plugin_canvas_live_tests.rs"]
mod tests;
#[path = "plugin_canvas_transition.rs"]
mod transition;

use super::{
    draw_static,
    geometry::Transform,
    hit::{self, Input, InputState, NodeKey},
    pet::{DrawPets, Runner},
    CanvasTheme, CACHE_LIMIT,
};
use crate::{
    paint::Card,
    pet_expressions::ExpressionSheets,
    plugin_canvas::{
        live::{Burst, Hit, LoopKind},
        Align, Canvas, Color, Kind, Node, Num, Paint, Style,
    },
};
use chrono::{DateTime, Utc};
use eframe::egui::{self, Color32, Rect, Vec2};
use std::{
    borrow::Cow,
    collections::{BTreeMap, VecDeque},
    f32::consts::TAU,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Copy)]
pub(crate) struct LiveOptions {
    pub now: DateTime<Utc>,
    pub revision: u64,
    pub allow_action: bool,
    pub interactive: bool,
}

#[derive(Default)]
pub(crate) struct LiveResult {
    pub timed_out: bool,
    pub hit: Option<Hit>,
    pub scroll_area: Option<Rect>,
    pub repaint_after: Option<Duration>,
    pub animating: bool,
}

impl LiveResult {
    fn schedule(&mut self, delay: Duration) {
        self.repaint_after = Some(self.repaint_after.map_or(delay, |old| old.min(delay)));
    }
}

#[derive(Clone, Default)]
struct LiveState {
    input: InputState,
    revision: Option<u64>,
    before: Option<(f64, Arc<Canvas>)>,
    current: Option<Arc<Canvas>>,
    visible_frame: Option<Arc<Canvas>>,
    transition_frame: Option<Arc<Canvas>>,
    rolls: BTreeMap<u64, Roll>,
    bursts: BTreeMap<u64, f64>,
    pet: Arc<Mutex<Runner>>,
}

#[derive(Clone)]
struct Roll {
    before: String,
    text: String,
    at: f64,
}

#[cfg(test)]
pub(crate) fn draw_live(
    c: &Card<'_>,
    id: &str,
    canvas: &Arc<Canvas>,
    area: Rect,
    theme: CanvasTheme,
    options: LiveOptions,
    image: impl FnMut(&str) -> Option<(egui::TextureId, Vec2)>,
) -> LiveResult {
    draw_live_with_pet(c, id, canvas, area, theme, options, None, image)
}

pub(crate) fn draw_live_with_pet(
    c: &Card<'_>,
    id: &str,
    canvas: &Arc<Canvas>,
    area: Rect,
    theme: CanvasTheme,
    options: LiveOptions,
    sheets: Option<Arc<ExpressionSheets>>,
    image: impl FnMut(&str) -> Option<(egui::TextureId, Vec2)>,
) -> LiveResult {
    draw_live_frame(c, id, canvas, area, theme, options, sheets, None, image)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_live_frame(
    c: &Card<'_>,
    id: &str,
    canvas: &Arc<Canvas>,
    area: Rect,
    theme: CanvasTheme,
    options: LiveOptions,
    sheets: Option<Arc<ExpressionSheets>>,
    pinned_elapsed: Option<Duration>,
    image: impl FnMut(&str) -> Option<(egui::TextureId, Vec2)>,
) -> LiveResult {
    let input = Input {
        c,
        area,
        theme,
        options,
        id: c.ui.make_persistent_id(("canvas-live", id)),
    };
    let ctx = c.ui.ctx();
    let cache_id = egui::Id::new("plugin-live-memory");
    let mut entries = ctx.data_mut(|data| {
        std::mem::take(data.get_temp_mut_or_default::<VecDeque<(NodeKey, LiveState)>>(cache_id))
    });
    let old = entries
        .iter()
        .position(|(key, _)| *key == input.id)
        .and_then(|i| entries.remove(i));
    let mut state = old.map_or_else(LiveState::default, |(_, state)| state);
    let visible = c.live && area.intersect(c.painter.clip_rect()).is_positive();
    let mut result = LiveResult::default();
    let time = c.ui.input(|state| state.time);
    let pet = Arc::clone(&state.pet);
    let mut runner = pet.lock().unwrap_or_else(|poison| poison.into_inner());
    let mut pets = DrawPets {
        runner: &mut runner,
        time,
        live: visible,
        sheets,
        next: None,
    };
    if visible {
        let (evaluated, next) = canvas.evaluate(options.now, c.s * ctx.pixels_per_point());
        result.repaint_after = next;
        let evaluated = match evaluated {
            Cow::Borrowed(_) => Arc::clone(canvas),
            Cow::Owned(canvas) => Arc::new(canvas),
        };
        if state.revision != Some(options.revision) {
            state.before = state
                .transition_frame
                .take()
                .or_else(|| state.current.clone())
                .map(|old| (time, old));
            state.revision = Some(options.revision);
        }
        state.current = Some(Arc::clone(&evaluated));
        let mut prepared = evaluated;
        if let Some((start, old)) = &state.before {
            let mut active = false;
            transition::advance(
                &mut Arc::make_mut(&mut prepared).nodes,
                &old.nodes,
                time - *start,
                theme,
                &mut active,
            );
            state.transition_frame = active.then(|| Arc::clone(&prepared));
            result.animating |= active;
            if !active {
                state.before = None;
            }
        }
        let mut looping = false;
        let frame_needs_work = needs_work(&prepared.nodes, options.interactive);
        let tip = if frame_needs_work {
            let nodes = &mut Arc::make_mut(&mut prepared).nodes;
            if let Some(elapsed) = pinned_elapsed {
                looping = pinned_text_flow(c, nodes, Transform::new(c.s), area.width(), elapsed);
            }
            state.effects(
                c,
                nodes,
                input.id,
                time,
                &mut result.animating,
                &mut looping,
            );
            let tip = state.input.apply(&input, nodes, time, &mut result);
            state.roll(c, nodes, input.id, c.s, time, &mut result.animating);
            tip
        } else {
            None
        };
        state.visible_frame = Some(Arc::clone(&prepared));
        result.timed_out = draw_static(c, id, &prepared, area, theme, &mut pets, image);
        if !result.timed_out {
            if let Some(tip) = tip {
                tip.draw(&input);
            }
        } else {
            result.hit = None;
        }
        if result.animating {
            result.schedule(Duration::from_nanos(16_666_667));
        } else if looping {
            result.schedule(Duration::from_nanos(33_333_334));
        }
        result.animating |= looping;
        if let Some(delay) = pets.next {
            result.schedule(delay);
            result.animating |= delay <= Duration::from_nanos(33_333_334);
        }
    } else {
        result.timed_out = draw_static(
            c,
            id,
            state.visible_frame.as_ref().unwrap_or(canvas),
            area,
            theme,
            &mut pets,
            image,
        );
    }
    entries.push_front((input.id, state));
    entries.truncate(CACHE_LIMIT);
    ctx.data_mut(|data| data.insert_temp(cache_id, entries));
    result
}

fn needs_work(nodes: &[Node], interactive: bool) -> bool {
    nodes.iter().any(|node| {
        node.looping.is_some()
            || node.burst.is_some()
            || node.roll
            || interactive
                && (node.hit.is_some()
                    || node.hover.is_some()
                    || node.scroll.is_some()
                    || !node.tips.is_empty())
            || matches!(&node.kind, Kind::Group { group, .. } if needs_work(group, interactive))
    })
}

impl LiveState {
    fn effects(
        &mut self,
        c: &Card<'_>,
        nodes: &mut Vec<Node>,
        parent: NodeKey,
        time: f64,
        animating: &mut bool,
        looping: &mut bool,
    ) {
        let mut confetti = Vec::new();
        for (index, node) in nodes.iter_mut().enumerate() {
            let key = hit::key(parent, index, node);
            if let Kind::Group { group, .. } = &mut node.kind {
                self.effects(c, group, key, time, animating, looping);
            }
            if node.style.opacity.initial() <= 0.0 {
                continue;
            }
            if let Some(Burst::Confetti) = &node.burst {
                let start = self.bursts.entry(key.value()).or_insert(time);
                let elapsed = time - *start;
                if (0.0..0.65).contains(&elapsed) {
                    let bounds = hit::bounds(c, node, Transform::new(1.0));
                    if bounds.is_positive() {
                        *animating = true;
                        confetti.extend(particles(
                            bounds.center(),
                            (elapsed / 0.65) as f32,
                            key.value(),
                        ));
                    }
                }
            }
            if let Some(effect) = node.looping.as_ref() {
                *looping = true;
                let period = f64::from(effect.period_ms) / 1000.0;
                // 循环相位按 30 Hz 取样，短过渡仍可按 60 Hz 绘制。
                let phase = (((time * 30.0).floor() / 30.0).rem_euclid(period) / period) as f32;
                match effect.kind {
                    LoopKind::Pulse => {
                        node.style.opacity =
                            num(node.style.opacity.initial() * (0.8 + 0.2 * (phase * TAU).cos()));
                    }
                    LoopKind::Bob => {
                        *node = group(
                            vec![node.clone()],
                            [0.0, -2.0 * (phase * TAU).sin()],
                            0.0,
                            None,
                        )
                    }
                    LoopKind::Spin => {
                        let bounds = hit::bounds(c, node, Transform::new(1.0));
                        if !bounds.is_positive() {
                            continue;
                        }
                        let center = bounds.center();
                        let angle = phase * TAU;
                        let rotated = egui::emath::Rot2::from_angle(angle) * center.to_vec2();
                        let translate = center.to_vec2() - rotated;
                        *node = group(
                            vec![node.clone()],
                            [translate.x, translate.y],
                            angle.to_degrees(),
                            None,
                        );
                    }
                    LoopKind::Flow => {
                        flow(node, phase);
                    }
                }
            }
        }
        nodes.extend(confetti);
    }

    fn roll(
        &mut self,
        c: &Card<'_>,
        nodes: &mut [Node],
        parent: NodeKey,
        scale: f32,
        time: f64,
        animating: &mut bool,
    ) {
        for (index, node) in nodes.iter_mut().enumerate() {
            let key = hit::key(parent, index, node);
            if let Kind::Group {
                group,
                scale: child_scale,
                ..
            } = &mut node.kind
            {
                self.roll(
                    c,
                    group,
                    key,
                    scale * child_scale.initial(),
                    time,
                    animating,
                );
            }
            if !node.roll
                || node.style.opacity.initial() <= 0.0
                || !(0.0 < scale && scale <= 1024.0)
            {
                continue;
            }
            let Kind::Text { text, .. } = &node.kind else {
                continue;
            };
            let state = self.rolls.entry(key.value()).or_insert_with(|| Roll {
                before: text.clone(),
                text: text.clone(),
                at: time,
            });
            if state.text != *text {
                state.before = std::mem::replace(&mut state.text, text.clone());
                state.at = time;
            }
            let elapsed = time - state.at;
            if state.before != state.text && (0.0..0.22).contains(&elapsed) {
                if let Some(rolled) =
                    roll_digits(c, node, &state.before, scale, (elapsed / 0.22) as f32)
                {
                    *node = rolled;
                    *animating = true;
                }
            }
        }
    }
}

fn pinned_text_flow(
    c: &Card<'_>,
    nodes: &mut [Node],
    transform: Transform,
    right: f32,
    elapsed: Duration,
) -> bool {
    if !(0.0 < transform.scale && transform.scale <= 1024.0) {
        return false;
    }
    let mut scrolling = false;
    for node in nodes {
        if let Kind::Group { group, clip, translate, rotate, scale } = &mut node.kind {
            let child = transform.child(translate, rotate.initial(), scale.initial());
            let right = clip.as_ref().map_or(right, |clip| right.min(child.bounds(clip).right()));
            scrolling |= pinned_text_flow(c, group, child, right, elapsed);
            continue;
        }
        let Some(effect) = node.looping.as_ref().filter(|effect| effect.kind == LoopKind::Flow) else {
            continue;
        };
        let period = f64::from(effect.period_ms) / 1000.0;
        let Kind::Text { text, at, size, weight, align, max_width, font, .. } = &mut node.kind else {
            continue;
        };
        let Some(window) = max_width.as_ref().map(Num::initial) else {
            continue;
        };
        let [x, y] = at.each_ref().map(Num::initial);
        let size = size.initial();
        let anchor = match align {
            Align::Left => x,
            Align::Center => x - window / 2.0,
            Align::Right => x - window,
        };
        let window = if transform.angle == 0.0 {
            window.min((right - transform.xy(anchor, y).x) / transform.scale)
        } else {
            window
        };
        node.looping = None;
        if window <= 0.0 {
            continue;
        }
        let max_font_size = 256.0 / c.ui.ctx().pixels_per_point();
        let text_width = c.ui.fonts(|fonts| {
            let job = super::text_job(
                text, (size * transform.scale).min(max_font_size),
                *weight, &Align::Left, None, 1, font.as_deref(), &fonts.families(),
            );
            fonts.layout_job(job).size().x / transform.scale
        });
        let overflow = text_width - window;
        if overflow <= 0.5 {
            continue;
        }
        let dwell = 0.65;
        let travel = (period - dwell * 2.0).max(f64::from(overflow) / 48.0);
        let phase = elapsed.as_secs_f64().rem_euclid(travel + dwell * 2.0);
        let moved = overflow * ((phase - dwell) / travel).clamp(0.0, 1.0) as f32;
        *align = Align::Left;
        at[0] = num(anchor - moved);
        *max_width = None;
        *node = group(vec![node.clone()], [0.0, 0.0], 0.0,
            Some([anchor, y - size * 1.5, window, size * 2.0].map(num)));
        scrolling = true;
    }
    scrolling
}

fn flow(node: &mut Node, phase: f32) {
    if let Kind::Text {
        at,
        max_width: Some(width),
        ..
    } = &mut node.kind
    {
        at[0] = num(at[0].initial() + width.initial() * (1.0 - 2.0 * phase));
        return;
    }
    if let Kind::Group { group, .. } = &mut node.kind {
        for child in group {
            flow(child, phase);
        }
    }
    let Kind::Rect { rect, .. } = &node.kind else {
        return;
    };
    let Some(Paint::Solid(color)) = &node.style.fill else {
        return;
    };
    let [x, y, w, h] = rect.each_ref().map(Num::initial);
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let length = w + h;
    let bands = (length / 14.0).ceil().clamp(1.0, 12.0);
    let hi = color.clone();
    let low = Color::Rgba([255, 255, 255, 155]);
    let mut stops = vec![(0.0, hi.clone())];
    for i in -1..=bands as i32 {
        let base = (i as f32 + phase) / bands;
        for (offset, ink) in [(0.0, &hi), (0.35, &hi), (0.5, &low), (0.63, &hi)] {
            let position = base + offset / bands;
            if position > 0.0 && position < 1.0 {
                stops.push((position, ink.clone()));
            }
        }
    }
    stops.push((1.0, hi));
    let from = [num(x), num(y + h)];
    let to = [num(x + length * 0.5), num(y + h - length * 0.5)];
    node.style.fill = Some(Paint::Linear { from, to, stops });
}

fn roll_digits(c: &Card<'_>, node: &Node, before: &str, scale: f32, t: f32) -> Option<Node> {
    let Kind::Text {
        text,
        at,
        size,
        weight,
        align,
        max_width,
        lines,
        font,
    } = &node.kind
    else {
        return None;
    };
    let mut changes = false;
    let fonts = c.ui.fonts(|fonts| fonts.families());
    let job = super::text_job(
        text,
        (size.initial() * scale).min(256.0 / c.ui.ctx().pixels_per_point()),
        *weight,
        align,
        max_width.as_ref().map(|w| w.initial() * scale),
        *lines,
        font.as_deref(),
        &fonts,
    );
    let galley = c.ui.fonts(|fonts| fonts.layout_job(job));
    let first = galley.rows.first()?.glyphs.first()?.pos.y;
    let [x, y] = at.each_ref().map(Num::initial);
    let mut cells = Vec::new();
    let previous: Vec<_> = before.chars().rev().filter(|ch| !ch.is_control()).collect();
    let count = galley
        .rows
        .iter()
        .map(|row| row.glyphs.len())
        .sum::<usize>();
    let mut remaining = count;
    for row in &galley.rows {
        for glyph in &row.glyphs {
            remaining -= 1;
            let char_now = glyph.chr;
            let char_before = previous.get(remaining).copied();
            let xpos = x + glyph.pos.x / scale;
            let ypos = y + (glyph.pos.y - first) / scale;
            let make = |ch: char, delta: f32| Node {
                kind: Kind::Text {
                    text: ch.to_string(),
                    at: [num(xpos), num(ypos + delta)],
                    size: size.clone(),
                    weight: *weight,
                    align: Align::Left,
                    max_width: None,
                    lines: 1,
                    font: font.clone(),
                },
                ..node.clone()
            };
            if char_now.is_ascii_digit() && Some(char_now) != char_before {
                changes = true;
                let height = glyph.font_height / scale;
                let ascent = glyph.font_ascent / scale;
                let t = t * t * (3.0 - 2.0 * t);
                let mut digits = Vec::new();
                if let Some(old) = char_before.filter(char::is_ascii_digit) {
                    digits.push(make(old, -height * t));
                }
                digits.push(make(char_now, height * (1.0 - t)));
                cells.push(group(
                    digits,
                    [0.0, 0.0],
                    0.0,
                    Some([
                        num(xpos),
                        num(ypos - ascent),
                        num(glyph.advance_width / scale),
                        num(height),
                    ]),
                ));
            } else {
                cells.push(make(char_now, 0.0));
            }
        }
    }
    changes.then(|| group(cells, [0.0, 0.0], 0.0, None))
}

fn particles(center: egui::Pos2, time: f32, seed: u64) -> Vec<Node> {
    const COLORS: [[u8; 3]; 5] = [
        [255, 201, 75],
        [114, 207, 255],
        [255, 110, 149],
        [146, 230, 128],
        [182, 151, 247],
    ];
    (0..14)
        .map(|i| {
            let delay = (i % 3) as f32 * 0.03;
            let t = (time - delay).max(0.0);
            let angle = (i as f32 * 2.399_963 + (seed & 255) as f32 * 0.1).rem_euclid(TAU);
            let distance = 28.0 + (i % 6) as f32 * 7.0;
            let point = center + Vec2::angled(angle) * distance * t + egui::vec2(0.0, 43.0 * t * t);
            let [r, g, b] = COLORS[i % COLORS.len()];
            let node = Node {
                kind: Kind::Rect {
                    rect: [num(-1.7), num(-2.4), num(3.4), num(4.8)],
                    r: [0.7; 4].map(num),
                },
                style: Style {
                    fill: Some(Paint::Solid(Color::Rgba([r, g, b, 255]))),
                    opacity: num((1.0 - t).min(1.0) * f32::from(u8::from(time >= delay))),
                    ..empty_style()
                },
                ..group(Vec::new(), [0.0, 0.0], 0.0, None)
            };
            group(
                vec![node],
                [point.x, point.y],
                (time * 365.0 + i as f32 * 48.0) % 360.0,
                None,
            )
        })
        .collect()
}

pub(super) fn num(value: f32) -> Num {
    Num::Literal(value)
}

fn empty_style() -> Style {
    Style {
        fill: None,
        stroke: None,
        width: num(1.0),
        cap: Default::default(),
        dash: None,
        opacity: num(1.0),
        shadow: None,
        glow: None,
    }
}

pub(super) fn group(
    children: Vec<Node>,
    translate: [f32; 2],
    angle: f32,
    clip: Option<[Num; 4]>,
) -> Node {
    Node {
        kind: Kind::Group {
            group: children,
            clip,
            translate: translate.map(num),
            rotate: num(angle),
            scale: num(1.0),
        },
        style: empty_style(),
        id: None,
        roll: false,
        animate: None,
        looping: None,
        burst: None,
        hit: None,
        hover: None,
        scroll: None,
        tips: Vec::new(),
    }
}

pub(super) fn mix_color(a: Color32, b: Color32, t: f32) -> Color {
    Color::Rgba(
        a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
            .to_srgba_unmultiplied(),
    )
}
