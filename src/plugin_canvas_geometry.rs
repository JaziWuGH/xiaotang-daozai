//! 画布图形细分与渐变着色。

use crate::plugin_canvas::{Bounds, Cap, Num, PathCommand, Point};
use eframe::{
    egui::{emath::Rot2, pos2, vec2, Color32, Pos2, Rect, Vec2},
    epaint::{
        tessellator::path::rounded_rectangle, CornerRadiusF32, CubicBezierShape, Mesh, PathShape,
        QuadraticBezierShape, Stroke, Tessellator, Vertex,
    },
};
use std::{f32::consts::PI, time::Instant};

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Transform {
    origin: Pos2,
    pub angle: f32,
    pub scale: f32,
}

impl Transform {
    pub fn new(scale: f32) -> Self {
        Self {
            origin: Pos2::ZERO,
            angle: 0.0,
            scale,
        }
    }

    pub fn at(scale: f32, origin: Pos2) -> Self {
        Self {
            origin,
            ..Self::new(scale)
        }
    }

    pub fn child(self, translate: &Point, angle: f32, scale: f32) -> Self {
        Self {
            origin: self.point(translate),
            angle: self.angle + angle.to_radians(),
            scale: self.scale * scale,
        }
    }

    pub fn pos(self, point: Pos2) -> Pos2 {
        self.origin + Rot2::from_angle(self.angle) * (point.to_vec2() * self.scale)
    }

    pub fn xy(self, x: f32, y: f32) -> Pos2 {
        self.pos(pos2(x, y))
    }

    pub fn point(self, [x, y]: &Point) -> Pos2 {
        self.xy(x.initial(), y.initial())
    }

    pub fn local(self, point: Pos2) -> Pos2 {
        let rotation = Rot2::from_angle(-self.angle);
        (rotation * (point - self.origin) / self.scale).to_pos2()
    }

    pub fn bounds(self, bounds: &Bounds) -> Rect {
        (rect(bounds).rotate_bb(Rot2::from_angle(self.angle)) * self.scale)
            .translate(self.origin.to_vec2())
    }
}

pub(super) fn rect(bounds: &Bounds) -> Rect {
    let [x, y, width, height] = bounds.each_ref().map(Num::initial);
    Rect::from_min_size(pos2(x, y), vec2(width, height))
}

#[derive(Clone, PartialEq)]
pub(super) struct Clip {
    pub rect: Rect,
    planes: Vec<(Pos2, Vec2)>,
}

impl Clip {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            planes: Vec::new(),
        }
    }

    pub fn child(&self, bounds: &Bounds, transform: Transform) -> Self {
        let mut clip = self.clone();
        clip.rect = clip.rect.intersect(transform.bounds(bounds));
        if (transform.angle / (PI * 0.5) - (transform.angle / (PI * 0.5)).round()).abs() > 0.00001 {
            let rect = rect(bounds);
            let points = [
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
            ]
            .map(|point| transform.pos(point));
            for i in 0..4 {
                let delta = (points[(i + 1) % 4] - points[i]).normalized();
                clip.planes.push((points[i], vec2(-delta.y, delta.x)));
            }
        }
        clip
    }

    pub fn horizontal(&mut self, pos: Pos2, rotation: Rot2, min: f32, max: f32) {
        let direction = rotation * vec2(1.0, 0.0);
        self.planes.push((pos + direction * min, direction));
        self.planes.push((pos + direction * max, -direction));
    }

    pub fn has_planes(&self) -> bool {
        !self.planes.is_empty()
    }

    pub fn contains(&self, point: Pos2) -> bool {
        self.rect.contains(point)
            && self
                .planes
                .iter()
                .all(|&(at, inward)| (point - at).dot(inward) >= 0.0)
    }

    /// 保留裁剪后顶点的纹理坐标、颜色和抗锯齿透明度。
    pub fn mesh(
        &self,
        mesh: &mut Mesh,
        to_canvas: impl Fn(Pos2) -> Pos2,
        deadline: Option<Instant>,
    ) -> Option<()> {
        if mesh.is_empty() || self.planes.is_empty() {
            return Some(());
        }
        let bounds = mesh.calc_bounds();
        let corners = [
            bounds.left_top(),
            bounds.right_top(),
            bounds.right_bottom(),
            bounds.left_bottom(),
        ]
        .map(&to_canvas);
        let distance = |p: Pos2, plane: (Pos2, Vec2)| (p - plane.0).dot(plane.1);
        if self
            .planes
            .iter()
            .all(|&plane| corners.iter().all(|&p| distance(p, plane) >= 0.0))
        {
            return Some(());
        }
        let mut output = Mesh::with_texture(mesh.texture_id);
        for indices in mesh.indices.chunks_exact(3) {
            if deadline.is_some_and(|at| Instant::now() >= at) {
                return None;
            }
            let mut polygon = indices
                .iter()
                .map(|&i| mesh.vertices[i as usize])
                .collect::<Vec<_>>();
            for &plane in &self.planes {
                polygon = clip_polygon(
                    &polygon,
                    |v| distance(to_canvas(v.pos), plane),
                    |a, b, t| Vertex {
                        pos: a.pos.lerp(b.pos, t),
                        uv: a.uv.lerp(b.uv, t),
                        color: mix(a.color, b.color, t),
                    },
                );
                if polygon.len() < 3 {
                    break;
                }
            }
            if polygon.len() >= 3 {
                let base = output.vertices.len() as u32;
                for i in 1..polygon.len() as u32 - 1 {
                    output.add_triangle(base, base + i, base + i + 1);
                }
                output.vertices.extend(polygon);
            }
        }
        *mesh = output;
        Some(())
    }
}

