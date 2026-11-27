//! 自绘画布的输出契约与校验。

use serde::{Deserialize, Deserializer};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) mod live;
mod path;
use live::{Animation, Binding, Burst, Hit, Hover, Loop, Scroll, Tip};
pub(crate) use path::{parse as parse_path, PathCommand};

const MAX_NODES: usize = 1500;
const MAX_DEPTH: usize = 8;
const MAX_PATH_COMMANDS: usize = 4000;
const MAX_TEXT: usize = 2000;
const COORD_LIMIT: f32 = 2000.0;

pub(crate) type Point = [Num; 2];
pub(crate) type Bounds = [Num; 4];

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Canvas {
    pub height: f32,
    pub nodes: Vec<Node>,
    pub binds: BTreeMap<String, Binding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCanvas {
    height: f32,
    nodes: Vec<RawNode>,
    #[serde(default)]
    binds: BTreeMap<String, Binding>,
}

impl Canvas {
    pub(crate) fn parse(value: serde_json::Value) -> Result<Self, String> {
        let raw: RawCanvas =
            serde_json::from_value(value).map_err(|error| format!("画布格式有误（{error}）"))?;
        if !(40.0..=360.0).contains(&raw.height) {
            return Err("画布高度必须在 40 到 360 之间".into());
        }
        Self::parse_nodes(raw, false)
    }

    pub(crate) fn parse_pinned(mut value: serde_json::Value) -> Result<(f32, Self), String> {
        let obj = value.as_object_mut().ok_or("常驻画布需要一个对象")?;
        let width = obj.remove("width").and_then(|width| width.as_f64())
            .filter(|width| (1.0..=156.0).contains(width))
            .ok_or("常驻画布宽度须在 1 到 156 之间")?;
        obj.entry("height").or_insert(serde_json::json!(22));
        let raw: RawCanvas = serde_json::from_value(value)
            .map_err(|error| format!("常驻画布格式有误（{error}）"))?;
        if raw.height != 22.0 {
            return Err("常驻画布高度须为 22".into());
        }
        Ok((width as f32, Self::parse_nodes(raw, true)?))
    }

    fn parse_nodes(raw: RawCanvas, pinned: bool) -> Result<Self, String> {
        for (name, binding) in &raw.binds {
            if !valid_name(name) || !binding.valid() {
                return Err(format!("画布绑定无效（{name}）"));
            }
        }
        let mut budget = Budget {
            binds: &raw.binds,
            nodes: 0,
            commands: 0,
            text: 0,
            loops: 0,
            pets: 0,
            hits: 0,
            ids: BTreeSet::new(),
        };
        let nodes = budget.nodes(raw.nodes, 1)?;
        if pinned {
            if budget.nodes > 60 {
                return Err("常驻画布最多 60 个节点".into());
            }
            fn has_interaction(nodes: &[Node]) -> bool {
                nodes.iter().any(|node| node.hit.is_some() || node.hover.is_some()
                    || node.scroll.is_some() || node.looping.as_ref().is_some_and(|effect| {
                        effect.kind != live::LoopKind::Flow || !matches!(&node.kind, Kind::Text { .. })
                    }) || node.burst.is_some()
                    || node.animate.is_some() || !node.tips.is_empty()
                    || matches!(&node.kind, Kind::Pet { .. })
                    || matches!(&node.kind, Kind::Group { group, .. } if has_interaction(group)))
            }
            if has_interaction(&nodes) {
                return Err("常驻画布只支持绑定、数字翻动与横向滚动文字".into());
            }
        }
        Ok(Self {
            height: raw.height,
            nodes,
            binds: raw.binds,
        })
    }

    pub(crate) fn validate_images(&self, exists: impl Fn(&str) -> bool) -> Result<(), String> {
        fn check(nodes: &[Node], exists: &impl Fn(&str) -> bool) -> Result<(), String> {
            for node in nodes {
                match &node.kind {
                    Kind::Image { image, .. } if !exists(image) => {
                        return Err(format!("画布引用的包内图片不存在（{image}）"));
                    }
                    Kind::Group { group, .. } => check(group, exists)?,
                    _ => {}
                }
            }
            Ok(())
        }
        check(&self.nodes, &exists)
    }
}

pub(crate) fn valid_image_path(path: &str) -> bool {
    !path.contains("..")
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && !part.ends_with(['.', ' '])
                && !part
                    .chars()
                    .any(|ch| ch.is_control() || "\\:<>\"|?*".contains(ch))
        })
        && path.rsplit_once('.').is_some_and(|(_, ext)| {
            ext.eq_ignore_ascii_case("png") || ext.eq_ignore_ascii_case("svg")
        })
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && !name.chars().any(char::is_control)
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum Num {
    Literal(f32),
    Bind { bind: String, map: Option<[f32; 2]> },
}

