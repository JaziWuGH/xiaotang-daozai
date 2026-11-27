use super::{fail, Field, Result, Values};
use crate::plugin_setting_catalog::{icon_path, setting_color};
use chrono::{DateTime, Local, Offset, Utc};
use chrono_tz::{OffsetComponents, Tz};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Choice {
    pub(super) value: String,
    pub(super) label: String,
}

pub(super) fn valid_fields(fields: &[Field], material: bool) -> bool {
    valid_field_group(fields, material, false)
}

fn valid_field_group(fields: &[Field], material: bool, list_children: bool) -> bool {
    let mut keys = BTreeSet::new();
    fields.iter().all(|field| {
        (!material || field.kind == "number")
            && (list_children || !matches!(field.kind.as_str(), "icon" | "color"))
            && !field.label.trim().is_empty()
            && field.label.chars().count() <= 80
            && field.key.starts_with(|c: char| c.is_ascii_lowercase())
            && field.key.len() <= 40
            && field
                .key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
            && keys.insert(&field.key)
            && valid_field(field, &field.default)
    })
}

pub(super) fn weekdays(value: &str) -> BTreeSet<u8> {
    value
        .split(',')
        .filter_map(|day| day.parse().ok())
        .collect()
}

pub(super) fn valid_field(field: &Field, value: &str) -> bool {
    if field.kind != "number"
        && [field.min, field.max, field.step]
            .iter()
            .any(Option::is_some)
        || field.kind != "number" && (field.unit.is_some() || field.precision.is_some())
        || field.kind != "choice" && field.choices.is_some()
        || field.kind != "list" && (field.item.is_some() || field.max_items.is_some())
        || field.kind != "text" && field.multiline.is_some()
    {
        return false;
    }
    match field.kind.as_str() {
        "text" => {
            !value.trim().is_empty()
                && value.chars().count() <= 80
                && (field.multiline != Some(true)
                    || value.chars().all(|ch| ch == '\n' || !ch.is_control()))
        }
        "date" => chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .is_ok_and(|date| date.to_string() == value),
        "switch" => matches!(value, "true" | "false"),
        "choice" => field.choices.as_ref().is_some_and(|choices| {
            let mut values = BTreeSet::new();
            (2..=6).contains(&choices.len())
                && choices.iter().all(|choice| {
                    !choice.value.trim().is_empty()
                        && choice.value.chars().count() <= 80
                        && !choice.label.trim().is_empty()
                        && choice.label.chars().count() <= 80
                        && values.insert(choice.value.as_str())
                })
                && values.contains(value)
        }),
        "number" => {
            let (Some(min), Some(max), Some(step)) = (field.min, field.max, field.step) else {
                return false;
            };
            let width = max - min;
            width.is_finite()
                && step > 0.0
                && step <= width
                && field
                    .unit
                    .as_ref()
                    .is_none_or(|unit| unit.chars().count() <= 4)
                && field.precision.is_none_or(|precision| precision <= 3)
                && value
                    .parse::<f32>()
                    .is_ok_and(|number| number.is_finite() && (min..=max).contains(&number))
        }
        "time" => chrono::NaiveTime::parse_from_str(value, "%H:%M")
            .is_ok_and(|time| time.format("%H:%M").to_string() == value),
        "timezone" => value.parse::<Tz>().is_ok(),
        "icon" => icon_path(value).is_some(),
        "color" => setting_color(value).is_some(),
        "weekdays" => {
            value.is_empty() || {
                let mut days = BTreeSet::new();
                value.split(',').all(|day| {
                    matches!(day, "1" | "2" | "3" | "4" | "5" | "6" | "7") && days.insert(day)
                })
            }
        }
        "list" => {
            let (Some(fields), Some(max)) = (&field.item, field.max_items) else {
                return false;
            };
            (1..=20).contains(&max)
                && (1..=7).contains(&fields.len())
                && fields.iter().all(|child| {
                    matches!(
                        child.kind.as_str(),
                        "text" | "choice" | "number" | "date" | "time" | "switch" | "timezone"
                            | "icon" | "color"
                    )
                })
                && valid_field_group(fields, false, true)
                && serde_json::from_str::<Vec<Values>>(value).is_ok_and(|rows| {
                    rows.len() <= max
                        && rows.iter().all(|row| {
                            row.len() == fields.len()
                                && fields.iter().all(|child| {
                                    row.get(&child.key)
                                        .is_some_and(|item| valid_field(child, item))
                                })
                        })
                })
        }
        _ => false,
    }
}

pub(super) fn time_input(
    input: &mut Value,
    now: DateTime<Utc>,
    zones: bool,
    fields: &[Field],
    settings: &Values,
) {
    let local = now.with_timezone(&Local);
    let offset = local.offset().local_minus_utc() / 60;
    input["now"] = json!(now.timestamp_millis());
    input["tz"] = json!({"offset_minutes": offset, "name": iana_time_zone::get_timezone()
        .unwrap_or_else(|_| local.offset().to_string())});
    if zones {
        let mut offsets = BTreeMap::new();
        let mut insert_zone = |name: &str| {
            if let Ok(tz) = name.parse::<Tz>() {
                let localized = now.with_timezone(&tz);
                let offset = localized.offset();
                offsets.insert(
                    name.to_owned(),
                    json!({
                        "offset_minutes": offset.fix().local_minus_utc() / 60,
                        "dst": offset.dst_offset().num_seconds() != 0,
                    }),
                );
            }
        };
        for field in fields {
            let Some(value) = settings.get(&field.key) else {
                continue;
            };
            if field.kind == "list" {
                if let Ok(rows) = serde_json::from_str::<Vec<Values>>(value) {
                    for row in rows {
                        row.values().for_each(|item| insert_zone(item));
                    }
                }
            } else {
                insert_zone(value);
            }
        }
        input["zones"] = json!(offsets);
    }
}

pub(super) fn next_delay(now: i64, requested: i64, interval: u32) -> Duration {
    Duration::from_secs(requested.saturating_sub(now).clamp(60, i64::from(interval)) as u64)
}

const STORAGE_LIMIT: usize = 256 * 1024;

pub(super) fn valid_storage(storage: &Value) -> bool {
    serde_json::to_vec(storage).is_ok_and(|bytes| bytes.len() <= STORAGE_LIMIT)
}

pub(super) fn split_output(raw: &str) -> Result<(Value, Value)> {
    const OUTPUT_LIMIT: usize = 16 * 1024;
    const CANVAS_OUTPUT_LIMIT: usize = 64 * 1024;
    if raw.len() > CANVAS_OUTPUT_LIMIT + STORAGE_LIMIT + 16 {
        return Err(fail("插件输出或私有存储超过大小限制（画布 64 KiB / 私有存储 256 KiB）"));
    }
    let mut output: Value = serde_json::from_str(raw)?;
    let storage = output
        .as_object_mut()
        .ok_or_else(|| fail("插件输出必须为对象"))?
        .remove("storage")
        .unwrap_or(Value::Null);
    if !valid_storage(&storage) {
        return Err(fail("私有存储已满，上限为 256 KiB"));
    }
    let canvas = !output["canvas"].is_null() || !output["pinned"]["canvas"].is_null();
    let limit = if canvas { CANVAS_OUTPUT_LIMIT } else { OUTPUT_LIMIT };
    if serde_json::to_vec(&output)?.len() > limit {
        return Err(fail(if canvas { "插件画面有误：插件输出超过 64 KiB" } else { "插件输出超过 16 KiB" }));
    }
    Ok((output, storage))
}