pub(super) struct Outline {
    pub points: Vec<Pos2>,
    pub closed: bool,
}

impl Outline {
    pub fn open(points: Vec<Pos2>) -> Self {
        Self {
            points,
            closed: false,
        }
    }

    pub fn stroke(
        &self,
        tess: &mut Tessellator,
        target: &mut Mesh,
        pigment: &Pigment,
        width: f32,
        cap: &Cap,
        dash: Option<[f32; 2]>,
        deadline: Option<Instant>,
    ) -> Option<()> {
        if width <= 0.0 {
            return Some(());
        }
        let mut source = Mesh::default();
        let mut paint = |points: Vec<Pos2>, closed| {
            if points.len() < 2 {
                return;
            }
            let stroke = Stroke::new(width, Color32::WHITE);
            if points.len() == 2 {
                tess.tessellate_line_segment([points[0], points[1]], stroke, &mut source);
            } else {
                let shape = if closed {
                    PathShape::closed_line(points.clone(), stroke)
                } else {
                    PathShape::line(points.clone(), stroke)
                };
                tess.tessellate_path(&shape, &mut source);
            }
            if !closed && matches!(cap, Cap::Round) {
                for (point, adjacent) in [
                    (points[0], points[1]),
                    (points[points.len() - 1], points[points.len() - 2]),
                ] {
                    let outward = point - adjacent;
                    let half = arc(
                        point,
                        width / 2.0,
                        outward.angle().to_degrees() - 90.0,
                        180.0,
                        2.0,
                    );
                    let shape =
                        PathShape::convex_polygon(half.points, Color32::WHITE, Stroke::NONE);
                    tess.tessellate_path(&shape, &mut source);
                }
            }
        };
        if let Some([length, space]) = dash.filter(|[length, space]| *length > 0.0 && *space > 0.0)
        {
            let (length, space) = (length.max(0.25), space.max(0.25));
            let mut on = true;
            let mut remaining = length;
            let mut segment = Vec::new();
            let closing = self
                .closed
                .then(|| self.points.last().zip(self.points.first()))
                .flatten();
            let pairs = self
                .points
                .windows(2)
                .map(|points| (&points[0], &points[1]))
                .chain(closing);
            for (&a, &b) in pairs {
                let delta = b - a;
                let distance = delta.length();
                let mut cursor = 0.0;
                while cursor < distance {
                    if deadline.is_some_and(|at| Instant::now() >= at) {
                        return None;
                    }
                    let step = (distance - cursor).min(remaining);
                    let next = cursor + step;
                    if next <= cursor {
                        return None;
                    }
                    if on {
                        if segment.is_empty() {
                            segment.push(a + (cursor / distance) * delta);
                        }
                        segment.push(a + (next / distance) * delta);
                    }
                    cursor = next;
                    remaining -= step;
                    if remaining <= f32::EPSILON {
                        if on {
                            paint(std::mem::take(&mut segment), false);
                        }
                        on = !on;
                        remaining = if on { length } else { space };
                    }
                }
            }
            paint(segment, false);
        } else {
            paint(self.points.clone(), self.closed);
        }
        pigment.shade(source, target, deadline)
    }
}

