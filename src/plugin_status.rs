//! 插件详情页的数据与布局契约。

use serde::{Deserialize, Serialize};

#[path = "plugin_status_realtime.rs"]
mod realtime;
pub(crate) use realtime::{CountdownFormat, StatusClock, StatusCountdown, StatusCounter};

pub(super) const MAX_ITEMS: usize = 8;
const MAX_REALTIME: usize = 4;
const MAX_COMPONENTS: usize = 10;
const MAX_TIMESTAMP: i64 = 253_402_300_799;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusHealth {
    Waiting,
    Unconfigured,
    Offline,
    Stale,
    Invalid,
    CanvasInvalid,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StatusState {
    Running,
    Paused,
    Succeeded,
    Error,
    Offline,
    Idle,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PluginStatus {
    pub state: StatusState,
    pub label: String,
    pub summary: Option<String>,
    pub progress_pct: Option<f64>,
    #[serde(default)]
    pub metrics: Vec<StatusMetric>,
    #[serde(default)]
    pub items: Vec<StatusItem>,
    pub series: Option<Vec<f64>>,
    pub series_label: Option<String>,
    pub note: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, deserialize_with = "realtime::one_or_many")]
    pub counter: Vec<StatusCounter>,
    #[serde(default, deserialize_with = "realtime::one_or_many")]
    pub clock: Vec<StatusClock>,
    #[serde(default, deserialize_with = "realtime::one_or_many")]
    pub countdown: Vec<StatusCountdown>,
    pub alert: Option<String>,
    pub observed_at: Option<i64>,
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub actions: Vec<StatusAction>,
    #[serde(default)]
    pub fields: Vec<StatusField>,
    #[serde(skip)]
    pub canvas: Option<std::sync::Arc<crate::plugin_canvas::Canvas>>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusMetric {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusItem {
    pub label: String,
    pub detail: Option<String>,
    pub state: Option<StatusState>,
    pub action: Option<String>,
    pub open: Option<String>,
    pub copy: Option<String>,
    #[serde(default)]
    pub sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusAction {
    pub id: String,
    pub label: String,
    pub open: Option<String>,
    pub copy: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusField {
    pub key: String,
    pub label: String,
    pub value: Option<String>,
    pub max: usize,
    #[serde(default)]
    pub multiline: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusLayout {
    pub components: Vec<StatusComponent>,
    #[serde(default)]
    pub style: StatusStyle,
    #[serde(rename = "canvasFields", default, skip_serializing_if = "Vec::is_empty")]
    pub canvas_fields: Vec<CanvasField>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CanvasField {
    pub key: String,
    pub rect: [u16; 4],
    pub placeholder: Option<String>,
    pub action: Option<String>,
}

impl StatusLayout {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if !(1..=MAX_COMPONENTS).contains(&self.components.len()) {
            return Err("状态页布局需要 1 至 10 个组件");
        }
        for (index, component) in self.components.iter().enumerate() {
            let limit = if component.is_realtime() { MAX_REALTIME } else { 1 };
            if self.occurrence(index) >= limit {
                return Err(if limit == 1 {
                    "状态页布局中同一普通组件只能出现一次"
                } else {
                    "状态页布局中同一实时组件最多出现 4 次"
                });
            }
            if matches!(component, StatusComponent::Metrics { columns } if !(1..=2).contains(columns))
            {
                return Err("状态页指标只能使用 1 列或 2 列");
            }
        }
        if self.style.accent.is_some() && self.style.accent_rgb().is_none() {
            return Err("状态页强调色需为 #RRGGBB 格式");
        }
        if self.canvas_fields.len() > 3
            || !self.canvas_fields.is_empty() && !self.components.contains(&StatusComponent::Canvas {})
        {
            return Err("画布输入框需要 canvas 组件，最多 3 个");
        }
        for (index, field) in self.canvas_fields.iter().enumerate() {
            let [x, y, w, h] = field.rect.map(u32::from);
            if w == 0 || h == 0 || x + w > 387 || y + h > 360
                || !valid_control_id(&field.key)
                || self.canvas_fields[..index].iter().any(|other| other.key == field.key)
                || field.action.as_deref().is_some_and(|id| !valid_control_id(id))
                || field.placeholder.as_deref().is_some_and(|text| !valid_text(text, 80))
            {
                return Err("画布输入框的键、位置、提示或动作无效");
            }
        }
        Ok(())
    }

    pub(super) fn active_canvas_fields<'a>(
        &'a self,
        status: &'a PluginStatus,
    ) -> impl Iterator<Item = &'a CanvasField> {
        self.canvas_fields.iter().filter(|anchor| {
            status.canvas.as_ref().is_some_and(|canvas| {
                f32::from(anchor.rect[1]) + f32::from(anchor.rect[3]) <= canvas.height
                    && status.fields.iter().any(|field| field.key == anchor.key)
                    && anchor.action.as_ref().is_none_or(|id| status.actions.iter().any(|action| action.id == *id))
            })
        })
    }

    pub(super) fn occurrence(&self, index: usize) -> usize {
        let kind = std::mem::discriminant(&self.components[index]);
        self.components[..index]
            .iter()
            .filter(|component| std::mem::discriminant(*component) == kind)
            .count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StatusComponent {
    State {},
    Summary {},
    Progress {
        #[serde(default)]
        variant: ProgressVariant,
    },
    Metrics {
        #[serde(default = "default_metric_columns")]
        columns: u8,
    },
    Alert {},
    UpdatedAt {},
    Actions {},
    Fields {},
    List {},
    Chart {},
    Text {},
    Badge {},
    Counter {},
    Clock {},
    Countdown {},
    Canvas {},
}

impl StatusComponent {
    fn is_realtime(self) -> bool {
        matches!(self, Self::Counter {} | Self::Clock {} | Self::Countdown {})
    }
}

fn default_metric_columns() -> u8 {
    2
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProgressVariant {
    #[default]
    Ring,
    Bar,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusStyle {
    #[serde(
        default,
        deserialize_with = "present_value",
        skip_serializing_if = "Option::is_none"
    )]
    accent: Option<String>,
    #[serde(default)]
    pub density: StatusDensity,
    #[serde(default)]
    pub surface: StatusSurface,
    #[serde(default)]
    pub emphasis: StatusEmphasis,
    #[serde(
        default,
        deserialize_with = "present_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub icon: Option<StatusIcon>,
}

fn present_value<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    input: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(input).map(Some)
}

impl StatusStyle {
    pub(crate) fn accent_rgb(&self) -> Option<[u8; 3]> {
        let hex = self.accent.as_deref()?.strip_prefix('#')?;
        if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let [_, red, green, blue] = u32::from_str_radix(hex, 16).ok()?.to_be_bytes();
        Some([red, green, blue])
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StatusDensity {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StatusSurface {
    #[default]
    Card,
    Plain,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StatusEmphasis {
    #[default]
    Normal,
    Strong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StatusIcon {
    Battery,
    Cup,
    Clock,
    Chart,
    Cloud,
    Calendar,
}

impl PluginStatus {
    pub(crate) fn valid(&self) -> bool {
        valid_text(&self.label, 40)
            && self
                .summary
                .as_deref()
                .is_none_or(|text| valid_text(text, 120))
            && self
                .alert
                .as_deref()
                .is_none_or(|text| valid_text(text, 120))
            && self
                .progress_pct
                .is_none_or(|pct| (0.0..=100.0).contains(&pct))
            && self.metrics.len() <= 6
            && self
                .metrics
                .iter()
                .all(|metric| valid_text(&metric.label, 28) && valid_text(&metric.value, 32))
            && self.actions.len() <= 3
            && self.actions.iter().enumerate().all(|(index, action)| {
                valid_control_id(&action.id)
                    && valid_text(&action.label, 6)
                    && valid_host_action(action.open.as_deref(), action.copy.as_deref())
                    && self.actions[..index].iter().all(|other| other.id != action.id)
            })
            && self.items.len() <= MAX_ITEMS
            && self.items.iter().all(|item| {
                valid_text(&item.label, 28)
                    && item.detail.as_deref().is_none_or(|text| valid_text(text, 80))
                    && item.action.as_deref().is_none_or(valid_control_id)
                    && valid_host_action(item.open.as_deref(), item.copy.as_deref())
            })
            && self.fields.len() <= 3
            && self.fields.iter().enumerate().all(|(index, field)| {
                valid_control_id(&field.key)
                    && valid_text(&field.label, 28)
                    && (1..=200).contains(&field.max)
                    && field.value.as_deref().is_none_or(|value| field.accepts(value))
                    && self.fields[..index].iter().all(|other| other.key != field.key)
            })
            && self.series.as_ref().is_none_or(|series| {
                (2..=48).contains(&series.len()) && series.iter().all(|value| value.is_finite())
            })
            && self.series_label.as_deref().is_none_or(|label| {
                self.series.is_some() && valid_text(label, 12)
            })
            && self.counter.len() <= MAX_REALTIME
            && self.counter.iter().all(StatusCounter::valid)
            && self.clock.len() <= MAX_REALTIME
            && self.clock.iter().all(StatusClock::valid)
            && self.countdown.len() <= MAX_REALTIME
            && self.countdown.iter().all(StatusCountdown::valid)
            && self.note.as_deref().is_none_or(valid_note)
            && self.tags.len() <= 4
            && self.tags.iter().all(|tag| valid_text(tag, 8))
            && self.observed_at.is_none_or(valid_timestamp)
            && self.expires_at.is_none_or(valid_timestamp)
            && self
                .observed_at
                .zip(self.expires_at)
                .is_none_or(|(at, expires)| at <= expires)
    }

    pub(crate) fn has_interactions(&self) -> bool {
        !self.actions.is_empty()
            || !self.fields.is_empty()
            || self.items.iter().any(|item| {
                item.action.is_some() || item.open.is_some() || item.copy.is_some()
            })
    }

    pub(crate) fn fresh_at(&self, now: i64) -> bool {
        self.expires_at.is_none_or(|expires| now < expires)
    }
}

fn valid_host_action(open: Option<&str>, copy: Option<&str>) -> bool {
    match (open, copy) {
        (Some(_), Some(_)) => false,
        (Some(open), None) => !open.is_empty() && open.len() <= 2048,
        (None, Some(text)) => {
            text.chars().count() <= 2000
                && text.chars().all(|ch| matches!(ch, '\r' | '\n' | '\t') || !ch.is_control())
        }
        (None, None) => true,
    }
}

impl StatusField {
    pub(crate) fn accepts(&self, value: &str) -> bool {
        value.chars().count() <= self.max && value.chars().all(|ch| self.accepts_char(ch))
    }

    pub(crate) fn accepts_char(&self, ch: char) -> bool {
        !ch.is_control() || self.multiline && ch == '\n'
    }
}

pub(crate) fn valid_control_id(id: &str) -> bool {
    id.len() <= 40
        && id.starts_with(|ch: char| ch.is_ascii_lowercase())
        && id.bytes().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, b'_' | b'-'))
}

pub(crate) fn valid_text(text: &str, max_chars: usize) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= max_chars
        && !text.chars().any(char::is_control)
}

fn valid_note(note: &str) -> bool {
    let note = note.replace("\r\n", "\n");
    !note.trim().is_empty()
        && note.chars().count() <= 300
        && !note.chars().any(|ch| ch != '\n' && ch.is_control())
}

fn valid_timestamp(timestamp: i64) -> bool {
    (0..=MAX_TIMESTAMP).contains(&timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plugin_status_accepts_bounded_fields_and_honors_expiry() {
        let status: PluginStatus = serde_json::from_value(json!({
            "state":"running", "label":"打印中", "summary":"首层校准完成", "progress_pct":48.5,
            "metrics":[{"label":"喷嘴","value":"220 °C"}], "alert":"耗材余量偏低",
            "observed_at":100, "expires_at":120
        }))
        .unwrap();
        assert!(status.valid());
        assert_eq!(status.state, StatusState::Running);
        assert_eq!(status.metrics[0].value, "220 °C");
        assert!(status.fresh_at(119));
        assert!(!status.fresh_at(120));
        let idle: PluginStatus =
            serde_json::from_value(json!({"state":"idle","label":"就绪"})).unwrap();
        assert!(idle.valid() && idle.fresh_at(MAX_TIMESTAMP));
    }

    #[test]
    fn plugin_status_rejects_invalid_or_oversized_values_and_unrecognized_fields() {
        for (key, value) in [
            ("label", json!("休眠\n正在打印")),
            ("summary", json!("测".repeat(121))),
            ("alert", json!(" ")),
            ("progress_pct", json!(100.1)),
            ("observed_at", json!(i64::MAX)),
            ("expires_at", json!(99)),
            (
                "metrics",
                json!(vec![json!({"label":"温度","value":"30 °C"}); 7]),
            ),
            ("metrics", json!([{"label":"温度","value":"测".repeat(33)}])),
        ] {
            let mut input = json!({"state":"running","label":"打印中","observed_at":100});
            input[key] = value;
            let status: PluginStatus = serde_json::from_value(input).unwrap();
            assert!(!status.valid(), "{key}: {status:?}");
        }
        assert!(serde_json::from_value::<PluginStatus>(json!({
            "state":"idle","label":"就绪","unknown_field":[]
        }))
        .is_err());
        assert!(serde_json::from_value::<PluginStatus>(json!({
            "state":"unknown","label":"异常"
        }))
        .is_err());
    }

    #[test]
    fn plugin_status_checks_component_data_limits() {
        let mut status: PluginStatus = serde_json::from_value(json!({
            "state":"running", "label":"打印中",
            "items":vec![json!({"label":"任务","detail":"热床升温","state":"paused"}); 8],
            "series":vec![15.0; 48],
            "note":format!("{}\r\n结束", "行".repeat(297)),
            "tags":["标签标签标签标签","模拟","精细","颜色"]
        }))
        .unwrap();
        assert!(status.valid());

        for series in [
            vec![],
            vec![1.0],
            vec![0.0; 49],
            vec![1.0, f64::INFINITY],
            vec![1.0, f64::NAN],
        ] {
            status.series = Some(series);
            assert!(!status.valid());
        }

        status.series = Some(vec![-f64::MAX, f64::MAX]);
        assert!(status.valid());
        status.items.push(status.items[0].clone());
        assert!(!status.valid());
        status.items.pop();
        status.items[0].detail = Some("测".repeat(81));
        assert!(!status.valid());
        status.items[0].detail = None;
        status.items[0].label = " ".into();
        assert!(!status.valid());
        status.items[0].label = "打印".into();
        status.note = Some("测".repeat(301));
        assert!(!status.valid());
        status.note = Some("第一行\t第二行".into());
        assert!(!status.valid());
        status.note = Some("第一行\n第二行".into());
        assert!(status.valid());

        status.tags.push("第五条".into());
        assert!(!status.valid());
        status.tags.pop();
        status.tags[0] = "标签标签标签标签超".into();
        assert!(!status.valid());
    }

    #[test]
    fn plugin_status_layout_accepts_the_old_format_and_rejects_duplicates() {
        let mut definition = json!({
            "components": [
                {"type":"state"}, {"type":"summary"}, {"type":"progress"}, {"type":"metrics"},
                {"type":"list"}, {"type":"chart"}, {"type":"text"}, {"type":"badge"}
            ],
            "style":{"emphasis":"strong","icon":"chart"}
        });
        let layout: StatusLayout = serde_json::from_value(definition.clone()).unwrap();
        assert!(layout.validate().is_ok());
        assert_eq!(layout.style.emphasis, StatusEmphasis::Strong);
        assert_eq!(layout.style.icon, Some(StatusIcon::Chart));

        let components = definition["components"].as_array_mut().unwrap();
        components.push(json!({"type":"alert"}));
        assert!(serde_json::from_value::<StatusLayout>(definition.clone())
            .unwrap()
            .validate()
            .is_ok());
        definition["components"][0] = json!({"type":"text"});
        definition["components"].as_array_mut().unwrap().pop();
        assert!(serde_json::from_value::<StatusLayout>(definition.clone())
            .unwrap()
            .validate()
            .is_err());

        definition["components"] = json!([{"type":"state"}]);
        for style in [
            json!({"icon":null}),
            json!({"icon":"custom"}),
            json!({"emphasis":"extra"}),
        ] {
            definition["style"] = style;
            assert!(serde_json::from_value::<StatusLayout>(definition.clone()).is_err());
        }
    }

    #[test]
    fn plugin_status_validates_realtime_parameters_and_list_actions() {
        let input = json!({
            "state":"running","label":"实时",
            "counter":{"start_value":10.0,"per_second":0.5,"start_at":100,"until":110,
                       "decimals":2,"prefix":"¥","sensitive":true},
            "clock":[{"label":"东京","utc_offset_minutes":540,"format":"HH:mm:ss"}],
            "countdown":{"label":"倒数","target_at":100,"format":"d天 HH:mm","done_label":"到了"},
            "items":[{"label":"便签","action":"delete-1","sensitive":true}],
            "series":[1, 2],"series_label":"走势"
        });
        let status: PluginStatus = serde_json::from_value(input.clone()).unwrap();
        assert!(status.valid());
        assert!(status.counter[0].sensitive);
        assert!(status.items[0].sensitive);
        assert_eq!(status.items[0].action.as_deref(), Some("delete-1"));
        assert_eq!(status.counter[0].value_at(105), "¥12.50");
        assert_eq!(status.counter[0].value_at(112), "¥15.00");
        assert_eq!(status.counter[0].repaint_at(110), None);
        assert_eq!(status.clock[0].value_at(86_399), "08:59:59");
        assert_eq!(status.countdown[0].value_at(99), "0天 00:01");
        assert_eq!(status.countdown[0].repaint_at(99), Some(100));
        assert_eq!(status.countdown[0].value_at(100), "到了");
        assert_eq!(status.countdown[0].value_at(101), "到了");
        assert_eq!(status.countdown[0].repaint_at(101), None);

        for (key, value) in [
            ("counter", json!(vec![input["counter"].clone(); 5])),
            ("counter", json!({"start_value":0, "per_second":1, "start_at":2, "until":1, "decimals":0})),
            ("counter", json!({"start_value":0, "per_second":1, "start_at":2, "decimals":5})),
            ("clock", json!({"label":"城市","utc_offset_minutes":841,"format":"HH:mm"})),
            ("countdown", json!({"label":"倒数","target_at":-1,"format":"HH:mm:ss"})),
            ("series_label", json!("测".repeat(13))),
            ("items", json!([{"label":"便签","action":"删除\n便签"}])),
        ] {
            let mut invalid = input.clone();
            invalid[key] = value;
            let status: PluginStatus = serde_json::from_value(invalid).unwrap();
            assert!(!status.valid(), "{key}");
        }
        let mut status: PluginStatus = serde_json::from_value(input).unwrap();
        status.counter[0].start_value = f64::INFINITY;
        assert!(!status.valid());
    }

    #[test]
    fn plugin_status_layout_counts_real_time_occurrences_in_the_total() {
        let mut components = vec![json!({"type":"counter"}); 4];
        components.extend([
            json!({"type":"clock"}),
            json!({"type":"clock"}),
            json!({"type":"countdown"}),
            json!({"type":"summary"}),
            json!({"type":"state"}),
            json!({"type":"text"}),
        ]);
        let validate = |components: &[serde_json::Value]| {
            serde_json::from_value::<StatusLayout>(json!({"components":components}))
                .unwrap()
                .validate()
        };
        assert!(validate(&components).is_ok());
        components.push(json!({"type":"list"}));
        assert!(validate(&components).is_err());
        components.pop();
        components[4] = json!({"type":"counter"});
        assert!(validate(&components).is_err());
    }
}
