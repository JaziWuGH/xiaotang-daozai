//! 插件画布的矢量绘制和静态几何缓存。

#[path = "plugin_canvas_geometry.rs"]
mod geometry;
#[path = "plugin_canvas_hit.rs"]
mod hit;
#[path = "plugin_canvas_motion.rs"]
mod motion;
#[path = "plugin_canvas_pet.rs"]
mod pet;

pub(crate) use motion::{draw_live_with_pet, LiveOptions, LiveResult};

use crate::{
    paint::Card,
    plugin_canvas::{Align, Canvas, Color, Fit, Glow, Kind, Node, Paint, PathCommand, Shadow, Style, Token},
    skin_paint::{self, Palette},
    theme::{self, W},
};
use eframe::{
    egui::{self, pos2, vec2, Color32, Pos2, Rect, Vec2},
    epaint::{Mesh, Shape, Tessellator, TextShape},
};
use geometry::{arc, path, rectangle, Clip, Outline, Pigment, Transform};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

const FRAME_BUDGET: Duration = Duration::from_millis(4);
const CACHE_LIMIT: usize = 8;

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct CanvasTheme {
    pub palette: Option<Palette>,
    pub accent: Option<Color32>,
    pub background: Color32,
}

impl CanvasTheme {
    fn token(self, token: &Token) -> Color32 {
        let nominal = match token {
            Token::Accent => return self.accent.unwrap_or_else(|| self.map(theme::BLUE)),
            Token::Text => theme::WHITE,
            Token::Muted => theme::GRAY,
            Token::Dim => theme::DIM,
            Token::Card => theme::CARD,
            Token::Track => theme::TRACK,
            Token::Bg => return self.background,
        };
        self.map(nominal)
    }

    fn map(self, color: Color32) -> Color32 {
        self.palette
            .map_or(color, |palette| skin_paint::map_color(color, &palette))
    }

    fn color(self, color: &Color) -> Color32 {
        match color {
            Color::Rgba([r, g, b, a]) => Color32::from_rgba_unmultiplied(*r, *g, *b, *a),
            Color::Token(token) => self.token(token),
            Color::Pick { pick, .. } => self.color(&pick[0]),
        }
    }

