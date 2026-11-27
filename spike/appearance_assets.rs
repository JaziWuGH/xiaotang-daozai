//! 外观资源包的校验、选择和宿主读取接口。
use super::{bounded, fail, font_assets, LocalPlugins, Manifest, Result};
use crate::model::Skin;
use crate::skin_paint::{Palette, CLASSIC, CREAM, GLASS_DARK, GLASS_LIGHT};
use base64::{engine::general_purpose::STANDARD, Engine};
use eframe::egui;
use image::ImageDecoder;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Cursor, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Skin,
    Icon,
    Pet,
    Font,
}

impl Kind {
    const SELECTABLE: [Self; 3] = [Self::Skin, Self::Icon, Self::Pet];

    pub(super) fn of(manifest: &Manifest) -> Option<Self> {
        if manifest.runtime != "assets" {
            return None;
        }
        Self::SELECTABLE
            .into_iter()
            .chain([Self::Font])
            .find(|kind| manifest.capabilities.iter().any(|c| c == kind.capability()))
    }

    pub(super) fn capability(self) -> &'static str {
        match self {
            Self::Skin => "appearance.skin",
            Self::Icon => "appearance.icon",
            Self::Pet => "appearance.pet",
            Self::Font => "appearance.font",
        }
    }

    pub(super) fn entry(self) -> &'static str {
        match self {
            Self::Skin => "theme.json",
            Self::Icon => "icons.json",
            Self::Pet => "expressions.json",
            Self::Font => "fonts.json",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Skin => "皮肤",
            Self::Icon => "功能图标",
            Self::Pet => "吉祥物表情",
            Self::Font => "画布字体",
        }
    }

    pub(super) fn limit(self) -> u64 {
        match self {
            Self::Skin => 4096,
            Self::Icon => 128 * 1024,
            Self::Pet => 768 * 1024,
            Self::Font => 4096,
        }
    }
}

#[derive(Clone)]
pub(super) enum Pack {
    Skin(SkinPack),
    Icon(BTreeMap<String, Icon>),
    Pet(Arc<crate::pet_expressions::ExpressionSheets>),
    Font(font_assets::Entry),
}