pub(super) fn rectangle(bounds: &Bounds, corners: &[Num; 4], transform: Transform) -> Outline {
    let rect = rect(bounds);
    let [nw, ne, se, sw] = corners.each_ref().map(Num::initial);
    let mut points = Vec::new();
    if rect.is_positive() {
        // 在最终缩放下细分圆角，然后施加旋转。
        rounded_rectangle(
            &mut points,
            rect * transform.scale,
            CornerRadiusF32 {
                nw: nw * transform.scale,
                ne: ne * transform.scale,
                se: se * transform.scale,
                sw: sw * transform.scale,
            },
        );
        let rotation = Rot2::from_angle(transform.angle);
        for point in &mut points {
            *point = transform.origin + rotation * point.to_vec2();
        }
    }
    Outline {
        points,
        closed: true,
    }
}

pub(super) fn arc(center: Pos2, radius: f32, start: f32, sweep: f32, pixels: f32) -> Outline {
    let closed = sweep.abs() >= 360.0;
    if radius <= 0.0 || sweep == 0.0 {
        return Outline {
            points: Vec::new(),
            closed,
        };
    }
    let sweep = sweep.clamp(-360.0, 360.0).to_radians();
    let start = start.to_radians();
    let n = (sweep.abs() * (radius * pixels).sqrt())
        .ceil()
        .clamp(3.0, 512.0) as usize;
    let points = (0..n + usize::from(!closed))
        .map(|i| center + Vec2::angled(start + sweep * i as f32 / n as f32) * radius)
        .collect();
    Outline { points, closed }
}

pub(super) fn path(
    commands: &[PathCommand],
    transform: Transform,
    pixels: f32,
    deadline: Option<Instant>,
) -> Option<Vec<Outline>> {
    let mut outlines = Vec::new();
    let mut points = Vec::new();
    let mut current = Pos2::ZERO;
    let mut subpath = Pos2::ZERO;
    let tolerance = Some(0.35 / pixels);
    let point = |p: [f32; 2]| transform.xy(p[0], p[1]);
    for command in commands {
        if deadline.is_some_and(|at| Instant::now() >= at) {
            return None;
        }
        match command {
            PathCommand::Move(p) => {
                if !points.is_empty() {
                    outlines.push(Outline::open(std::mem::take(&mut points)));
                }
                current = point(*p);
                subpath = current;
                points.push(current);
            }
            PathCommand::Close => {
                if points.last() == points.first() {
                    points.pop();
                }
                outlines.push(Outline {
                    points: std::mem::take(&mut points),
                    closed: true,
                });
                current = subpath;
            }
            _ => {
                if points.is_empty() {
                    points.push(current);
                }
                match command {
                    PathCommand::Line(p) => points.push(point(*p)),
                    PathCommand::Quad(control, to) => {
                        let curve = QuadraticBezierShape::from_points_stroke(
                            [current, point(*control), point(*to)],
                            false,
                            Color32::TRANSPARENT,
                            Stroke::NONE,
                        );
                        points.extend(curve.flatten(tolerance).into_iter().skip(1));
                    }
                    PathCommand::Cubic(first, second, to) => {
                        let curve = CubicBezierShape::from_points_stroke(
                            [current, point(*first), point(*second), point(*to)],
                            false,
                            Color32::TRANSPARENT,
                            Stroke::NONE,
                        );
                        points.extend(curve.flatten(tolerance).into_iter().skip(1));
                    }
                    _ => unreachable!(),
                }
                current = *points.last()?;
            }
        }
    }
    if !points.is_empty() {
        outlines.push(Outline::open(points));
    }
    for outline in &mut outlines {
        outline.points.dedup();
    }
    Some(outlines)
}

