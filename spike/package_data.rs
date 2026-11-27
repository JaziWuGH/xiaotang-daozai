use super::{cover::open_plain_file, fail, Result};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
};

pub(super) const DATA_FILE_LIMIT: u64 = 64 * 1024;
pub(super) const DATA_TOTAL_LIMIT: usize = 256 * 1024;
pub(super) type PackageData = BTreeMap<String, String>;

pub(super) fn valid_names(names: &[String]) -> bool {
    let mut unique = BTreeSet::new();
    names.iter().all(|name| {
        let Some(stem) = name.strip_suffix(".json") else {
            return false;
        };
        let first = stem
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        !stem.trim().is_empty()
            && !name.starts_with(['.', ' '])
            && !name
                .chars()
                .any(|ch| ch.is_control() || "/\\:*?\"<>|".contains(ch))
            && !matches!(
                first.as_str(),
                "CON"
                    | "PRN"
                    | "AUX"
                    | "NUL"
                    | "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            )
            && !name.eq_ignore_ascii_case("plugin.json")
            && unique.insert(name.to_lowercase())
    })
}

pub(super) fn read_file(dir: &Path, name: &str) -> Result<String> {
    let path = dir.join(name);
    let message = format!("数据文件 {name} 必须是包根目录中的普通文件");
    let file = open_plain_file(&path, &message)?;
    let mut bytes = Vec::new();
    file.take(DATA_FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > DATA_FILE_LIMIT {
        return Err(fail(&format!("数据文件 {name} 超过 64 KiB")));
    }
    String::from_utf8(bytes).map_err(|_| fail(&format!("数据文件 {name} 必须使用 UTF-8")))
}

pub(super) fn read_data(
    names: &[String],
    mut read: impl FnMut(&str) -> Result<String>,
) -> Result<PackageData> {
    let mut total = 0;
    let mut files = BTreeMap::new();
    for name in names {
        let text = read(name)?;
        total += text.len();
        if text.len() as u64 > DATA_FILE_LIMIT || total > DATA_TOTAL_LIMIT {
            return Err(fail("插件数据文件上限为单个 64 KiB、合计 256 KiB"));
        }
        if serde_json::from_str::<Value>(&text).is_err() {
            return Err(fail(&format!("数据文件 {name} 必须是有效的 JSON")));
        }
        files.insert(name.clone(), text);
    }
    Ok(files)
}

pub(super) fn valid_snapshot(names: &[String], files: &PackageData) -> bool {
    names.len() == files.len()
        && files.values().map(String::len).sum::<usize>() <= DATA_TOTAL_LIMIT
        && names.iter().all(|name| {
            files.get(name).is_some_and(|text| {
                text.len() as u64 <= DATA_FILE_LIMIT && serde_json::from_str::<Value>(text).is_ok()
            })
        })
}

pub(super) fn data_input(files: &PackageData) -> Result<BTreeMap<&str, Value>> {
    files
        .iter()
        .map(|(name, text)| Ok((name.as_str(), serde_json::from_str(text)?)))
        .collect()
}
