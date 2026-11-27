//! 插件在额度面板上的只读展示数据。

const MAX_TIMESTAMP: i64 = 253_402_300_799;
const MAX_PERIOD_SECONDS: i64 = 366 * 24 * 60 * 60;
const MAX_AMOUNT: f64 = 1_000_000_000_000_000.0;

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PluginQuota {
    pub windows: Vec<PluginQuotaWindow>,
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PluginQuotaWindow {
    pub label: String,
    pub remaining_pct: Option<f64>,
    pub remaining: Option<f64>,
    pub limit: Option<f64>,
    pub unit: Option<String>,
    pub period_secs: Option<i64>,
    pub reset_at: Option<i64>,
}

impl PluginQuota {
    pub(crate) fn valid(&self) -> bool {
        (1..=3).contains(&self.windows.len())
            && self.expires_at.is_none_or(valid_timestamp)
            && self.windows.iter().all(PluginQuotaWindow::valid)
    }
}

impl PluginQuotaWindow {
    fn valid(&self) -> bool {
        if !valid_text(&self.label, 40)
            || !self
                .period_secs
                .is_none_or(|seconds| (1..=MAX_PERIOD_SECONDS).contains(&seconds))
            || !self.reset_at.is_none_or(valid_timestamp)
        {
            return false;
        }
        match (self.remaining_pct, self.remaining, self.unit.as_deref()) {
            (Some(percent), None, None) => self.limit.is_none() && (0.0..=100.0).contains(&percent),
            (None, Some(remaining), Some(unit)) => {
                valid_text(unit, 12)
                    && (0.0..=MAX_AMOUNT).contains(&remaining)
                    && self.limit.is_none_or(|limit| {
                        (0.0..=MAX_AMOUNT).contains(&limit) && limit > 0.0 && remaining <= limit
                    })
            }
            _ => false,
        }
    }

    pub(crate) fn value_text(&self) -> String {
        if let Some(remaining) = self.remaining {
            let amount = match self.limit {
                Some(limit) => format!("{} / {}", number_text(remaining), number_text(limit)),
                None => number_text(remaining),
            };
            return format!(
                "{amount} {}",
                self.unit.as_deref().unwrap_or_default().trim()
            );
        }
        match self.remaining_pct {
            Some(percent) if (0.0..1.0).contains(&percent) && percent > 0.0 => "<1%".into(),
            Some(percent) => format!("{percent:.0}%"),
            None => "—".into(),
        }
    }

    pub(crate) fn percent(&self) -> Option<f32> {
        self.remaining_pct
            .or_else(|| {
                self.remaining
                    .zip(self.limit)
                    .map(|(remaining, limit)| remaining / limit * 100.0)
            })
            .map(|percent| percent as f32)
    }
}

fn valid_text(text: &str, max_chars: usize) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= max_chars
        && !text.chars().any(char::is_control)
}

fn valid_timestamp(timestamp: i64) -> bool {
    (0..=MAX_TIMESTAMP).contains(&timestamp)
}

fn number_text(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if !(0.0001..1.0e12).contains(&value) {
        return format!("{value:.3e}");
    }
    format!("{value:.4}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plugin_quota_displays_percentage_and_preserves_native_units() {
        let quota: PluginQuota = serde_json::from_value(json!({
            "expires_at": 1893456000,
            "windows": [
                {"label": "本周", "remaining_pct": 0.25, "period_secs": 604800},
                {"label": "剩余额度", "remaining": 125.5, "limit": 250, "unit": "积分"},
                {"label": "充值余额", "remaining": 32.25, "unit": "美元"}
            ]
        }))
        .unwrap();
        assert!(quota.valid());
        assert_eq!(quota.windows[0].value_text(), "<1%");
        assert_eq!(quota.windows[0].percent(), Some(0.25));
        assert_eq!(quota.windows[1].value_text(), "125.5 / 250 积分");
        assert_eq!(quota.windows[1].percent(), Some(50.2));
        assert_eq!(quota.windows[2].value_text(), "32.25 美元");
        assert_eq!(quota.windows[2].percent(), None);
    }

    #[test]
    fn plugin_quota_rejects_invalid_numbers_labels_and_dates() {
        for window in [
            json!({"label": "本周", "remaining_pct": 101}),
            json!({"label": "本周", "remaining_pct": 50, "remaining": 10, "unit": "点"}),
            json!({"label": "本周", "remaining": 8, "limit": 7, "unit": "点"}),
            json!({"label": "本周", "remaining": 0, "limit": 0, "unit": "点"}),
            json!({"label": "本周", "remaining": -1, "unit": "点"}),
            json!({"label": "本周", "remaining": 8}),
            json!({"label": "本周", "remaining": 8, "unit": " "}),
            json!({"label": "注入\n标题", "remaining_pct": 35}),
            json!({"label": "本周", "remaining_pct": 35, "reset_at": i64::MAX}),
            json!({"label": "本周", "remaining_pct": 35, "period_secs": 0}),
        ] {
            let quota: PluginQuota = serde_json::from_value(json!({"windows": [window]})).unwrap();
            assert!(!quota.valid(), "{quota:?}");
        }
        let window = json!({"label":"本周","remaining_pct":35});
        let quota: PluginQuota = serde_json::from_value(json!({
            "windows":[window.clone(),window.clone(),window.clone(),window.clone()]
        }))
        .unwrap();
        assert!(!quota.valid());
        let quota: PluginQuota = serde_json::from_value(json!({
            "windows":[window], "expires_at": -1
        }))
        .unwrap();
        assert!(!quota.valid());
        assert!(serde_json::from_value::<PluginQuota>(json!({
            "windows":[{"label":"本周","remaining_pct":35,"account_id":"native"}]
        }))
        .is_err());
    }
}