pub(super) fn fill(
    tess: &mut Tessellator,
    target: &mut Mesh,
    contours: &[Outline],
    pigment: &Pigment,
    deadline: Option<Instant>,
) -> Option<()> {
    let contours: Vec<_> = contours
        .iter()
        .filter(|outline| outline.points.len() >= 3)
        .collect();
    let mut source = Mesh::default();
    if let [outline] = contours.as_slice() {
        if convex(&outline.points) {
            let shape =
                PathShape::convex_polygon(outline.points.clone(), Color32::WHITE, Stroke::NONE);
            tess.tessellate_path(&shape, &mut source);
            return pigment.shade(source, target, deadline);
        }
    }
    scan_fill(&mut source, &contours, deadline)?;
    pigment.shade(source, target, deadline)
}

fn convex(points: &[Pos2]) -> bool {
    let n = points.len();
    let mut negative = false;
    let mut positive = false;
    for i in 0..n {
        let a = points[(i + 1) % n] - points[i];
        let b = points[(i + 2) % n] - points[(i + 1) % n];
        let turn = a.x * b.y - a.y * b.x;
        negative |= turn < -f32::EPSILON;
        positive |= turn > f32::EPSILON;
        if negative && positive {
            return false;
        }
    }
    negative || positive
}

/// 用相邻顶点的水平分带填充凹形和带孔轮廓。绕向抵消时留下孔洞。
fn scan_fill(mesh: &mut Mesh, contours: &[&Outline], deadline: Option<Instant>) -> Option<()> {
    let mut ys = Vec::new();
    let mut edges = Vec::new();
    for outline in contours {
        let iter = outline.points.iter().copied();
        for (a, b) in iter.clone().zip(iter.cycle().skip(1)) {
            ys.push(a.y);
            if a.y < b.y {
                edges.push((a, b, 1));
            } else if a.y > b.y {
                edges.push((b, a, -1));
            }
        }
    }
    ys.sort_by(f32::total_cmp);
    ys.dedup();
    let mut crossings = Vec::new();
    for band in ys.windows(2) {
        if deadline.is_some_and(|at| Instant::now() >= at) {
            return None;
        }
        let [top, bottom] = [band[0], band[1]];
        let mid = (top + bottom) / 2.0;
        crossings.clear();
        crossings.extend(edges.iter().filter_map(|&(a, b, direction)| {
            if mid < a.y || mid >= b.y {
                return None;
            }
            let dx = (b.x - a.x) / (b.y - a.y);
            let x = |y| a.x + dx * (y - a.y);
            Some((x(mid), x(top), x(bottom), direction))
        }));
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0;
        let mut left = (0.0, 0.0);
        for &(_, x0, x1, direction) in &crossings {
            if winding == 0 {
                left = (x0, x1);
            }
            winding += direction;
            if winding == 0 {
                let index = mesh.vertices.len() as u32;
                for point in [
                    pos2(left.0, top),
                    pos2(x0, top),
                    pos2(x1, bottom),
                    pos2(left.1, bottom),
                ] {
                    mesh.colored_vertex(point, Color32::WHITE);
                }
                mesh.add_triangle(index, index + 1, index + 2);
                mesh.add_triangle(index, index + 2, index + 3);
            }
        }
    }
    Some(())
}

#[derive(Clone)]
pub(super) enum Pigment {
    Solid(Color32),
    Linear {
        from: Pos2,
        projected: Vec2,
        stops: Vec<(f32, Color32)>,
    },
    Radial {
        center: Pos2,
        radius: f32,
        stops: Vec<(f32, Color32)>,
    },
}

fn mix(a: Color32, b: Color32, fraction: f32) -> Color32 {
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * fraction).round() as u8;
    Color32::from_rgba_premultiplied(
        lerp(a.r(), b.r()),
        lerp(a.g(), b.g()),
        lerp(a.b(), b.b()),
        lerp(a.a(), b.a()),
    )
}

