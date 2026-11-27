use super::{
    appearance_assets,
    cover::{open_plain_file, COVER_LIMIT},
    fail, Result, COVER_FILE,
};
use crate::plugin_canvas::valid_image_path;
use base64::{engine::general_purpose::STANDARD, Engine};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read},
    path::Path,
    sync::{Arc, OnceLock},
};

pub(super) const IMAGE_LIMIT: u64 = 512 * 1024;
const SIDE_LIMIT: u32 = 1024;
pub(super) type CanvasImages = BTreeMap<String, CanvasImage>;

#[derive(Clone)]
pub(super) struct CanvasImage {
    bytes: Arc<[u8]>,
    texture: Arc<OnceLock<egui::TextureHandle>>,
}

impl CanvasImage {
    pub(super) fn checked(name: &str, bytes: Vec<u8>) -> Result<Self> {
        let image = Self {
            bytes: bytes.into(),
            texture: Arc::new(OnceLock::new()),
        };
        image.decode(name)?;
        Ok(image)
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn decode(&self, name: &str) -> Result<egui::ColorImage> {
        if Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
        {
            return appearance_assets::load_static_svg(
                std::str::from_utf8(&self.bytes)?,
                IMAGE_LIMIT as usize,
                SIDE_LIMIT,
            );
        }
        let mut reader =
            image::ImageReader::with_format(Cursor::new(&self.bytes), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(SIDE_LIMIT);
        limits.max_image_height = Some(SIDE_LIMIT);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let rgba = reader.decode()?.into_rgba8();
        Ok(egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        ))
    }

    pub(super) fn texture(
        &self,
        ctx: &egui::Context,
        name: &str,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        if self.texture.get().is_none() {
            let decoded = self.decode(name).ok()?;
            let _ = self
                .texture
                .set(ctx.load_texture(name, decoded, egui::TextureOptions::LINEAR));
        }
        self.texture
            .get()
            .map(|texture| (texture.id(), texture.size_vec2()))
    }
}

impl Serialize for CanvasImage {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(&self.bytes))
    }
}

impl<'de> Deserialize<'de> for CanvasImage {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        use serde::de::Error;
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() as u64 > IMAGE_LIMIT.div_ceil(3) * 4 {
            return Err(Error::custom("插件画布图片超过 512 KiB"));
        }
        let bytes = STANDARD.decode(encoded).map_err(Error::custom)?;
        Ok(Self {
            bytes: bytes.into(),
            texture: Arc::new(OnceLock::new()),
        })
    }
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png") || ext.eq_ignore_ascii_case("svg"))
}

pub(super) fn names(base: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let mut folders = vec![base.to_owned()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() && path.canonicalize()? == path {
                folders.push(path);
            } else if is_image(&path) && path.is_file() {
                let name = path
                    .strip_prefix(base)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !valid_image_path(&name) {
                    return Err(fail(&format!("插件画布图片路径无效：{name}")));
                }
                names.push(name);
            }
        }
    }
    Ok(names)
}

pub(super) fn read(base: &Path) -> Result<CanvasImages> {
    decode_all(names(base)?, |name, remaining| {
        let mut bytes = Vec::new();
        open_plain_file(&base.join(name), "插件画布图片必须是包内普通文件")?
            .take(remaining + 1)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

pub(super) fn decode_all(
    mut names: Vec<String>,
    mut load: impl FnMut(&str, u64) -> Result<Vec<u8>>,
) -> Result<CanvasImages> {
    let mut remaining = IMAGE_LIMIT;
    let mut images = BTreeMap::new();
    // 封面是集市的独立资源，先给专用画布图片留出额度。
    names.sort_by_key(|name| name == COVER_FILE);
    for name in names {
        if !valid_image_path(&name) {
            return Err(fail(&format!("插件画布图片路径无效：{name}")));
        }
        let limit = if name == COVER_FILE {
            COVER_LIMIT
        } else {
            remaining
        };
        let bytes = load(&name, limit)?;
        if bytes.len() as u64 > limit {
            return Err(fail("插件画布图片合计超过 512 KiB"));
        }
        if name == COVER_FILE && bytes.len() as u64 > remaining {
            continue;
        }
        let image = match CanvasImage::checked(&name, bytes) {
            Ok(image) => image,
            // 合规的集市封面可能超过画布图元的尺寸。
            Err(_) if name == COVER_FILE => continue,
            Err(error) => return Err(error),
        };
        remaining -= image.bytes().len() as u64;
        images.insert(name, image);
    }
    Ok(images)
}

pub(super) fn valid_snapshot(images: &CanvasImages) -> bool {
    images
        .values()
        .map(|image| image.bytes.len() as u64)
        .sum::<u64>()
        <= IMAGE_LIMIT
        && images
            .iter()
            .all(|(name, image)| valid_image_path(name) && image.decode(name).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_images_check_dimensions_and_total_bytes() {
        let png = |width| {
            let mut output = Cursor::new(Vec::new());
            image::DynamicImage::new_rgba8(width, 1)
                .write_to(&mut output, image::ImageFormat::Png)
                .unwrap();
            output.into_inner()
        };
        assert!(CanvasImage::checked("image.png", png(1024)).is_ok());
        assert!(CanvasImage::checked("image.png", png(1025)).is_err());

        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#000"/></svg>"##;
        let mut bytes = svg.to_vec();
        bytes.resize(IMAGE_LIMIT as usize / 2 + 1, b' ');
        assert!(decode_all(vec!["first.svg".into()], |_, _| Ok(bytes.clone())).is_ok());
        assert!(
            decode_all(vec!["first.svg".into(), "second.svg".into()], |_, _| Ok(
                bytes.clone()
            ))
            .is_err()
        );
        assert!(decode_all(vec!["../other.svg".into()], |_, _| panic!(
            "越界路径不得交给读取器"
        ))
        .is_err());
    }
}
