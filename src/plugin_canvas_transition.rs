//! 同一 id 的场景变更插值，离场节点继续用原有树位置淡出。

use super::{mix_color, num};
use crate::{
    plugin_canvas::{live::Ease, Color, Kind, Node, Num, Paint, PathCommand},
    plugin_canvas_ui::CanvasTheme,
};
use eframe::egui::Color32;
use std::collections::{HashMap, HashSet};

pub(super) fn advance(
    nodes: &mut Vec<Node>,
    previous: &[Node],
    elapsed: f64,
    theme: CanvasTheme,
    active: &mut bool,
) {
    let mut previous_ids = HashMap::new();
    index(previous, &mut previous_ids);
    let mut current = HashMap::new();
    index(nodes, &mut current);
    let current = current.keys().map(|id| (*id).to_owned()).collect();
    let transition = Transition {
        previous: previous_ids,
        current,
        elapsed,
        theme,
    };
    transition.nodes(nodes, previous, active);
}

fn index<'a>(nodes: &'a [Node], ids: &mut HashMap<&'a str, &'a Node>) {
    for node in nodes {
        if let Some(id) = node.id.as_deref() {
            ids.insert(id, node);
        }
        if let Kind::Group { group, .. } = &node.kind {
            index(group, ids);
        }
    }
}

struct Transition<'a> {
    previous: HashMap<&'a str, &'a Node>,
    current: HashSet<String>,
    elapsed: f64,
    theme: CanvasTheme,
}

impl Transition<'_> {
    fn nodes(&self, nodes: &mut Vec<Node>, previous_siblings: &[Node], active: &mut bool) {
        let mut matched = HashSet::new();
        for (i, node) in nodes.iter_mut().enumerate() {
            let previous = node
                .id
                .as_deref()
                .and_then(|id| self.previous.get(id).copied());
            if let Some(animation) = node.animate.as_ref().filter(|_| node.id.is_some()) {
                let progress = self.elapsed * 1000.0 / f64::from(animation.ms);
                if progress < 1.0 && previous.is_none_or(|previous| different(node, previous)) {
                    let eased = ease(animation.ease, progress as f32);
                    if let Some(previous) = previous {
                        blend(node, previous, eased, self.theme);
                    } else {
                        node.style.opacity = num(node.style.opacity.initial() * eased);
                    }
                    *active = true;
                }
            }
            if let Kind::Group { group, .. } = &mut node.kind {
                let old = previous.or_else(|| {
                    previous_siblings
                        .get(i)
                        .filter(|old| old.id.is_none() && node.id.is_none())
                });
                if let Some(old) = old {
                    matched.insert(std::ptr::from_ref(old));
                }
                let old = match old.map(|node| &node.kind) {
                    Some(Kind::Group { group, .. }) => group.as_slice(),
                    _ => &[],
                };
                self.nodes(group, old, active);
            }
        }
        for old in previous_siblings {
            if matched.contains(&std::ptr::from_ref(old)) {
                continue;
            }
            if let Some(retired) = self.retire(old, active) {
                nodes.push(retired);
            }
        }
    }

    fn retire(&self, old: &Node, active: &mut bool) -> Option<Node> {
        if old.id.as_ref().is_some_and(|id| self.current.contains(id)) {
            return None;
        }
        if let Some(animation) = old.animate.as_ref().filter(|_| old.id.is_some()) {
            let progress = self.elapsed * 1000.0 / f64::from(animation.ms);
            if progress < 1.0 {
                let mut node = old.clone();
                stop_interaction(&mut node);
                node.style.opacity =
                    num(node.style.opacity.initial()
                        * (1.0 - ease(animation.ease, progress as f32)));
                *active = true;
                return Some(node);
            }
        }
        if let Kind::Group { group, .. } = &old.kind {
            let remaining: Vec<_> = group
                .iter()
                .filter_map(|child| self.retire(child, active))
                .collect();
            if !remaining.is_empty() {
                let mut node = old.clone();
                if let Kind::Group { group, .. } = &mut node.kind {
                    *group = remaining;
                }
                stop_interaction(&mut node);
                return Some(node);
            }
        }
        None
    }
}