impl Pigment {
    fn stop_color(stops: &[(f32, Color32)], t: f32) -> Color32 {
        let index = stops.partition_point(|(offset, _)| *offset <= t);
        match (
            index.checked_sub(1).and_then(|i| stops.get(i)),
            stops.get(index),
        ) {
            (Some(&(left, a)), Some(&(right, b))) => mix(a, b, (t - left) / (right - left)),
            (Some(&(_, color)), None) | (None, Some(&(_, color))) => color,
            (None, None) => Color32::TRANSPARENT,
        }
    }

    pub fn color_at(&self, pos: Pos2) -> Color32 {
        match self {
            Self::Solid(color) => *color,
            Self::Linear {
                from,
                projected,
                stops,
            } => Self::stop_color(stops, (pos - *from).dot(*projected)),
            Self::Radial {
                center,
                radius,
                stops,
            } => Self::stop_color(stops, pos.distance(*center) / *radius),
        }
    }

    pub fn multiply(&mut self, factor: f32) {
        match self {
            Self::Solid(color) => *color = color.gamma_multiply(factor),
            Self::Linear { stops, .. } | Self::Radial { stops, .. } => {
                for (_, color) in stops {
                    *color = color.gamma_multiply(factor);
                }
            }
        }
    }

    fn shade(&self, mut source: Mesh, target: &mut Mesh, deadline: Option<Instant>) -> Option<()> {
        if let Self::Solid(color) = self {
            for vertex in &mut source.vertices {
                vertex.color = color.gamma_multiply(f32::from(vertex.color.a()) / 255.0);
            }
            target.append(source);
            return Some(());
        }
        for idx in source.indices.chunks_exact(3) {
            let triangle = [idx[0], idx[1], idx[2]].map(|i| {
                let vertex = source.vertices[i as usize];
                (vertex.pos, f32::from(vertex.color.a()) / 255.0)
            });
            if deadline.is_some_and(|at| Instant::now() >= at) {
                return None;
            }
            match self {
                Self::Linear {
                    from,
                    projected,
                    stops,
                } => {
                    let t = |p: Pos2| (p - *from).dot(*projected);
                    let [a, b, c] = triangle.map(|(p, _)| t(p));
                    let low = a.min(b).min(c);
                    let high = a.max(b).max(c);
                    let offsets = stops
                        .iter()
                        .map(|(k, _)| *k)
                        .filter(|k| *k > low && *k < high);
                    let mut polygon = triangle.to_vec();
                    for offset in offsets {
                        let side = |p| t(p) - offset;
                        let first = cut(&polygon, side, false);
                        self.paint_polygon(target, &first);
                        polygon = cut(&polygon, side, true);
                    }
                    self.paint_polygon(target, &polygon);
                }
                Self::Radial { center, radius, .. } => {
                    self.radial_triangle(target, triangle, *center, *radius, 0, deadline)?;
                }
                Self::Solid(_) => unreachable!(),
            }
        }
        Some(())
    }

    fn paint_polygon(&self, target: &mut Mesh, polygon: &[(Pos2, f32)]) {
        if polygon.len() < 3 {
            return;
        }
        let base = target.vertices.len() as u32;
        for &(pos, coverage) in polygon {
            target.colored_vertex(
                pos,
                self.color_at(pos).gamma_multiply(coverage.clamp(0.0, 1.0)),
            );
        }
        for i in 1..polygon.len() as u32 - 1 {
            target.add_triangle(base, base + i, base + i + 1);
        }
    }

    fn radial_triangle(
        &self,
        target: &mut Mesh,
        triangle: [(Pos2, f32); 3],
        center: Pos2,
        radius: f32,
        depth: u8,
        deadline: Option<Instant>,
    ) -> Option<()> {
        if deadline.is_some_and(|at| Instant::now() >= at) {
            return None;
        }
        let points = triangle.map(|(pos, _)| pos);
        let bounds = Rect::from_points(&points);
        if depth < 6
            && bounds.size().max_elem() > (radius * 0.35).max(3.0)
            && bounds.clamp(center).distance(center) < radius
        {
            let [a, b, c] = triangle;
            let mid = |(p, alpha): (Pos2, f32), (q, beta)| (p.lerp(q, 0.5), (alpha + beta) * 0.5);
            let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
            for next in [[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]] {
                self.radial_triangle(target, next, center, radius, depth + 1, deadline)?;
            }
        } else {
            self.paint_polygon(target, &triangle);
        }
        Some(())
    }
}

