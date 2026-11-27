use serde_json::Value;

use super::UnavailableWindow;
use crate::quota::Window;

const FIVE_HOURS: i64 = 5 * 60 * 60;
const WEEKLY: i64 = 7 * 24 * 60 * 60;

pub(super) struct Usage {
    pub windows: Vec<Window>,
    pub unavailable: Vec<UnavailableWindow>,
}

fn report_data<'a>(report: &'a Value, name: &str) -> Option<&'a Value> {
    let command = report.get("command")?;
    if report["status"].as_str() != Some("SUCCESS")
        || report["num_turns"].as_u64() != Some(0)
        || report
            .pointer("/usage/total_tokens")
            .is_some_and(|value| value.as_u64() != Some(0))
        || command["name"].as_str() != Some(name)
    {
        return None;
    }
    command.get("data").filter(|data| data.is_object())
}

fn display_text(value: &Value) -> Option<&str> {
    let text = value.as_str()?.trim();
    (!text.is_empty() && text.chars().count() <= 128 && !text.chars().any(char::is_control))
        .then_some(text)
}

fn known_bucket(id: &str) -> Option<(&'static str, i64)> {
    match id {
        "gemini-5h" => Some(("Gemini", FIVE_HOURS)),
        "gemini-weekly" => Some(("Gemini", WEEKLY)),
        "3p-5h" => Some(("Claude/GPT", FIVE_HOURS)),
        "3p-weekly" => Some(("Claude/GPT", WEEKLY)),
        "gemini-argon-5h" => Some(("Argon", FIVE_HOURS)),
        "gemini-argon-wtu-weekly" | "gemini-argon-weekly" => Some(("Argon", WEEKLY)),
        _ => None,
    }
}

pub(super) fn usage(report: &Value) -> Result<Usage, String> {
    let invalid = "Antigravity CLI 未返回有效的额度报告";
    let data = report_data(report, "usage").ok_or(invalid)?;
    let groups = match data.get("groups") {
        None | Some(Value::Null) => &[][..],
        Some(groups) => groups.as_array().ok_or(invalid)?.as_slice(),
    };
    let mut windows = Vec::new();
    let mut unavailable = Vec::new();
    for group in groups {
        let Some(buckets) = group["buckets"].as_array() else {
            continue;
        };
        let group_name = display_text(&group["name"]);
        for bucket in buckets {
            let Some(id) = display_text(&bucket["id"]) else {
                continue;
            };
            let known = known_bucket(id);
            let cadence = display_text(&bucket["window"]);
            let seconds = match cadence {
                Some("5h") => Some(FIVE_HOURS),
                Some("weekly") => Some(WEEKLY),
                Some(_) => None,
                None => known.map(|(_, seconds)| seconds),
            };
            let period_label = match seconds {
                Some(FIVE_HOURS) => Some("5 小时"),
                Some(WEEKLY) => Some("本周"),
                _ => cadence,
            };
            let bucket_name = display_text(&bucket["name"]);
            let label = if let Some((pool, _)) = known {
                format!("{pool} {}", period_label.or(bucket_name).unwrap_or("额度"))
            } else if let Some(group_name) = group_name {
                let detail = bucket_name.or(period_label).unwrap_or(id);
                if group_name == detail {
                    group_name.to_owned()
                } else {
                    format!("{group_name} · {detail}")
                }
            } else {
                bucket_name.unwrap_or(id).to_owned()
            };
            let reset = bucket["reset_time"]
                .as_str()
                .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                .map(|time| time.timestamp())
                .filter(|&time| time > 0);
            let disabled = bucket["disabled"].as_bool() == Some(true);
            let remaining = bucket["remaining_fraction"]
                .as_f64()
                .filter(|fraction| !disabled && (0.0..=1.0).contains(fraction));

            if let Some(fraction) = remaining {
                // 转成 f32 时保留极小正数与真正耗尽的区别。
                let remaining = if fraction > 0.0 {
                    ((fraction * 100.0) as f32).max(f32::MIN_POSITIVE)
                } else {
                    0.0
                };
                windows.push(Window {
                    label,
                    seconds,
                    remaining,
                    reset,
                    amount: None,
                });
            } else {
                unavailable.push(UnavailableWindow {
                    label,
                    reset,
                    disabled,
                });
            }
        }
    }
    windows.sort_by(|left, right| {
        left.remaining.total_cmp(&right.remaining).then_with(|| {
            left.seconds
                .unwrap_or(i64::MAX)
                .cmp(&right.seconds.unwrap_or(i64::MAX))
        })
    });
    Ok(Usage {
        windows,
        unavailable,
    })
}