fn stop_interaction(node: &mut Node) {
    node.hit = None;
    node.hover = None;
    node.tips.clear();
    node.scroll = None;
    node.looping = None;
    node.burst = None;
    node.roll = false;
    if let Kind::Group { group, .. } = &mut node.kind {
        for child in group {
            stop_interaction(child);
        }
    }
}

fn different(next: &Node, previous: &Node) -> bool {
    next.style != previous.style
        || match (&next.kind, &previous.kind) {
            (
                Kind::Group {
                    clip: a,
                    translate: b,
                    rotate: c,
                    scale: d,
                    ..
                },
                Kind::Group {
                    clip: e,
                    translate: f,
                    rotate: g,
                    scale: h,
                    ..
                },
            ) => (a, b, c, d) != (e, f, g, h),
            (next, previous) => next != previous,
        }
}

fn ease(ease: Ease, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match ease {
        Ease::Linear => t,
        Ease::Spring => {
            let spring = |x: f32| 1.0 - (-7.0 * x).exp() * (1.0 + 7.0 * x);
            spring(t) / spring(1.0)
        }
    }
}

fn blend(next: &mut Node, old: &Node, t: f32, theme: CanvasTheme) {
    let number = |next: &mut Num, old: &Num| {
        *next = num(old.initial() + (next.initial() - old.initial()) * t);
    };
    let rotate = |next: &mut Num, old: &Num| {
        let delta = (next.initial() - old.initial() + 180.0).rem_euclid(360.0) - 180.0;
        *next = num(old.initial() + delta * t);
    };
    let paint = |next: &mut Option<Paint>, old: &Option<Paint>| blend_paint(next, old, t, theme);

    number(&mut next.style.opacity, &old.style.opacity);
    number(&mut next.style.width, &old.style.width);
    paint(&mut next.style.fill, &old.style.fill);
    paint(&mut next.style.stroke, &old.style.stroke);

    match (&mut next.kind, &old.kind) {
        (
            Kind::Rect { rect, r },
            Kind::Rect {
                rect: from,
                r: radii,
            },
        ) => {
            numbers(rect, from, number);
            numbers(r, radii, number);
        }
        (Kind::Image { rect, .. }, Kind::Image { rect: from, .. })
        | (Kind::Line(rect), Kind::Line(from)) => numbers(rect, from, number),
        (Kind::Circle(rect), Kind::Circle(from)) => numbers(rect, from, number),
        (
            Kind::Pet { at, size, .. },
            Kind::Pet {
                at: from,
                size: old_size,
                ..
            },
        ) => {
            numbers(at, from, number);
            number(size, old_size);
        }
        (
            Kind::Arc { c, r, start, sweep },
            Kind::Arc {
                c: center,
                r: radius,
                start: first,
                sweep: angle,
            },
        ) => {
            numbers(c, center, number);
            number(r, radius);
            rotate(start, first);
            number(sweep, angle);
        }
        (Kind::Path(commands), Kind::Path(previous)) if commands.len() == previous.len() => {
            let point = |a: &mut [f32; 2], b: &[f32; 2]| {
                for (a, b) in a.iter_mut().zip(b) {
                    *a = *b + (*a - *b) * t;
                }
            };
            for (command, old) in commands.iter_mut().zip(previous) {
                match (command, old) {
                    (PathCommand::Move(a), PathCommand::Move(b))
                    | (PathCommand::Line(a), PathCommand::Line(b)) => point(a, b),
                    (PathCommand::Quad(a, b), PathCommand::Quad(c, d)) => {
                        point(a, c);
                        point(b, d);
                    }
                    (PathCommand::Cubic(a, b, c), PathCommand::Cubic(d, e, f)) => {
                        point(a, d);
                        point(b, e);
                        point(c, f);
                    }
                    _ => {}
                }
            }
        }
        (
            Kind::Polyline { points, area },
            Kind::Polyline {
                points: old_points,
                area: old_area,
            },
        ) => {
            if points.len() == old_points.len() {
                for (point, old) in points.iter_mut().zip(old_points) {
                    numbers(point, old, number);
                }
            }
            if let (Some(area), Some(old)) = (area, old_area) {
                number(area, old);
            }
        }
        (
            Kind::Text {
                at,
                size,
                max_width,
                ..
            },
            Kind::Text {
                at: from,
                size: old_size,
                max_width: old_width,
                ..
            },
        ) => {
            numbers(at, from, number);
            number(size, old_size);
            if let (Some(width), Some(old)) = (max_width, old_width) {
                number(width, old);
            }
        }
        (
            Kind::Group {
                translate,
                rotate: angle,
                scale,
                clip,
                ..
            },
            Kind::Group {
                translate: from,
                rotate: old_angle,
                scale: old_scale,
                clip: old_clip,
                ..
            },
        ) => {
            numbers(translate, from, number);
            rotate(angle, old_angle);
            number(scale, old_scale);
            if let (Some(rect), Some(old)) = (clip, old_clip) {
                numbers(rect, old, number);
            }
        }
        _ => {}
    }
}