impl Pack {
    pub(super) fn decode(kind: Kind, source: &str) -> Result<Self> {
        if source.len() as u64 > kind.limit() {
            return Err(fail("外观资源入口文件过大"));
        }
        Ok(match kind {
            Kind::Skin => Self::Skin(SkinPack::decode(source)?),
            Kind::Icon => Self::Icon(parse_icons(source)?),
            Kind::Pet => Self::Pet(Arc::new(parse_expressions(source)?)),
            Kind::Font => Self::Font(font_assets::Entry::decode(source)?),
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SkinPack {
    pub(crate) base: Skin,
    pub(crate) palette: Palette,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkinEntry {
    base: SkinBase,
    colors: SkinColors,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SkinBase {
    Classic,
    Cream,
    GlassLight,
    GlassDark,
    Fluid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkinColors {
    #[serde(default, deserialize_with = "super::present_value")]
    paper: Option<String>,
    #[serde(default, deserialize_with = "super::present_value")]
    ink: Option<String>,
    #[serde(default, deserialize_with = "super::present_value")]
    accent: Option<String>,
}

impl SkinPack {
    fn decode(source: &str) -> Result<Self> {
        let entry: SkinEntry = serde_json::from_str(source)?;
        let (base, mut palette) = match entry.base {
            SkinBase::Classic => (Skin::Classic, CLASSIC),
            SkinBase::Cream => (Skin::Cream, CREAM),
            SkinBase::GlassLight => (Skin::GlassLight, GLASS_LIGHT),
            SkinBase::GlassDark => (Skin::GlassDark, GLASS_DARK),
            SkinBase::Fluid => (Skin::Fluid, CLASSIC),
        };
        let SkinColors { paper, ink, accent } = entry.colors;
        if paper.is_none() && ink.is_none() && accent.is_none() {
            return Err(fail("皮肤包至少需要一个颜色令牌"));
        }
        if !matches!(base, Skin::Classic | Skin::Cream) && (paper.is_some() || ink.is_some()) {
            return Err(fail("毛玻璃和悬浮云皮肤只开放 accent 配色"));
        }
        if let Some(paper) = paper {
            palette.paper = rgb(&paper)?;
        }
        if let Some(ink) = ink {
            palette.ink = rgb(&ink)?;
        }
        if let Some(accent) = accent {
            palette.accent = Some(rgb(&accent)?);
        }
        Ok(Self { base, palette })
    }
}

fn rgb(value: &str) -> Result<[u8; 3]> {
    let hex = value
        .strip_prefix('#')
        .filter(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| fail("皮肤颜色须为 #RRGGBB"))?;
    let [_, red, green, blue] = u32::from_str_radix(hex, 16)?.to_be_bytes();
    Ok([red, green, blue])
}

#[derive(Clone)]
pub(super) struct Icon {
    bytes: Arc<[u8]>,
    hash: String,
}

fn parse_icons(source: &str) -> Result<BTreeMap<String, Icon>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Entry {
        icons: BTreeMap<String, String>,
    }
    let Entry { icons } = serde_json::from_str(source)?;
    if icons.is_empty()
        || !icons
            .keys()
            .all(|key| crate::app::PAGE_SHORTCUT_KEYS.contains(&key.as_str()))
    {
        return Err(fail("图标包需要至少一个已开放的功能页图标"));
    }
    icons
        .into_iter()
        .map(|(key, svg)| {
            validate_svg(&svg)?;
            let hash = ring::digest::digest(&ring::digest::SHA256, svg.as_bytes());
            let hash = hash
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            Ok((
                key,
                Icon {
                    bytes: Arc::from(svg.into_bytes()),
                    hash,
                },
            ))
        })
        .collect()
}

fn validate_svg(svg: &str) -> Result<()> {
    load_static_svg(svg, 8 * 1024, 512)?;
    Ok(())
}

pub(super) fn load_static_svg(svg: &str, bytes: usize, side: u32) -> Result<eframe::egui::ColorImage> {
    const SVG_NS: &str = "http://www.w3.org/2000/svg";
    if svg.len() > bytes {
        return Err(fail("SVG 超过大小限制"));
    }
    let doc = roxmltree::Document::parse(svg)?;
    let root = doc.root_element();
    let view_box = root
        .attribute("viewBox")
        .unwrap_or_default()
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|word| !word.is_empty())
        .map(str::parse::<f64>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if root.tag_name().name() != "svg"
        || root.tag_name().namespace() != Some(SVG_NS)
        || view_box.len() != 4
        || !view_box.iter().all(|v| v.abs() <= f64::from(side))
        || view_box[2] < 1.0
        || view_box[3] < 1.0
    {
        return Err(fail("SVG 功能图标需要有效的 viewBox"));
    }
    for node in doc.descendants() {
        if node.is_pi() || node.is_text() && !node.text().unwrap_or_default().trim().is_empty() {
            return Err(fail("SVG 功能图标只接受静态矢量图形"));
        }
        if !node.is_element() {
            continue;
        }
        if node.tag_name().namespace() != Some(SVG_NS)
            || node != root
                && !matches!(
                    node.tag_name().name(),
                    "g" | "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon"
                )
        {
            return Err(fail("SVG 功能图标只接受静态矢量图形"));
        }
        for attr in node.attributes() {
            let value = attr.value();
            let allowed = match attr.name() {
                "fill" | "stroke" => {
                    value.eq_ignore_ascii_case("none")
                        || value.eq_ignore_ascii_case("currentColor")
                        || value.strip_prefix('#').is_some_and(|hex| {
                            matches!(hex.len(), 3 | 4 | 6 | 8)
                                && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                }
                "width" | "height" if node == root => value
                    .strip_suffix("px")
                    .unwrap_or(value)
                    .parse::<f64>()
                    .is_ok_and(|n| (1.0..=f64::from(side)).contains(&n)),
                "viewBox" | "version" if node == root => true,
                "stroke-linecap" => matches!(value, "butt" | "round" | "square"),
                "stroke-linejoin" => matches!(value, "miter" | "round" | "bevel"),
                "fill-rule" => matches!(value, "evenodd" | "nonzero"),
                "d" | "points" | "transform" | "x" | "y" | "x1" | "x2" | "y1" | "y2" | "cx"
                | "cy" | "rx" | "ry" | "r" | "width" | "height" | "stroke-width" | "opacity"
                | "fill-opacity" | "stroke-opacity" => true,
                _ => false,
            };
            if attr.namespace().is_some() || !allowed {
                return Err(fail("SVG 功能图标包含不支持的属性或资源引用"));
            }
        }
    }
    // 用宿主实际的 SVG 解码器验证作者提供的尺寸和画布。
    egui_extras::image::load_svg_bytes(svg.as_bytes()).map_err(|e| fail(&e))
}

#[cfg(test)]
#[path = "appearance_assets_tests.rs"]
mod tests;

fn parse_expressions(source: &str) -> Result<crate::pet_expressions::ExpressionSheets> {
    use crate::pet_expressions::Expression;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Entry {
        expressions: BTreeMap<String, String>,
    }
    let Entry { mut expressions } = serde_json::from_str(source)?;
    if expressions.is_empty() {
        return Err(fail("表情包需要至少一个表情序列"));
    }
    let mut sheets = crate::pet_expressions::ExpressionSheets::default();
    for expression in Expression::ALL {
        if let Some(encoded) = expressions.remove(expression.key()) {
            let limit: usize = 64 * 1024;
            if encoded.len() > limit.div_ceil(3) * 4 {
                return Err(fail("每个表情的 PNG 序列图最多 64 KiB"));
            }
            let bytes = STANDARD.decode(encoded)?;
            if bytes.len() > limit {
                return Err(fail("每个表情的 PNG 序列图最多 64 KiB"));
            }
            if !bytes.ends_with(b"\0\0\0\0IEND\xaeB`\x82") {
                return Err(fail("表情序列必须是完整的 PNG 图片"));
            }
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(144);
            limits.max_image_height = Some(1440);
            limits.max_alloc = Some(4 * 1024 * 1024);
            let decoder = image::codecs::png::PngDecoder::with_limits(Cursor::new(&bytes), limits)?;
            if decoder.is_apng()? || decoder.dimensions() != (144, 1440) {
                return Err(fail("表情须为 144×1440 的单张 PNG 序列图"));
            }
            image::DynamicImage::from_decoder(decoder)?;
            sheets[expression as usize] = Some(bytes);
        }
    }
    if !expressions.is_empty() {
        return Err(fail("表情包包含未知的表情名称"));
    }
    Ok(sheets)
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Selection {
    skin: Option<String>,
    icon: Option<String>,
    pet: Option<String>,
}

impl Selection {
    const FILE: &str = "active-appearance.json";

    pub(super) fn read(dir: &std::path::Path) -> Self {
        bounded(&dir.join(Self::FILE), 512)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn get(&mut self, kind: Kind) -> Option<&mut Option<String>> {
        match kind {
            Kind::Skin => Some(&mut self.skin),
            Kind::Icon => Some(&mut self.icon),
            Kind::Pet => Some(&mut self.pet),
            Kind::Font => None,
        }
    }
}

impl LocalPlugins {
    pub(super) fn retain_available_appearance(&mut self) {
        for kind in Kind::SELECTABLE {
            if self.appearance_id(kind).is_none() {
                if let Some(slot) = self.appearance.get(kind) {
                    *slot = None;
                }
            }
        }
    }

    pub(super) fn forget_appearance(&mut self, id: &str) -> Result<()> {
        let mut selection = self.appearance.clone();
        let mut changed = false;
        for kind in Kind::SELECTABLE {
            if let Some(slot) = selection.get(kind).filter(|slot| slot.as_deref() == Some(id)) {
                *slot = None;
                changed = true;
            }
        }
        if changed {
            self.save_appearance(selection)?;
        }
        Ok(())
    }

    fn save_appearance(&mut self, selection: Selection) -> Result<()> {
        crate::model::atomic_write(
            &self.dir.join(Selection::FILE),
            &serde_json::to_vec(&selection)?,
        )?;
        self.appearance = selection;
        Ok(())
    }

    fn appearance_package(&self, id: &str, kind: Kind) -> Option<&super::Package> {
        let record = self.records.get(id)?;
        if !record.enabled {
            return None;
        }
        let package = record.package.as_ref()?;
        (Kind::of(&package.manifest) == Some(kind)).then_some(package)
    }

    pub(crate) fn appearance_id(&self, kind: Kind) -> Option<&str> {
        let id = match kind {
            Kind::Skin => &self.appearance.skin,
            Kind::Icon => &self.appearance.icon,
            Kind::Pet => &self.appearance.pet,
            Kind::Font => return None,
        }
        .as_deref()?;
        self.appearance_package(id, kind).map(|_| id)
    }

    pub(crate) fn appearance_choices(&self, kind: Kind) -> impl Iterator<Item = (&str, &str)> {
        self.records.keys().filter_map(move |id| {
            self.appearance_package(id, kind)
                .map(|package| (id.as_str(), package.manifest.name.as_str()))
        })
    }

    pub(crate) fn select_appearance(&mut self, kind: Kind, id: Option<&str>) -> Result<()> {
        if id.is_some_and(|id| self.appearance_package(id, kind).is_none()) {
            return Err(fail("所选外观资源包未安装或未启用"));
        }
        let mut selection = self.appearance.clone();
        let slot = selection.get(kind).ok_or_else(|| fail("画布字体随字体包启用"))?;
        if slot.as_deref() == id {
            return Ok(());
        }
        *slot = id.map(str::to_owned);
        self.save_appearance(selection)
    }

    fn active_appearance(&self, kind: Kind) -> Option<&Pack> {
        self.appearance_package(self.appearance_id(kind)?, kind)?
            .appearance
            .get()
    }

    pub(crate) fn active_skin(&self) -> Option<SkinPack> {
        match self.active_appearance(Kind::Skin)? {
            Pack::Skin(skin) => Some(*skin),
            _ => None,
        }
    }

    pub(crate) fn appearance_icon(
        &self,
        page: usize,
        sidebar: bool,
    ) -> Option<egui::ImageSource<'static>> {
        let Pack::Icon(icons) = self.active_appearance(Kind::Icon)? else {
            return None;
        };
        let icon = icons.get(*crate::app::PAGE_SHORTCUT_KEYS.get(page)?)?;
        let location = if sidebar { "sidebar" } else { "header" };
        Some(egui::ImageSource::Bytes {
            uri: format!("bytes://appearance-{location}-{}.svg", icon.hash).into(),
            bytes: icon.bytes.clone().into(),
        })
    }

    pub(crate) fn appearance_pet(&self) -> Option<Arc<crate::pet_expressions::ExpressionSheets>> {
        match self.active_appearance(Kind::Pet)? {
            Pack::Pet(sheets) => Some(sheets.clone()),
            _ => None,
        }
    }

    pub(crate) fn appearance_picker(
        &mut self,
        ui: &mut egui::Ui,
        kind: Kind,
        mut skin: Option<&mut Skin>,
    ) -> bool {
        #[derive(Clone, PartialEq)]
        enum Choice {
            BuiltIn(Option<Skin>),
            Plugin(String),
        }

        let mut choices = self.appearance_choices(kind).peekable();
        if choices.peek().is_none() {
            return false;
        }
        let built_in = skin.as_deref().copied();
        let current = self.appearance_id(kind);
        let title = current
            .and_then(|id| self.appearance_package(id, kind))
            .map_or_else(
                || built_in.map_or("内置", Skin::label),
                |pkg| pkg.manifest.name.as_str(),
            );
        let initial = current.map_or(Choice::BuiltIn(built_in), |id| Choice::Plugin(id.into()));
        let mut selection = initial.clone();
        let label = kind.label();
        crate::extras::settings_control_row(ui, label, 68.0, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(label)
                    .selected_text(title)
                    .width((ui.available_width() - 40.0).clamp(90.0, 220.0))
                    .truncate()
                    .show_ui(ui, |ui| {
                        if built_in.is_some() {
                            for option in Skin::ALL {
                                ui.selectable_value(
                                    &mut selection,
                                    Choice::BuiltIn(Some(option)),
                                    option.label(),
                                );
                            }
                        } else {
                            ui.selectable_value(&mut selection, Choice::BuiltIn(None), "内置");
                        }
                        ui.separator();
                        for (id, name) in choices {
                            ui.horizontal(|ui| {
                                ui.selectable_value(
                                    &mut selection,
                                    Choice::Plugin(id.into()),
                                    name,
                                );
                                super::ui::plugin_tag(ui);
                            });
                        }
                    });
                if current.is_some() {
                    super::ui::plugin_tag(ui);
                }
            })
            .response
        });
        let error_id = ui.make_persistent_id((label, "appearance-picker-error"));
        if initial != selection {
            let requested = match &selection {
                Choice::BuiltIn(_) => None,
                Choice::Plugin(id) => Some(id.as_str()),
            };
            match self.select_appearance(kind, requested) {
                Ok(()) => {
                    ui.data_mut(|data| data.remove::<String>(error_id));
                    if let (Choice::BuiltIn(Some(option)), Some(skin)) =
                        (selection, skin.as_deref_mut())
                    {
                        *skin = option;
                    }
                    ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                }
                Err(error) => ui.data_mut(|data| data.insert_temp(error_id, error.to_string())),
            }
        }
        if let Some(error) = ui.data(|data| data.get_temp::<String>(error_id)) {
            crate::extras::settings_hint(ui, &error);
        }
        true
    }
}