    fn pigment(self, paint: &Paint, transform: Transform, opacity: f32) -> Pigment {
        let color = |c: &Color| self.color(c).gamma_multiply(opacity);
        let stops = |stops: &[(f32, Color)]| {
            stops
                .iter()
                .map(|(offset, ink)| (*offset, color(ink)))
                .collect()
        };
        match paint {
            Paint::Solid(ink) => Pigment::Solid(color(ink)),
            Paint::Linear {
                from,
                to,
                stops: colors,
            } => {
                let from = transform.point(from);
                let direction = transform.point(to) - from;
                Pigment::Linear {
                    from,
                    projected: direction / direction.length_sq().max(f32::EPSILON),
                    stops: stops(colors),
                }
            }
            Paint::Radial {
                center,
                radius,
                stops: colors,
            } => Pigment::Radial {
                center: transform.point(center),
                radius: (radius.initial() * transform.scale).max(f32::EPSILON),
                stops: stops(colors),
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct View {
    scale: f32,
    pixels: f32,
    size: Vec2,
    theme: CanvasTheme,
    fonts: u64,
}

#[derive(Clone)]
struct Cache {
    id: egui::Id,
    view: View,
    canvas: Arc<Canvas>,
    scene: Option<Arc<Scene>>,
    failures: u8,
    previous: Option<Arc<Scene>>,
    geometry: HashMap<egui::Id, CachedMesh>,
}

#[derive(Clone)]
struct CachedMesh {
    kind: Kind,
    style: Style,
    transform: Transform,
    clip: Clip,
    opacity: f32,
    mesh: Arc<Mesh>,
}

#[derive(Default)]
struct Scene {
    items: Vec<(Rect, Item)>,
}

enum Item {
    Mesh(Arc<Mesh>),
    Text {
        job: egui::text::LayoutJob,
        base: Pos2,
        angle: f32,
        pigment: Pigment,
        clip: Clip,
    },
    Image {
        name: String,
        rect: Rect,
        transform: Transform,
        fit: Fit,
        color: Color32,
        clip: Clip,
    },
    Pet(pet::Placement),
}

/// 侧栏和快捷导航共用 24×24 的矢量路径，各自传入前景色。
pub(crate) fn draw_navigation_icon(
    painter: &egui::Painter,
    area: Rect,
    commands: &[PathCommand],
    color: Color32,
) {
    let side = area.width().min(area.height());
    let scale = side / 24.0;
    let transform = Transform::at(scale, area.center() - Vec2::splat(side / 2.0));
    let Some(outlines) = path(commands, transform, painter.pixels_per_point(), None) else {
        return;
    };
    let painter = painter.with_clip_rect(area);
    let stroke = egui::Stroke::new(1.35 * scale, color);
    for Outline { points, closed } in outlines {
        if points.len() < 2 {
            continue;
        }
        painter.add(if closed {
            Shape::closed_line(points, stroke)
        } else {
            Shape::line(points, stroke)
        });
    }
}

#[cfg(test)]
pub(crate) fn draw(
    c: &Card,
    id: &str,
    canvas: &Arc<Canvas>,
    area: Rect,
    theme: CanvasTheme,
    image: impl FnMut(&str) -> Option<(egui::TextureId, Vec2)>,
) -> bool {
    draw_canvas(c, id, canvas, area, theme, None, image)
}

/// 常驻内容沿用画布原字号，点击由常驻位处理。
pub(crate) fn draw_pinned(
    c: &Card,
    id: &str,
    canvas: &Arc<Canvas>,
    area: Rect,
    theme: CanvasTheme,
    elapsed: Duration,
    image: impl FnMut(&str) -> Option<(egui::TextureId, Vec2)>,
) -> bool {
    draw_canvas(c, id, canvas, area, theme, Some(elapsed), image)
}

fn draw_canvas(
    c: &Card,
    id: &str,
    canvas: &Arc<Canvas>,
    area: Rect,
    theme: CanvasTheme,
    pinned_elapsed: Option<Duration>,
    image: impl FnMut(&str) -> Option<(egui::TextureId, Vec2)>,
) -> bool {
    let options = LiveOptions {
        now: chrono::Utc::now(),
        revision: 0,
        allow_action: false,
        interactive: false,
    };
    let result = motion::draw_live_frame(c, id, canvas, area, theme, options, None, pinned_elapsed, image);
    if let Some(delay) = result.repaint_after {
        let dt = c.ui.input(|input| input.predicted_dt).max(0.0);
        c.ui.ctx()
            .request_repaint_after(delay + Duration::from_secs_f32(dt));
    }
    result.timed_out
}

pub(crate) fn has_pinned_text_flow(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| match &node.kind {
        Kind::Text { max_width: Some(_), .. } => node.looping.as_ref()
            .is_some_and(|effect| effect.kind == crate::plugin_canvas::live::LoopKind::Flow),
        Kind::Group { group, .. } => has_pinned_text_flow(group),
        _ => false,
    })
}

pub(crate) fn pinned_content_width(ui: &egui::Ui, canvas: &Canvas, limit: f32) -> f32 {
    let c = Card::new(ui, Pos2::ZERO, ui.clip_rect());
    let (canvas, _) = canvas.evaluate(chrono::Utc::now(), ui.ctx().pixels_per_point());
    let right = canvas.nodes.iter()
        .map(|node| hit::bounds(&c, node, Transform::new(1.0)).right())
        .fold(0.0, f32::max);
    (right + 4.0).max(24.0).min(limit)
}

/// 返回是否超出绘制预算。错误或超时由插件宿主登记到开发调试页面。
fn draw_static(
    c: &Card,
    id: &str,
    canvas: &Arc<Canvas>,
    area: Rect,
    theme: CanvasTheme,
    pets: &mut pet::DrawPets<'_>,
    mut image: impl FnMut(&str) -> Option<(egui::TextureId, Vec2)>,
) -> bool {
    let ctx = c.ui.ctx();
    let pixels = ctx.pixels_per_point();
    let fonts = ctx.fonts(|fonts| fonts.families().into_iter().filter(theme::canvas_font_family).collect::<Vec<_>>());
    let view = View {
        scale: c.s,
        pixels,
        size: area.size(),
        theme,
        fonts: egui::Id::new(&fonts).value(),
    };
    let id = c.ui.make_persistent_id(("plugin-canvas", id));
    let cache_id = egui::Id::new("plugin-canvas-scenes");
    let mut caches = ctx
        .data_mut(|data| std::mem::take(data.get_temp_mut_or_default::<VecDeque<Cache>>(cache_id)));
    let old = caches
        .iter()
        .position(|cache| cache.id == id)
        .and_then(|i| caches.remove(i));
    let mut cache = match old {
        Some(mut old)
            if old.view == view
                && (Arc::ptr_eq(&old.canvas, canvas) || old.canvas.as_ref() == canvas.as_ref()) =>
        {
            old.canvas = Arc::clone(canvas);
            old
        }
        old => {
            let (previous, geometry) = old.filter(|old| old.view == view).map_or_else(
                || (None, HashMap::new()),
                |old| (old.previous, old.geometry),
            );
            Cache {
                id,
                view,
                canvas: Arc::clone(canvas),
                scene: None,
                failures: 0,
                previous,
                geometry,
            }
        }
    };
    // 没有完整画面可回退时，必须完整绘制当前帧。
    let deadline = cache.previous.is_some().then(|| Instant::now() + FRAME_BUDGET);
    let offset = (area.min.to_vec2() * pixels).round() / pixels;
    let clip = area.intersect(c.painter.clip_rect());
    let mut timed_out = cache.failures >= 2;
    if !timed_out {
        if cache.scene.is_none() {
            let previous = std::mem::take(&mut cache.geometry);
            let (scene, geometry) = Builder::new(view, deadline, &fonts, previous).build(canvas);
            cache.scene = scene.map(Arc::new);
            cache.geometry = geometry;
        }
        if let Some(scene) = &cache.scene {
            timed_out = !scene.draw(c, offset, clip, deadline, theme, pets, &mut image);
            if !timed_out {
                cache.previous = Some(Arc::clone(scene));
                cache.failures = 0;
            }
        } else {
            timed_out = true;
        }
        if timed_out {
            cache.failures += 1;
            if cache.failures == 1 {
                ctx.request_repaint();
            }
        }
    }
    if timed_out {
        if let Some(previous) = &cache.previous {
            // 上一帧已细分的几何仍可复用；纹理跟随正在显示的包。
            previous.draw(c, offset, clip, None, theme, pets, &mut image);
        } else {
            let local = c.to_local(clip.center());
            c.centered_text(
                local.x,
                local.y,
                "插件画面有误",
                12.0,
                W::Regular,
                theme::DIM,
                crate::paint::H::Center,
            );
        }
    }
    caches.push_front(cache);
    caches.truncate(CACHE_LIMIT);
    ctx.data_mut(|data| data.insert_temp(cache_id, caches));
    timed_out
}

impl Scene {
    fn draw(
        &self,
        c: &Card,
        offset: Vec2,
        area: Rect,
        deadline: Option<Instant>,
        theme: CanvasTheme,
        pets: &mut pet::DrawPets<'_>,
        mut image: impl FnMut(&str) -> Option<(egui::TextureId, Vec2)>,
    ) -> bool {
        enum PaintItem<'a> {
            Shape(Shape),
            Pet(&'a pet::Placement),
        }
        let mut shapes = Vec::with_capacity(self.items.len());
        for (local_clip, item) in &self.items {
            let clip = local_clip.translate(offset).intersect(area);
            if !clip.is_positive() {
                continue;
            }
            if deadline.is_some_and(|at| Instant::now() >= at) {
                return false;
            }
            match item {
                Item::Mesh(mesh) => {
                    let mut shape = Shape::Mesh(Arc::clone(mesh));
                    if offset != Vec2::ZERO {
                        shape.translate(offset);
                    }
                    shapes.push((clip, PaintItem::Shape(shape)));
                }
                Item::Text {
                    job,
                    base,
                    angle,
                    pigment,
                    clip: group_clip,
                } => {
                    if let Some(shape) = text_shape(
                        c, job, *base, *angle, pigment, offset, clip, group_clip, deadline,
                    ) {
                        shapes.push((shape.0, PaintItem::Shape(shape.1)));
                    }
                }
                Item::Image {
                    name,
                    rect,
                    transform,
                    fit,
                    color,
                    clip: group_clip,
                } => {
                    if let Some((id, size)) = image(name) {
                        let mut mesh = image_mesh(id, size, *rect, *transform, fit, *color);
                        if group_clip.mesh(&mut mesh, |point| point, deadline).is_none() {
                            return false;
                        }
                        for vertex in &mut mesh.vertices {
                            vertex.pos += offset;
                        }
                        shapes.push((clip, PaintItem::Shape(Shape::mesh(mesh))));
                    }
                }
                Item::Pet(place) => shapes.push((clip, PaintItem::Pet(place))),
            }
        }
        if deadline.is_some_and(|at| Instant::now() >= at) {
            return false;
        }
        let layer = c.painter.layer_id();
        let first = skin_paint::shape_count(c.ui.ctx(), layer);
        let mut shapes = shapes.into_iter().peekable();
        while let Some((clip, item)) = shapes.next() {
            match item {
                PaintItem::Shape(shape) => {
                    c.painter
                        .with_clip_rect(clip)
                        .extend(std::iter::once(shape).chain(std::iter::from_fn(|| {
                            shapes
                                .next_if(|(next, item)| {
                                    *next == clip && matches!(item, PaintItem::Shape(_))
                                })
                                .and_then(|(_, item)| match item {
                                    PaintItem::Shape(shape) => Some(shape),
                                    PaintItem::Pet(_) => None,
                                })
                        })))
                }
                PaintItem::Pet(place) => pets.paint(c, place, offset, clip, theme),
            }
        }
        skin_paint::tone_since(c.ui.ctx(), layer, first, |color, _| color);
        true
    }
}

struct Builder<'a> {
    view: View,
    deadline: Option<Instant>,
    tessellator: Tessellator,
    scene: Scene,
    fonts: &'a [egui::FontFamily],
    previous: HashMap<egui::Id, CachedMesh>,
    geometry: HashMap<egui::Id, CachedMesh>,
}

impl<'a> Builder<'a> {
    fn new(view: View, deadline: Option<Instant>, fonts: &'a [egui::FontFamily], previous: HashMap<egui::Id, CachedMesh>) -> Self {
        let tessellator = Tessellator::new(view.pixels, Default::default(), [1; 2], Vec::new());
        Self {
            view,
            deadline,
            tessellator,
            fonts,
            scene: Scene::default(),
            previous,
            geometry: HashMap::new(),
        }
    }

    fn build(mut self, canvas: &Canvas) -> (Option<Scene>, HashMap<egui::Id, CachedMesh>) {
        let transform = Transform::new(self.view.scale);
        let clip = Clip::new(Rect::from_min_size(Pos2::ZERO, self.view.size));
        let complete = self
            .nodes(
                &canvas.nodes,
                transform,
                &clip,
                1.0,
                egui::Id::new("canvas-root-geometry"),
            )
            .is_some();
        if !complete {
            self.geometry.extend(self.previous);
        }
        (complete.then_some(self.scene), self.geometry)
    }

    fn nodes(
        &mut self,
        nodes: &[Node],
        transform: Transform,
        clip: &Clip,
        opacity: f32,
        parent: egui::Id,
    ) -> Option<()> {
        if !clip.rect.is_positive() {
            return Some(());
        }
        for (index, node) in nodes.iter().enumerate() {
            if self.deadline.is_some_and(|at| Instant::now() >= at) {
                return None;
            }
            self.node(
                node,
                transform,
                clip,
                opacity * node.style.opacity.initial().clamp(0.0, 1.0),
                parent.with(index),
            )?;
        }
        Some(())
    }

    fn node(
        &mut self,
        node: &Node,
        transform: Transform,
        clip: &Clip,
        opacity: f32,
        key: egui::Id,
    ) -> Option<()> {
        if opacity == 0.0 {
            return Some(());
        }
        if let Some(previous) = self.previous.remove(&key).filter(|previous| {
            previous.kind == node.kind
                && previous.style == node.style
                && previous.transform == transform
                && previous.clip == *clip
                && previous.opacity == opacity
        }) {
            self.mesh(clip.rect, Arc::clone(&previous.mesh));
            self.geometry.insert(key, previous);
            return Some(());
        }
        let style = &node.style;
        let theme = self.view.theme;
        let fill = style
            .fill
            .as_ref()
            .map(|paint| theme.pigment(paint, transform, opacity));
        let mut stroke = style
            .stroke
            .as_ref()
            .map(|paint| theme.pigment(paint, transform, opacity));
        let width = style.width.initial().max(0.0) * transform.scale;
        let (outlines, fill_outlines) = match &node.kind {
            Kind::Group {
                group,
                clip: group_clip,
                translate,
                rotate,
                scale,
            } => {
                let t = transform.child(translate, rotate.initial(), scale.initial().max(0.0));
                if t.scale <= 0.0 {
                    return Some(());
                }
                if t.scale > 1024.0 {
                    return None;
                }
                let group_clip = group_clip.as_ref().map(|r| clip.child(r, t));
                return self.nodes(group, t, group_clip.as_ref().unwrap_or(clip), opacity, key);
            }
            Kind::Text {
                text,
                at,
                size,
                weight,
                align,
                max_width,
                lines,
                font,
            } => {
                let size = (size.initial().clamp(6.0, 96.0) * transform.scale)
                    .min(256.0 / self.view.pixels);
                let max_width = max_width.as_ref().map(|v| v.initial() * transform.scale);
                let job = text_job(text, size, *weight, align, max_width, *lines, font.as_deref(), self.fonts);
                let pigment = fill.unwrap_or_else(|| {
                    Pigment::Solid(theme.token(&Token::Text).gamma_multiply(opacity))
                });
                self.scene.items.push((
                    clip.rect,
                    Item::Text {
                        job,
                        base: transform.point(at),
                        angle: transform.angle,
                        pigment,
                        clip: clip.clone(),
                    },
                ));
                return Some(());
            }
            Kind::Image { image, rect, fit } => {
                let rect = geometry::rect(rect);
                if rect.is_positive() {
                    self.scene.items.push((
                        clip.rect,
                        Item::Image {
                            name: image.clone(),
                            rect,
                            transform,
                            fit: *fit,
                            color: Color32::WHITE.gamma_multiply(opacity),
                            clip: clip.clone(),
                        },
                    ));
                }
                return Some(());
            }
            Kind::Icon { path: commands, rect } => {
                let rect = geometry::rect(rect);
                let scale = rect.width().min(rect.height()) / 24.0;
                if scale <= 0.0 {
                    return Some(());
                }
                let top_left = rect.center() - vec2(12.0, 12.0) * scale;
                let local = [top_left.x, top_left.y].map(crate::plugin_canvas::Num::Literal);
                if fill.is_none() && stroke.is_none() {
                    stroke = Some(Pigment::Solid(theme.token(&Token::Text).gamma_multiply(opacity)));
                }
                (path(commands, transform.child(&local, 0.0, scale), self.view.pixels, self.deadline)?, None)
            }
            Kind::Pet {
                at,
                size,
                mood,
                motion,
            } => {
                let size = size.initial() * transform.scale;
                let at = transform.point(at);
                if size > 0.0
                    && Rect::from_center_size(at, vec2(size * 1.5, size * 1.4))
                        .intersects(clip.rect)
                {
                    self.scene.items.push((
                        clip.rect,
                        Item::Pet(pet::Placement {
                            id: node.id.clone(),
                            at,
                            size,
                            angle: transform.angle,
                            clip: clip.clone(),
                            mood: *mood,
                            motion: *motion,
                            opacity,
                        }),
                    ));
                }
                return Some(());
            }
            Kind::Rect { rect, r } => (vec![rectangle(rect, r, transform)], None),
            Kind::Circle([x, y, r]) => (
                vec![arc(
                    transform.xy(x.initial(), y.initial()),
                    r.initial() * transform.scale,
                    0.0,
                    360.0,
                    self.view.pixels,
                )],
                None,
            ),
            Kind::Arc { c, r, start, sweep } => (
                vec![arc(
                    transform.point(c),
                    r.initial() * transform.scale,
                    start.initial() + transform.angle.to_degrees(),
                    sweep.initial(),
                    self.view.pixels,
                )],
                None,
            ),
            Kind::Line([x0, y0, x1, y1]) => (
                vec![Outline::open(vec![
                    transform.xy(x0.initial(), y0.initial()),
                    transform.xy(x1.initial(), y1.initial()),
                ])],
                None,
            ),
            Kind::Polyline { points, area } => {
                let line = Outline::open(points.iter().map(|p| transform.point(p)).collect());
                let area = area.as_ref().and_then(|area| {
                    let [first, last] = [points.first()?, points.last()?];
                    let mut points = line.points.clone();
                    points.push(transform.xy(last[0].initial(), area.initial()));
                    points.push(transform.xy(first[0].initial(), area.initial()));
                    Some(vec![Outline {
                        points,
                        closed: true,
                    }])
                });
                (vec![line], area)
            }
            Kind::Path(commands) => (
                path(commands, transform, self.view.pixels, self.deadline)?,
                None,
            ),
        };
        let mut mesh = Mesh::default();
        self.tessellator.set_clip_rect(clip.rect);
        if let Some(shadow) = &style.shadow {
            let bounds = outlines.iter().fold(Rect::NOTHING, |b, outline| {
                b.union(Rect::from_points(&outline.points))
            });
            if bounds.is_positive() {
                let (drop, blur) = match shadow {
                    Shadow::Sm => (2.0, 6.0),
                    Shadow::Md => (4.0, 12.0),
                    Shadow::Lg => (7.0, 20.0),
                };
                let shape = egui::epaint::RectShape::filled(
                    bounds.translate(vec2(0.0, drop * transform.scale)),
                    (blur * transform.scale * 0.5).round().min(255.0) as u8,
                    Color32::from_black_alpha(95).gamma_multiply(opacity),
                )
                .with_blur_width(blur * transform.scale);
                self.tessellator.tessellate_rect(&shape, &mut mesh);
            }
        }
        if let Some(glow) = &style.glow {
            let mut pigment = match glow {
                Glow::Color(color) => Pigment::Solid(theme.color(color).gamma_multiply(opacity)),
                Glow::Default => {
                    stroke
                        .as_ref()
                        .or(fill.as_ref())
                        .cloned()
                        .unwrap_or(Pigment::Solid(
                            theme.token(&Token::Accent).gamma_multiply(opacity),
                        ))
                }
            };
            pigment.multiply(0.10);
            for step in [9.0, 6.0, 3.0] {
                for outline in &outlines {
                    outline.stroke(
                        &mut self.tessellator,
                        &mut mesh,
                        &pigment,
                        width + step * transform.scale,
                        &style.cap,
                        None,
                        self.deadline,
                    )?;
                }
                pigment.multiply(1.35);
            }
        }
        let area_fill = fill_outlines.as_deref().unwrap_or(&outlines);
        if let Some(fill) = &fill {
            geometry::fill(
                &mut self.tessellator,
                &mut mesh,
                area_fill,
                fill,
                self.deadline,
            )?;
        }
        if let Some(stroke) = &stroke {
            let dash = style.dash.as_ref().map(|[length, gap]| {
                [
                    length.initial() * transform.scale,
                    gap.initial() * transform.scale,
                ]
            });
            for outline in &outlines {
                outline.stroke(
                    &mut self.tessellator,
                    &mut mesh,
                    stroke,
                    width,
                    &style.cap,
                    dash,
                    self.deadline,
                )?;
            }
        }
        clip.mesh(&mut mesh, |point| point, self.deadline)?;
        let mesh = Arc::new(mesh);
        self.mesh(clip.rect, Arc::clone(&mesh));
        self.geometry.insert(
            key,
            CachedMesh {
                kind: node.kind.clone(),
                style: node.style.clone(),
                transform,
                clip: clip.clone(),
                opacity,
                mesh,
            },
        );
        Some(())
    }

    fn mesh(&mut self, clip: Rect, mesh: Arc<Mesh>) {
        if !mesh.is_empty() {
            match self.scene.items.last_mut() {
                Some((last_clip, Item::Mesh(batch))) if *last_clip == clip => {
                    Arc::make_mut(batch).append_ref(&mesh)
                }
                _ => self.scene.items.push((clip, Item::Mesh(mesh))),
            }
        }
    }
}

fn text_job(
    text: &str,
    size: f32,
    weight: u16,
    align: &Align,
    max_width: Option<f32>,
    lines: u8,
    font: Option<&str>,
    fonts: &[egui::FontFamily],
) -> egui::text::LayoutJob {
    let weight = match weight {
        0..=499 => W::Regular,
        500..=649 => W::Semibold,
        _ => W::Bold,
    };
    let mut job = egui::text::LayoutJob::simple(
        crate::rtl_text::display(text).into_owned(),
        theme::canvas_font(size.max(0.1), weight, font, fonts),
        Color32::PLACEHOLDER,
        max_width.unwrap_or(f32::INFINITY),
    );
    job.wrap.max_rows = usize::from(lines);
    job.wrap.break_anywhere = lines == 1;
    job.halign = match align {
        Align::Left => egui::Align::LEFT,
        Align::Center => egui::Align::Center,
        Align::Right => egui::Align::RIGHT,
    };
    job
}

fn text_shape(
    c: &Card,
    job: &egui::text::LayoutJob,
    base: Pos2,
    angle: f32,
    pigment: &Pigment,
    offset: Vec2,
    clip: Rect,
    group_clip: &Clip,
    deadline: Option<Instant>,
) -> Option<(Rect, Shape)> {
    let mut galley = c.ui.fonts(|fonts| fonts.layout_job(job.clone()));
    let first_glyph = galley.rows.first()?.glyphs.first();
    let ascent = first_glyph.map_or(0.0, |glyph| glyph.pos.y);
    let rotation = egui::emath::Rot2::from_angle(angle);
    let position = base - rotation * vec2(0.0, ascent);
    let pixels = c.ui.ctx().pixels_per_point();
    let position = ((position.to_vec2() * pixels).round() / pixels).to_pos2();
    let mut local_clip = group_clip.clone();
    let width = job.wrap.max_width;
    let clip = if width.is_finite() {
        let left = match job.halign {
            egui::Align::Min => 0.0,
            egui::Align::Center => -width / 2.0,
            egui::Align::Max => -width,
        };
        if angle != 0.0 {
            local_clip.horizontal(position, rotation, left, left + width);
        }
        let bounds = Rect::from_min_size(pos2(left, 0.0), vec2(width, galley.size().y));
        clip.intersect(
            bounds
                .rotate_bb(rotation)
                .translate(position.to_vec2() + offset),
        )
    } else {
        clip
    };
    if !clip.is_positive() {
        return None;
    }
    let color = if let Pigment::Solid(color) = pigment {
        *color
    } else {
        for row in &mut Arc::make_mut(&mut galley).rows {
            for vertex in &mut row.visuals.mesh.vertices {
                vertex.color = pigment.color_at(position + rotation * vertex.pos.to_vec2());
            }
        }
        Color32::WHITE
    };
    if local_clip.has_planes() {
        for row in &mut Arc::make_mut(&mut galley).rows {
            local_clip.mesh(
                &mut row.visuals.mesh,
                |point| position + rotation * point.to_vec2(),
                deadline,
            )?;
            row.visuals.mesh_bounds = row.visuals.mesh.calc_bounds();
        }
    }
    let shape = TextShape::new(position + offset, galley, color).with_angle(angle);
    Some((clip, shape.into()))
}

fn image_mesh(
    id: egui::TextureId,
    size: Vec2,
    rect: Rect,
    transform: Transform,
    fit: &Fit,
    color: Color32,
) -> Mesh {
    let unit = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    let x = rect.width() / size.x;
    let y = rect.height() / size.y;
    let (rect, uv) = match fit {
        Fit::Contain => (Rect::from_center_size(rect.center(), size * x.min(y)), unit),
        Fit::Cover => (
            rect,
            Rect::from_center_size(unit.center(), rect.size() / (size * x.max(y))),
        ),
    };
    let mut mesh = Mesh::with_texture(id);
    mesh.add_rect_with_uv(rect, uv, color);
    for vertex in &mut mesh.vertices {
        vertex.pos = transform.pos(vertex.pos);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_canvas_first_frame_finishes_and_later_timeout_reuses_it() {
        let ctx = egui::Context::default();
        theme::load_fonts(&ctx);
        let frame = |text: &str| {
            Arc::new(Canvas::parse(serde_json::json!({
                "height": 40,
                "nodes": [
                    {"image": "probe.png", "rect": [0, 0, 8, 8]},
                    {"text": text, "at": [12, 21], "size": 12}
                ]
            })).unwrap())
        };
        let area = Rect::from_min_size(Pos2::ZERO, vec2(200.0, 40.0));
        let fallback = CanvasTheme { palette: None, accent: None, background: Color32::BLACK };
        let texture = ctx.load_texture(
            "首帧测试",
            egui::ColorImage::new([1, 1], Color32::WHITE),
            egui::TextureOptions::NEAREST,
        );
        for (text, expected) in [("首帧内容", false), ("后续内容", true)] {
            let canvas = frame(text);
            let output = ctx.run(
                egui::RawInput { screen_rect: Some(area), ..Default::default() },
                |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        let card = Card::new(ui, Pos2::ZERO, area);
                        assert_eq!(draw(&card, "first-frame", &canvas, area, fallback, |_| {
                            std::thread::sleep(FRAME_BUDGET + Duration::from_millis(1));
                            Some((texture.id(), texture.size_vec2()))
                        }), expected);
                    });
                },
            );
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                Shape::Text(text) if text.galley.text() == "首帧内容")));
            assert!(output.shapes.iter().any(|shape| shape.shape.texture_id() == texture.id()));
        }
    }
    use crate::skin_paint::{CREAM, GLASS_DARK, GLASS_LIGHT};

    #[test]
    fn plugin_canvas_theme_keeps_literal_colors_and_resolves_tokens() {
        let palette = CanvasTheme {
            palette: Some(CREAM),
            accent: Some(Color32::BLUE),
            background: Color32::RED,
        };
        assert_eq!(palette.token(&Token::Text), Color32::from_rgb(46, 32, 22));
        assert_eq!(palette.token(&Token::Accent), Color32::BLUE);
        assert_eq!(palette.token(&Token::Bg), Color32::RED);
        assert_eq!(palette.color(&Color::Rgba([255; 4])), Color32::WHITE);
    }

    #[test]
    fn plugin_canvas_demo_renders_at_both_scales() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let js = rquickjs::Context::full(&runtime).unwrap();
        let output: String = js.with(|js| {
            js.eval::<(), _>(include_str!("../spike/examples/canvas-demo/main.js"))
                .unwrap();
            js.eval("JSON.stringify(refresh().canvas)").unwrap()
        });
        let canvas = Arc::new(Canvas::parse(serde_json::from_str(&output).unwrap()).unwrap());
        let ctx = egui::Context::default();
        theme::load_fonts(&ctx);
        let badge = egui_extras::image::load_svg_bytes(include_bytes!(
            "../spike/examples/canvas-demo/badge.svg"
        ))
        .unwrap();
        let texture = ctx.load_texture("插件演示图", badge, egui::TextureOptions::LINEAR);
        let destination =
            std::env::var_os("DAOZAI_CANVAS_SCREENSHOTS").map(std::path::PathBuf::from);
        let mut screenshot = crate::preview_raster::Raster::new(419, 230);
        for (name, palette) in [
            ("classic", None),
            ("cream", Some(CREAM)),
            ("glass-light", Some(GLASS_LIGHT)),
            ("glass-dark", Some(GLASS_DARK)),
        ] {
            let background = palette.map_or(Color32::BLACK, |palette| {
                let [r, g, b] = palette.paper;
                Color32::from_rgb(r, g, b)
            });
            let theme = CanvasTheme {
                palette,
                accent: None,
                background,
            };
            for scale in [1.0, 2.0] {
                let panel = Rect::from_min_size(Pos2::ZERO, vec2(419.0, 230.0) * scale);
                let frame = |ui: &egui::Ui| {
                    let mut card = Card::new(ui, Pos2::ZERO, panel);
                    card.s = scale;
                    card.fill(0.0, 0.0, 419.0, 230.0, 18.0, background);
                    card.text(
                        16.0,
                        26.0,
                        "画布演示",
                        15.0,
                        W::Bold,
                        theme.token(&Token::Text),
                        crate::paint::H::Left,
                    );
                    draw(
                        &card,
                        "canvas-demo",
                        &canvas,
                        card.rect(16.0, 42.0, 387.0, canvas.height),
                        theme,
                        |name| (name == "badge.svg").then(|| (texture.id(), texture.size_vec2())),
                    )
                };
                let mut found = false;
                for _ in 0..2 {
                    let out = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(panel),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().frame(egui::Frame::NONE).show(
                                ctx,
                                |ui| {
                                    frame(ui);
                                },
                            );
                        },
                    );
                    screenshot.update_textures(&out.textures_delta);
                    let label = out.shapes.iter().find_map(|entry| match &entry.shape {
                        Shape::Text(shape) if shape.galley.text() == "72%" => {
                            Some((entry.clip_rect, shape))
                        }
                        _ => None,
                    });
                    if let Some((clip, label)) = label {
                        assert!(clip.width() <= 54.0 * scale + f32::EPSILON);
                        assert!(label.galley.size().x <= clip.width());
                        assert!(out
                            .shapes
                            .iter()
                            .any(|entry| entry.shape.texture_id() == texture.id()));
                        found = true;
                        if let Some(path) = destination.as_ref().filter(|_| scale == 1.0) {
                            std::fs::create_dir_all(path).unwrap();
                            let points = ctx.tessellate(out.shapes, out.pixels_per_point);
                            screenshot.buf.fill([0.0; 4]);
                            screenshot.draw(&points, out.pixels_per_point);
                            let mut rgba = Vec::new();
                            screenshot.to_rgba8(&mut rgba);
                            image::RgbaImage::from_raw(419, 230, rgba)
                                .unwrap()
                                .save(path.join(format!("A-canvas-{name}.png")))
                                .unwrap();
                        }
                    }
                    screenshot.free_textures(&out.textures_delta);
                    if found {
                        break;
                    }
                }
                assert!(found, "{name}、缩放 {scale}：未绘制演示画布");
            }
        }
    }
}