impl Num {
    pub(crate) fn initial(&self) -> f32 {
        match self {
            Self::Literal(value) => *value,
            Self::Bind { map, .. } => map.map_or(0.0, |[start, _]| start),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Color {
    Rgba([u8; 4]),
    Token(Token),
    Pick { bind: String, pick: Box<[Color; 2]> },
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged, deny_unknown_fields)]
        enum Raw {
            Name(String),
            Pick { bind: String, pick: [String; 2] },
        }

        let parse = |name: String| -> Result<Self, D::Error> {
            color_name(&name)
                .ok_or_else(|| serde::de::Error::custom(format!("画布颜色无效（{name}）")))
        };
        match Raw::deserialize(input)? {
            Raw::Name(name) => parse(name),
            Raw::Pick {
                bind,
                pick: [off, on],
            } => Ok(Self::Pick {
                bind,
                pick: Box::new([parse(off)?, parse(on)?]),
            }),
        }
    }
}

fn color_name(name: &str) -> Option<Color> {
    let token = match name {
        "$accent" => Token::Accent,
        "$text" => Token::Text,
        "$muted" => Token::Muted,
        "$dim" => Token::Dim,
        "$card" => Token::Card,
        "$track" => Token::Track,
        "$bg" => Token::Bg,
        _ => {
            let hex = name.strip_prefix('#')?;
            if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            let rgba = u32::from_str_radix(hex, 16).ok()?;
            let rgba = if hex.len() == 6 {
                rgba << 8 | 255
            } else {
                rgba
            };
            return Some(Color::Rgba(rgba.to_be_bytes()));
        }
    };
    Some(Color::Token(token))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Token {
    Accent,
    Text,
    Muted,
    Dim,
    Card,
    Track,
    Bg,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Paint {
    Solid(Color),
    Linear {
        from: Point,
        to: Point,
        stops: Vec<(f32, Color)>,
    },
    Radial {
        center: Point,
        radius: Num,
        stops: Vec<(f32, Color)>,
    },
}

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum RawPaint {
    Linear {
        linear: [Point; 2],
        stops: Vec<(f32, Color)>,
    },
    Radial {
        radial: RawRadial,
        stops: Vec<(f32, Color)>,
    },
    Solid(Color),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawRadial {
    Radius((Point, Num)),
    Edge([[f32; 2]; 2]),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Style {
    pub fill: Option<Paint>,
    pub stroke: Option<Paint>,
    pub width: Num,
    pub cap: Cap,
    pub dash: Option<[Num; 2]>,
    pub opacity: Num,
    pub shadow: Option<Shadow>,
    pub glow: Option<Glow>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Fit {
    #[default]
    Contain,
    Cover,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Cap {
    Round,
    #[default]
    Butt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Shadow {
    Sm,
    Md,
    Lg,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Glow {
    Default,
    Color(Color),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawGlow {
    Enabled(bool),
    Color(Color),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Node {
    pub kind: Kind,
    pub style: Style,
    pub id: Option<String>,
    pub roll: bool,
    pub animate: Option<Animation>,
    pub looping: Option<Loop>,
    pub burst: Option<Burst>,
    pub hit: Option<Hit>,
    pub hover: Option<Hover>,
    pub scroll: Option<Scroll>,
    pub tips: Vec<Tip>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Rect {
        rect: Bounds,
        r: [Num; 4],
    },
    Circle([Num; 3]),
    Arc {
        c: Point,
        r: Num,
        start: Num,
        sweep: Num,
    },
    Line(Bounds),
    Polyline {
        points: Vec<Point>,
        area: Option<Num>,
    },
    Path(Vec<PathCommand>),
    Text {
        text: String,
        at: Point,
        size: Num,
        weight: u16,
        align: Align,
        max_width: Option<Num>,
        lines: u8,
        font: Option<String>,
    },
    Image {
        image: String,
        rect: Bounds,
        fit: Fit,
    },
    Icon { path: Vec<PathCommand>, rect: Bounds },
    Pet {
        at: Point,
        size: Num,
        mood: PetMood,
        motion: PetMotion,
    },
    Group {
        group: Vec<Node>,
        clip: Option<Bounds>,
        translate: Point,
        rotate: Num,
        scale: Num,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNode {
    rect: Option<Bounds>,
    r: Option<RawRadii>,
    circle: Option<[Num; 3]>,
    arc: Option<RawArc>,
    line: Option<Bounds>,
    points: Option<Vec<Point>>,
    area: Option<Num>,
    path: Option<String>,
    text: Option<String>,
    at: Option<Point>,
    size: Option<Num>,
    weight: Option<u16>,
    #[serde(default)]
    align: Align,
    max_width: Option<Num>,
    lines: Option<u8>,
    font: Option<String>,
    image: Option<String>,
    icon: Option<String>,
    pet: Option<RawPet>,
    #[serde(default)]
    fit: Fit,
    group: Option<Vec<RawNode>>,
    clip: Option<Bounds>,
    translate: Option<Point>,
    rotate: Option<Num>,
    scale: Option<Num>,
    fill: Option<RawPaint>,
    color: Option<RawPaint>,
    stroke: Option<RawPaint>,
    width: Option<Num>,
    #[serde(default)]
    cap: Cap,
    dash: Option<[Num; 2]>,
    opacity: Option<Num>,
    shadow: Option<Shadow>,
    glow: Option<RawGlow>,
    id: Option<String>,
    #[serde(default)]
    roll: bool,
    animate: Option<Animation>,
    #[serde(rename = "loop")]
    looping: Option<Loop>,
    burst: Option<Burst>,
    hit: Option<Hit>,
    hover: Option<Hover>,
    scroll: Option<Scroll>,
    #[serde(default)]
    tips: Vec<Tip>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PetMotion {
    #[default]
    None,
    Run,
    Bob,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PetMood {
    #[default]
    Expressions,
    Sleeping,
    Happy,
    Expression(crate::pet_expressions::Expression),
}

impl<'de> Deserialize<'de> for PetMood {
    fn deserialize<D: Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        use crate::pet_expressions::Expression;
        use serde::de::Error;
        let name = String::deserialize(input)?;
        match name.as_str() {
            "expressions" => Ok(Self::Expressions),
            "sleeping" => Ok(Self::Sleeping),
            "happy" => Ok(Self::Happy),
            name => Expression::ALL
                .into_iter()
                .find(|expression| expression.key() == name)
                .map(Self::Expression)
                .ok_or_else(|| D::Error::custom("无效的岛仔表情")),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPet {
    at: Point,
    size: Num,
    #[serde(default)]
    mood: PetMood,
    #[serde(default)]
    motion: PetMotion,
    id: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawRadii {
    Single(Num),
    Corners([Num; 4]),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArc {
    c: Point,
    r: Num,
    start: Num,
    sweep: Num,
}

type Validation = Result<(), String>;

struct Budget<'a> {
    binds: &'a BTreeMap<String, Binding>,
    nodes: usize,
    commands: usize,
    text: usize,
    loops: usize,
    pets: usize,
    hits: usize,
    ids: BTreeSet<String>,
}

impl Budget<'_> {
    fn nodes(&mut self, nodes: Vec<RawNode>, depth: usize) -> Result<Vec<Node>, String> {
        if depth > MAX_DEPTH && !nodes.is_empty() {
            return Err("画布最多嵌套 8 层".into());
        }
        self.nodes += nodes.len();
        if self.nodes > MAX_NODES {
            return Err("画布最多 1500 个节点".into());
        }
        nodes
            .into_iter()
            .map(|node| self.node(node, depth))
            .collect()
    }

    fn reference(&self, name: &str) -> Validation {
        if self.binds.contains_key(name) {
            Ok(())
        } else {
            Err(format!("画布绑定不存在（{name}）"))
        }
    }

    fn number(&self, number: &Num, min: f32, max: f32, name: &str) -> Validation {
        let check = |value: f32| {
            if (min..=max).contains(&value) {
                Ok(())
            } else {
                Err(format!("画布{name}必须为 {min} 到 {max} 之间的有限值"))
            }
        };
        match number {
            Num::Literal(value) => check(*value),
            Num::Bind { bind, map } => {
                self.reference(bind)?;
                if let Some([start, end]) = map {
                    check(*start)?;
                    check(*end)?;
                }
                Ok(())
            }
        }
    }

    fn coord(&self, number: &Num) -> Validation {
        self.number(number, -COORD_LIMIT, COORD_LIMIT, "坐标")
    }

    fn coords(&self, numbers: &[Num]) -> Validation {
        numbers.iter().try_for_each(|number| self.coord(number))
    }

    fn length(&self, number: &Num) -> Validation {
        self.number(number, 0.0, COORD_LIMIT, "长度")
    }

    fn positive(&self, number: &Num) -> Validation {
        self.number(number, f32::MIN_POSITIVE, COORD_LIMIT, "长度")
    }

    fn bounds(&self, bounds: &Bounds) -> Validation {
        self.coords(&bounds[..2])?;
        bounds[2..]
            .iter()
            .try_for_each(|number| self.length(number))
    }

    fn color(&self, color: &Color) -> Validation {
        match color {
            Color::Pick { bind, .. } => self.reference(bind),
            _ => Ok(()),
        }
    }

    fn paint(&self, paint: RawPaint) -> Result<Paint, String> {
        let check_stops = |stops: &[(f32, Color)]| {
            if stops.len() < 2
                || stops.iter().any(|(pos, _)| !(0.0..=1.0).contains(pos))
                || stops.windows(2).any(|pair| pair[0].0 > pair[1].0)
            {
                return Err("画布渐变至少需要两个色标，位置须在 0 到 1 内按顺序排列".into());
            }
            stops.iter().try_for_each(|(_, color)| self.color(color))
        };
        Ok(match paint {
            RawPaint::Solid(color) => {
                self.color(&color)?;
                Paint::Solid(color)
            }
            RawPaint::Linear {
                linear: [from, to],
                stops,
            } => {
                self.coords(&from)?;
                self.coords(&to)?;
                check_stops(&stops)?;
                Paint::Linear { from, to, stops }
            }
            RawPaint::Radial { radial, stops } => {
                let (center, radius) = match radial {
                    RawRadial::Radius((center, radius)) => (center, radius),
                    RawRadial::Edge([center, edge]) => {
                        self.coords(&edge.map(Num::Literal))?;
                        let radius = (center[0] - edge[0]).hypot(center[1] - edge[1]);
                        (center.map(Num::Literal), Num::Literal(radius))
                    }
                };
                self.coords(&center)?;
                self.positive(&radius)?;
                check_stops(&stops)?;
                Paint::Radial {
                    center,
                    radius,
                    stops,
                }
            }
        })
    }

    fn node(&mut self, mut raw: RawNode, depth: usize) -> Result<Node, String> {
        let kind = self.kind(&mut raw, depth)?;
        if let Some(id) = &raw.id {
            if !valid_name(id) || !self.ids.insert(id.clone()) {
                return Err(format!("画布节点 id 无效或重复（{id}）"));
            }
        }
        self.live_fields(&kind, &raw)?;
        let fill = match (raw.fill, raw.color) {
            (None, color) if matches!(kind, Kind::Text { .. }) => color,
            (fill, None) => fill,
            _ => return Err("画布文字颜色只能在 fill 或 color 中定义一次".into()),
        };
        let glow = match raw.glow {
            Some(RawGlow::Enabled(true)) => Some(Glow::Default),
            Some(RawGlow::Enabled(false)) => return Err("画布发光需写 true 或颜色".into()),
            Some(RawGlow::Color(color)) => {
                self.color(&color)?;
                Some(Glow::Color(color))
            }
            None => None,
        };
        let width = raw.width.unwrap_or(Num::Literal(1.0));
        let opacity = raw.opacity.unwrap_or(Num::Literal(1.0));
        self.length(&width)?;
        self.number(&opacity, 0.0, 1.0, "透明度")?;
        if let Some(dash) = &raw.dash {
            dash.iter().try_for_each(|number| self.positive(number))?;
        }
        Ok(Node {
            kind,
            style: Style {
                fill: fill.map(|paint| self.paint(paint)).transpose()?,
                stroke: raw.stroke.map(|paint| self.paint(paint)).transpose()?,
                width,
                cap: raw.cap,
                dash: raw.dash,
                opacity,
                shadow: raw.shadow,
                glow,
            },
            id: raw.id,
            roll: raw.roll,
            animate: raw.animate,
            looping: raw.looping,
            burst: raw.burst,
            hit: raw.hit,
            hover: raw.hover,
            scroll: raw.scroll,
            tips: raw.tips,
        })
    }

    fn kind(&mut self, raw: &mut RawNode, depth: usize) -> Result<Kind, String> {
        let count = [
            raw.rect.is_some() && raw.image.is_none() && raw.icon.is_none(),
            raw.circle.is_some(),
            raw.arc.is_some(),
            raw.line.is_some(),
            raw.points.is_some(),
            raw.path.is_some(),
            raw.text.is_some(),
            raw.image.is_some(),
            raw.icon.is_some(),
            raw.pet.is_some(),
            raw.group.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        if count != 1 {
            return Err("画布节点必须包含一种图元".into());
        }
        if let Some(image) = raw.image.take() {
            let rect = raw.rect.take().ok_or("画布图片缺少 rect")?;
            self.bounds(&rect)?;
            if !valid_image_path(&image) {
                return Err(format!("画布图片须为包内 PNG 或 SVG（{image}）"));
            }
            return Ok(Kind::Image {
                image,
                rect,
                fit: raw.fit,
            });
        }
        if let Some(icon) = raw.icon.take() {
            let rect = raw.rect.take().ok_or("画布图标缺少 rect")?;
            self.bounds(&rect)?;
            let svg_path = crate::plugin_setting_catalog::icon_path(&icon)
                .ok_or_else(|| format!("未识别的画布图标（{icon}）"))?;
            let path = path::parse(svg_path, MAX_PATH_COMMANDS - self.commands)?;
            self.commands += path.len();
            return Ok(Kind::Icon { path, rect });
        }
        if let Some(rect) = raw.rect.take() {
            self.bounds(&rect)?;
            let r = match raw.r.take().unwrap_or(RawRadii::Single(Num::Literal(0.0))) {
                RawRadii::Single(radius) => std::array::from_fn(|_| radius.clone()),
                RawRadii::Corners(corners) => corners,
            };
            r.iter().try_for_each(|radius| self.length(radius))?;
            return Ok(Kind::Rect { rect, r });
        }
        if let Some(circle) = raw.circle.take() {
            self.coords(&circle[..2])?;
            self.positive(&circle[2])?;
            return Ok(Kind::Circle(circle));
        }
        if let Some(RawArc { c, r, start, sweep }) = raw.arc.take() {
            self.coords(&c)?;
            self.positive(&r)?;
            self.coord(&start)?;
            self.coord(&sweep)?;
            return Ok(Kind::Arc { c, r, start, sweep });
        }
        if let Some(line) = raw.line.take() {
            self.coords(&line)?;
            return Ok(Kind::Line(line));
        }
        if let Some(points) = raw.points.take() {
            if points.len() < 2 {
                return Err("画布折线至少需要两个点".into());
            }
            for point in &points {
                self.coords(point)?;
            }
            let area = raw.area.take();
            if let Some(baseline) = &area {
                self.coord(baseline)?;
            }
            return Ok(Kind::Polyline { points, area });
        }
        if let Some(path) = raw.path.take() {
            let commands = path::parse(&path, MAX_PATH_COMMANDS - self.commands)?;
            self.commands += commands.len();
            return Ok(Kind::Path(commands));
        }
        if let Some(text) = raw.text.take() {
            return self.text(raw, text);
        }
        if let Some(pet) = raw.pet.take() {
            self.pets += 1;
            if self.pets > 1 {
                return Err("每个画布最多放一只岛仔".into());
            }
            if raw.looping.is_some() {
                return Err("岛仔动画通过 pet.motion 声明".into());
            }
            if pet.motion != PetMotion::None
                || matches!(pet.mood, PetMood::Expressions | PetMood::Expression(_))
            {
                self.loops += 1;
                if self.loops > 8 {
                    return Err("画布最多同时循环 8 个节点".into());
                }
            }
            if let Some(id) = pet.id {
                if raw.id.as_ref().is_some_and(|other| *other != id) {
                    return Err("岛仔节点的 id 不一致".into());
                }
                raw.id = Some(id);
            }
            self.coords(&pet.at)?;
            self.number(&pet.size, 12.0, 96.0, "岛仔大小")?;
            return Ok(Kind::Pet {
                at: pet.at,
                size: pet.size,
                mood: pet.mood,
                motion: pet.motion,
            });
        }
        let group = self.nodes(raw.group.take().ok_or("画布分组缺少节点")?, depth + 1)?;
        let clip = raw.clip.take();
        if let Some(clip) = &clip {
            self.bounds(clip)?;
        }
        let translate = raw
            .translate
            .take()
            .unwrap_or([Num::Literal(0.0), Num::Literal(0.0)]);
        let rotate = raw.rotate.take().unwrap_or(Num::Literal(0.0));
        let scale = raw.scale.take().unwrap_or(Num::Literal(1.0));
        self.coords(&translate)?;
        self.coord(&rotate)?;
        self.positive(&scale)?;
        Ok(Kind::Group {
            group,
            clip,
            translate,
            rotate,
            scale,
        })
    }

    fn text(&mut self, raw: &mut RawNode, text: String) -> Result<Kind, String> {
        self.text += text.chars().count();
        if self.text > MAX_TEXT {
            return Err("画布文字最多 2000 字".into());
        }
        if text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
        {
            return Err("画布文字含非法字符".into());
        }
        let at = raw.at.take().ok_or("画布文字缺少 at")?;
        let size = raw.size.take().ok_or("画布文字缺少 size")?;
        let weight = raw.weight.take().unwrap_or(400);
        let lines = raw.lines.take().unwrap_or(1);
        let max_width = raw.max_width.take();
        let font = raw.font.take();
        self.coords(&at)?;
        self.number(&size, 6.0, 96.0, "字号")?;
        if let Some(max_width) = &max_width {
            self.positive(max_width)?;
        }
        if !(1..=1000).contains(&weight) || lines == 0 {
            return Err("画布文字需要有效字重和行数".into());
        }
        if font.as_deref().is_some_and(|name| !valid_name(name)) {
            return Err("画布字体名称无效".into());
        }
        Ok(Kind::Text {
            text,
            at,
            size,
            weight,
            align: raw.align,
            max_width,
            lines,
            font,
        })
    }

    fn live_fields(&mut self, kind: &Kind, raw: &RawNode) -> Validation {
        if raw
            .animate
            .as_ref()
            .is_some_and(|animation| !(120..=1200).contains(&animation.ms))
        {
            return Err("画布过渡时长必须在 120 到 1200 毫秒之间".into());
        }
        if let Some(animation) = &raw.looping {
            self.loops += 1;
            if self.loops > 8 || animation.period_ms < 800 {
                return Err("画布最多同时循环 8 个节点，且周期至少为 800 毫秒".into());
            }
        }
        if let Some(hit) = &raw.hit {
            self.hits += 1;
            if self.hits > 64 {
                return Err("画布最多 64 个点击区域".into());
            }
            if !hit.valid() {
                return Err("画布点击内容无效".into());
            }
        }
        if let Some(hover) = &raw.hover {
            if let Some(opacity) = &hover.opacity {
                self.number(opacity, 0.0, 1.0, "悬停透明度")?;
            }
            if let Some(lift) = &hover.lift {
                self.coord(lift)?;
            }
            if let Some(fill) = &hover.fill {
                self.color(fill)?;
            }
            if let Some(hint) = &hover.hint {
                if !crate::plugin_status::valid_text(hint, 40) {
                    return Err("画布悬停提示需要 1 至 40 字的单行文字".into());
                }
                self.text += hint.chars().count();
                if self.text > MAX_TEXT {
                    return Err("画布文字最多 2000 字".into());
                }
            }
        }
        if let Some(scroll) = &raw.scroll {
            if !matches!(kind, Kind::Group { clip: Some(_), .. }) || !scroll.valid() {
                return Err("画布滚动区需要分组、裁剪区域和有效内容高度".into());
            }
        }
        if !raw.tips.is_empty() {
            if !matches!(kind, Kind::Polyline { .. }) {
                return Err("画布悬停读数只能用于折线".into());
            }
            for tip in &raw.tips {
                self.coord(&Num::Literal(tip.x))?;
            }
        }
        if raw.roll && !matches!(kind, Kind::Text { .. }) {
            return Err("画布数字翻动只能用于文字".into());
        }
        if raw.burst.is_some() && raw.id.is_none() {
            return Err("画布一次性效果需要节点 id".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