fn numbers<const N: usize>(next: &mut [Num; N], old: &[Num; N], number: impl Fn(&mut Num, &Num)) {
    for (next, old) in next.iter_mut().zip(old) {
        number(next, old);
    }
}

fn blend_paint(next: &mut Option<Paint>, from: &Option<Paint>, t: f32, theme: CanvasTheme) {
    let color = |current: &Color, previous: &Color| {
        mix_color(theme.color(previous), theme.color(current), t)
    };
    let number = |next: &mut Num, old: &Num| {
        *next = num(old.initial() + (next.initial() - old.initial()) * t);
    };
    let blend_stops = |next: &mut [(f32, Color)], from: &[(f32, Color)]| {
        if next.len() == from.len() {
            for ((at, to), (from_at, from)) in next.iter_mut().zip(from) {
                *at = from_at + (*at - from_at) * t;
                *to = color(to, from);
            }
        }
    };
    match (next.as_mut(), from.as_ref()) {
        (Some(Paint::Solid(to)), Some(Paint::Solid(from))) => *to = color(to, from),
        (Some(Paint::Solid(to)), None) => *to = mix_color(Color32::TRANSPARENT, theme.color(to), t),
        (None, Some(Paint::Solid(from))) => {
            *next = Some(Paint::Solid(mix_color(
                theme.color(from),
                Color32::TRANSPARENT,
                t,
            )))
        }
        (
            Some(Paint::Linear { from, to, stops }),
            Some(Paint::Linear {
                from: start,
                to: end,
                stops: previous,
            }),
        ) => {
            numbers(from, start, number);
            numbers(to, end, number);
            blend_stops(stops, previous);
        }
        (
            Some(Paint::Radial {
                center,
                radius,
                stops,
            }),
            Some(Paint::Radial {
                center: from,
                radius: old_radius,
                stops: previous,
            }),
        ) => {
            numbers(center, from, number);
            number(radius, old_radius);
            blend_stops(stops, previous);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_canvas::Canvas;
    use serde_json::json;

    #[test]
    fn plugin_canvas_transition_fades_in_out_and_keeps_its_endpoints() {
        let canvas = |nodes| {
            Canvas::parse(json!({"height": 80, "nodes": nodes}))
                .unwrap()
                .nodes
        };
        let start = canvas(json!([
            {"id":"move","rect":[0,0,40,20],"fill":"#FF0000","animate":{"ms":400}},
            {"id":"hide","circle":[40,40,10],"fill":"#FFFFFF","animate":{"ms":400}}
        ]));
        let end = canvas(json!([
            {"id":"move","rect":[20,0,20,20],"fill":"#0000FF","animate":{"ms":400}},
            {"id":"show","circle":[20,40,10],"fill":"#FFFFFF","animate":{"ms":400}}
        ]));
        let theme = CanvasTheme {
            palette: None,
            accent: None,
            background: Color32::BLACK,
        };
        let mut mid = end.clone();
        let mut active = false;
        advance(&mut mid, &start, 0.2, theme, &mut active);
        assert!(active);
        let Kind::Rect { rect, .. } = &mid[0].kind else {
            panic!("缺少矩形")
        };
        assert_eq!(rect[0].initial(), 10.0);
        assert_eq!(rect[2].initial(), 30.0);
        assert_eq!(mid[1].style.opacity.initial(), 0.5);
        assert_eq!(mid[2].style.opacity.initial(), 0.5);
        let mut done = end.clone();
        let mut active = false;
        advance(&mut done, &start, 0.4, theme, &mut active);
        assert_eq!(done, end);
        assert!(!active);
    }
}