pub(super) fn credits(report: &Value) -> Option<f64> {
    report_data(report, "credits")?["remaining_credits"]
        .as_f64()
        .filter(|balance| balance.is_finite() && *balance >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(name: &str, data: Value) -> Value {
        json!({
            "status": "SUCCESS",
            "num_turns": 0,
            "usage": { "total_tokens": 0 },
            "command": { "name": name, "data": data }
        })
    }

    #[test]
    fn verified_report_without_groups_is_distinct_from_invalid_shape() {
        let result = usage(&report("usage", json!({"description": "No quota details"}))).unwrap();
        assert!(result.windows.is_empty());
        assert!(result.unavailable.is_empty());
        assert!(usage(&report("usage", json!({"groups": 0}))).is_err());
    }

    #[test]
    fn weekly_only_report_keeps_both_pools_without_inventing_windows() {
        let result = usage(&report(
            "usage",
            json!({
                "groups": [
                    {"name": "Gemini Models", "buckets": [
                        {"id": "gemini-weekly", "window": "weekly", "remaining_fraction": 0.875,
                         "reset_time": "2026-11-26T22:16:37Z"}
                    ]},
                    {"name": "Claude and GPT models", "buckets": [
                        {"id": "3p-weekly", "window": "weekly", "remaining_fraction": 0.625}
                    ]}
                ]
            }),
        ))
        .unwrap();

        assert_eq!(result.windows.len(), 2);
        assert_eq!(result.windows[0].label, "Claude/GPT 本周");
        assert_eq!(result.windows[0].remaining, 62.5);
        assert_eq!(result.windows[0].reset, None);
        assert_eq!(result.windows[1].label, "Gemini 本周");
        assert_eq!(result.windows[1].seconds, Some(WEEKLY));
        assert_eq!(result.windows[1].remaining, 87.5);
        assert_eq!(result.windows[1].reset, Some(1_795_731_397));
        assert!(result.unavailable.is_empty());
    }

    #[test]
    fn keeps_distinct_model_pools_and_sorts_urgent_windows_first() {
        let result = usage(&report(
            "usage",
            json!({
                "groups": [
                    {"name": "Gemini Models", "buckets": [
                        {"id": "gemini-weekly", "window": "weekly", "remaining_fraction": 0.25},
                        {"id": "gemini-5h", "window": "5h", "remaining_fraction": 0.25}
                    ]},
                    {"name": "Claude and GPT models", "buckets": [
                        {"id": "3p-weekly", "window": "weekly", "remaining_fraction": 0.0},
                        {"id": "3p-5h", "window": "5h", "remaining_fraction": 1.0}
                    ]},
                    {"name": "Gemini 4 Argon", "buckets": [
                        {"id": "gemini-argon-wtu-weekly", "remaining_fraction": 0.1}
                    ]},
                    {"name": "Future Pool", "buckets": [
                        {"id": "future-monthly", "name": "Monthly Allowance", "window": "monthly",
                         "remaining_fraction": 0.125}
                    ]}
                ]
            }),
        ))
        .unwrap();

        let labels: Vec<_> = result
            .windows
            .iter()
            .map(|window| window.label.as_str())
            .collect();
        assert_eq!(
            labels,
            [
                "Claude/GPT 本周",
                "Argon 本周",
                "Future Pool · Monthly Allowance",
                "Gemini 5 小时",
                "Gemini 本周",
                "Claude/GPT 5 小时"
            ]
        );
        assert_eq!(result.windows[0].remaining, 0.0);
        assert_eq!(result.windows[2].seconds, None);
        assert_eq!(result.windows[3].seconds, Some(FIVE_HOURS));
    }

    #[test]
    fn unavailable_buckets_keep_reset_without_fabricating_zero() {
        let result = usage(&report(
            "usage",
            json!({
                "groups": [{"buckets": [
                    {"id": "disabled", "disabled": true, "remaining_fraction": 0,
                     "reset_time": "2026-11-27T06:16:37+08:00"},
                    {"id": "missing"},
                    {"id": "invalid", "remaining_fraction": 1.2, "reset_time": "invalid"},
                    {"id": "tiny", "remaining_fraction": 1e-100}
                ]}]
            }),
        ))
        .unwrap();

        assert_eq!(result.windows.len(), 1);
        assert!(result.windows[0].remaining > 0.0);
        assert_eq!(result.unavailable.len(), 3);
        assert_eq!(result.unavailable[0].label, "disabled");
        assert!(result.unavailable[0].disabled);
        assert_eq!(result.unavailable[0].reset, Some(1_795_731_397));
        assert!(!result.unavailable[1].disabled);
        assert_eq!(result.unavailable[2].reset, None);
    }

    #[test]
    fn accepts_empty_usage_but_rejects_wrong_command_or_model_output() {
        let data = json!({"groups": []});
        let mut input = report("usage", data.clone());
        assert!(usage(&input).unwrap().windows.is_empty());
        assert!(usage(&report("credits", data)).is_err());
        input["num_turns"] = json!(1);
        assert!(usage(&input).is_err());
        input["num_turns"] = json!(0);
        input["usage"]["total_tokens"] = json!(1);
        assert!(usage(&input).is_err());
        input["usage"]["total_tokens"] = json!(0);
        input["status"] = json!("ERROR");
        assert!(usage(&input).is_err());
    }

    #[test]
    fn credits_only_accepts_the_explicit_remaining_balance() {
        assert_eq!(
            credits(&report("credits", json!({"remaining_credits": 420.5}))),
            Some(420.5)
        );
        assert_eq!(
            credits(&report("credits", json!({"remaining_credits": 0}))),
            Some(0.0)
        );
        assert_eq!(
            credits(&report("credits", json!({"remaining_credits": -1}))),
            None
        );
        assert_eq!(credits(&report("credits", json!({"balance": 999}))), None);
        assert_eq!(
            credits(&report("usage", json!({"remaining_credits": 999}))),
            None
        );
    }
}
