//! 画布专用字体资源的声明、校验和安装快照。

use super::{cover::open_plain_file, fail, LocalPlugins, Manifest, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use eframe::egui;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
    sync::Arc,
};

const FACE_LIMIT: u64 = 8 * 1024 * 1024;
pub(super) const PACKAGE_LIMIT: u64 = 16 * 1024 * 1024;
pub(super) const SNAPSHOT_LIMIT: u64 = 24 * 1024 * 1024;
pub(super) const LICENSES: [&str; 2] = ["LICENSE", "OFL.txt"];

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    fonts: Vec<Face>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Face {
    name: String,
    file: String,
}

impl Entry {
    pub(super) fn decode(source: &str) -> Result<Self> {
        let entry: Self = serde_json::from_str(source)?;
        let mut names = BTreeSet::new();
        let mut files = BTreeSet::new();
        if !(1..=4).contains(&entry.fonts.len())
            || entry.fonts.iter().any(|face| {
                face.name.trim().is_empty()
                    || face.name.chars().any(char::is_control)
                    || !names.insert(&face.name)
                    || !files.insert(face.file.to_lowercase())
                    || !valid_face_path(&face.file)
            })
        {
            return Err(fail(
                "字体包需要 1 至 4 个名称和文件各不相同的包内 TTF 或 OTF 字体",
            ));
        }
        Ok(entry)
    }

    pub(super) fn files(&self) -> impl Iterator<Item = &str> {
        self.fonts.iter().map(|face| face.file.as_str())
    }
}

fn valid_face_path(name: &str) -> bool {
    !name.contains("..")
        && name.split('/').all(|part| {
            !part.is_empty()
                && !part.ends_with(['.', ' '])
                && !part
                    .chars()
                    .any(|ch| ch.is_control() || "\\:<>\"|?*".contains(ch))
        })
        && Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf"))
}

#[derive(Clone)]
pub(super) struct Blob(pub(super) Arc<egui::FontData>);

impl Blob {
    pub(super) fn bytes(&self) -> &[u8] {
        self.0.font.as_ref()
    }

    fn is_font(&self) -> bool {
        let bytes = self.bytes();
        (bytes.starts_with(&[0, 1, 0, 0])
            || bytes.starts_with(b"OTTO")
            || bytes.starts_with(b"true"))
            && ab_glyph::FontRef::try_from_slice(bytes).is_ok()
    }
}

impl Serialize for Blob {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(self.bytes()))
    }
}

impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error;
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() as u64 > PACKAGE_LIMIT.div_ceil(3) * 4 {
            return Err(Error::custom("字体包资源超过 16 MiB"));
        }
        let bytes = STANDARD.decode(encoded).map_err(Error::custom)?;
        Ok(Self(Arc::new(egui::FontData::from_owned(bytes))))
    }
}

pub(super) type Bundle = BTreeMap<String, Blob>;

pub(super) fn base_size(manifest: &Manifest, source: &str, cover_size: usize) -> Result<u64> {
    Ok((serde_json::to_vec(manifest)?.len() + source.len() + cover_size) as u64)
}

pub(super) fn read_local(entry: &Entry, base: &Path, used: u64) -> Result<Bundle> {
    let licenses = LICENSES.into_iter().filter(|name| base.join(name).exists());
    read(entry, licenses, used, |name, limit| {
        let mut bytes = Vec::new();
        open_plain_file(&base.join(name), "字体和许可文件必须是包内普通文件")?
            .take(limit + 1)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

pub(super) fn read(
    entry: &Entry,
    licenses: impl Iterator<Item = &'static str>,
    used: u64,
    mut load: impl FnMut(&str, u64) -> Result<Vec<u8>>,
) -> Result<Bundle> {
    let licenses: Vec<_> = licenses.collect();
    if licenses.is_empty() {
        return Err(fail("字体包缺少 LICENSE 或 OFL.txt"));
    }
    let mut remaining = PACKAGE_LIMIT
        .checked_sub(used)
        .ok_or_else(|| fail("字体包超过 16 MiB"))?;
    let mut files = BTreeMap::new();
    for (name, is_font) in entry
        .files()
        .map(|name| (name, true))
        .chain(licenses.into_iter().map(|name| (name, false)))
    {
        let limit = if is_font {
            FACE_LIMIT.min(remaining)
        } else {
            remaining
        };
        let bytes = load(name, limit)?;
        if bytes.len() as u64 > limit {
            return Err(fail("单个字体最多 8 MiB，整包最多 16 MiB"));
        }
        remaining -= bytes.len() as u64;
        let blob = Blob(Arc::new(egui::FontData::from_owned(bytes)));
        if is_font && !blob.is_font() {
            return Err(fail(&format!(
                "无法读取字体 {name}，请使用有效的 TTF 或 OTF 文件"
            )));
        }
        files.insert(name.to_owned(), blob);
    }
    Ok(files)
}

pub(super) fn valid_snapshot(entry: &Entry, files: &Bundle, used: u64) -> bool {
    LICENSES.iter().any(|name| files.contains_key(*name))
        && files.iter().all(|(name, blob)| {
            LICENSES.contains(&name.as_str())
                || (entry.files().any(|file| file == name)
                    && blob.bytes().len() as u64 <= FACE_LIMIT
                    && blob.is_font())
        })
        && entry.files().all(|file| files.contains_key(file))
        && used
            + files
                .values()
                .map(|blob| blob.bytes().len() as u64)
                .sum::<u64>()
            <= PACKAGE_LIMIT
}

impl LocalPlugins {
    pub(crate) fn sync_canvas_fonts(&self, ctx: &egui::Context) {
        let mut active = BTreeMap::new();
        for record in self.records.values().filter(|record| record.enabled) {
            let Some(package) = &record.package else {
                continue;
            };
            let Some(entry) = package.font_entry() else {
                continue;
            };
            for face in &entry.fonts {
                if let Some(blob) = package.font_assets.get(&face.file) {
                    active
                        .entry(format!("plugin:{}", face.name))
                        .or_insert_with(|| blob.0.clone());
                }
            }
        }
        crate::theme::sync_canvas_fonts(ctx, active);
    }
}