fn cut(
    polygon: &[(Pos2, f32)],
    distance: impl Fn(Pos2) -> f32,
    positive: bool,
) -> Vec<(Pos2, f32)> {
    clip_polygon(
        polygon,
        |&(point, _)| {
            if positive {
                distance(point)
            } else {
                -distance(point)
            }
        },
        |(a, before), (b, after), t| (a.lerp(b, t), before + (after - before) * t),
    )
}

fn clip_polygon<T: Copy>(
    polygon: &[T],
    distance: impl Fn(&T) -> f32,
    lerp: impl Fn(T, T, f32) -> T,
) -> Vec<T> {
    let mut out = Vec::with_capacity(polygon.len() + 1);
    let Some(&last) = polygon.last() else {
        return out;
    };
    let mut previous = last;
    let mut a = distance(&previous);
    for &point in polygon {
        let b = distance(&point);
        let prev_inside = a >= 0.0;
        let inside = b >= 0.0;
        if prev_inside != inside {
            out.push(lerp(previous, point, a / (a - b)));
        }
        if inside {
            out.push(point);
        }
        previous = point;
        a = b;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_canvas_rotated_clip_preserves_uv_and_diamond_boundary() {
        let canvas = Clip::new(Rect::EVERYTHING);
        let transform =
            Transform::new(1.0).child(&[Num::Literal(20.0), Num::Literal(20.0)], 45.0, 1.0);
        let bounds = [-6.0, -6.0, 12.0, 12.0].map(Num::Literal);
        let diamond = canvas.child(&bounds, transform);
        let original = Rect::from_min_max(pos2(10.0, 10.0), pos2(30.0, 30.0));
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        let mut mesh = Mesh::default();
        mesh.add_rect_with_uv(original, uv, Color32::WHITE);
        diamond.mesh(&mut mesh, |p| p, None).unwrap();
        assert!(!mesh.is_empty());
        for vertex in &mesh.vertices {
            let delta = vertex.pos - pos2(20.0, 20.0);
            assert!(delta.x.abs() + delta.y.abs() <= 6.0 * 2.0_f32.sqrt() + 0.001);
            assert!((vertex.uv.x - (vertex.pos.x - 10.0) / 20.0).abs() < 0.001);
            assert!((vertex.uv.y - (vertex.pos.y - 10.0) / 20.0).abs() < 0.001);
        }
    }

    #[test]
    fn plugin_canvas_cut_intersects_edges_and_preserves_coverage() {
        let polygon = [
            (pos2(0.0, 0.0), 0.0),
            (pos2(2.0, 0.0), 1.0),
            (pos2(0.0, 2.0), 0.0),
        ];
        let clipped = cut(&polygon, |p| p.x - 1.0, true);
        assert_eq!(clipped.len(), 3);
        assert!(clipped.contains(&(pos2(1.0, 0.0), 0.5)));
        assert!(clipped.contains(&(pos2(1.0, 1.0), 0.5)));
    }

    #[test]
    fn plugin_canvas_path_fill_respects_contour_holes() {
        let outer = Outline {
            points: vec![
                pos2(0.0, 0.0),
                pos2(80.0, 0.0),
                pos2(80.0, 80.0),
                pos2(0.0, 80.0),
            ],
            closed: true,
        };
        let hole = Outline {
            points: vec![
                pos2(20.0, 20.0),
                pos2(20.0, 60.0),
                pos2(60.0, 60.0),
                pos2(60.0, 20.0),
            ],
            closed: true,
        };
        let mut mesh = Mesh::default();
        scan_fill(&mut mesh, &[&outer, &hole], None).unwrap();
        let area: f32 = mesh
            .indices
            .chunks_exact(3)
            .map(|index| {
                let [a, b, c] =
                    [index[0], index[1], index[2]].map(|i| mesh.vertices[i as usize].pos);
                let (b, c) = (b - a, c - a);
                (b.x * c.y - b.y * c.x).abs() * 0.5
            })
            .sum();
        assert_eq!(area, 80.0 * 80.0 - 40.0 * 40.0);
    }
}
