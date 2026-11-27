//! 画布中保存的实时值、动画与交互声明。

use super::{Color, Num};
use crate::plugin_status::CountdownFormat;
use chrono_tz::Tz;
use serde::Deserialize;

mod evaluate;

const MAX_TIMESTAMP: i64 = 253_402_300_799;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum Binding {
    Counter { counter: Counter },
    Countdown { countdown: Countdown },
    Progress { progress: [i64; 2] },
    ClockAngle { clock_angle: ClockAngle },
    Now { now: NowFormat, zone: Option<Tz> },
    Daylight { daylight: Daylight },
}

impl Binding {
    pub(super) fn valid(&self) -> bool {
        let timestamp = |value| (0..=MAX_TIMESTAMP).contains(&value);
        match self {
            Self::Counter { counter } => {
                counter.start_value.is_finite()
                    && counter.per_second.is_finite()
                    && timestamp(counter.start_at)
                    && counter.decimals <= 4
                    && counter
                        .until
                        .is_none_or(|end| timestamp(end) && end >= counter.start_at)
                    && counter
                        .per_second
                        .mul_add(
                            (counter.until.unwrap_or(MAX_TIMESTAMP) - counter.start_at) as f64,
                            counter.start_value,
                        )
                        .is_finite()
            }
            Self::Countdown { countdown } => timestamp(countdown.target_at),
            Self::Progress {
                progress: [start, end],
            } => timestamp(*start) && timestamp(*end) && start < end,
            Self::ClockAngle { .. } | Self::Now { .. } => true,
            Self::Daylight { daylight } => daylight.hours[0] < 24 && daylight.hours[1] <= 24,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Counter {
    pub start_value: f64,
    pub per_second: f64,
    pub start_at: i64,
    pub decimals: usize,
    pub until: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Countdown {
    pub target_at: i64,
    pub format: CountdownFormat,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClockAngle {
    pub zone: Tz,
    pub hand: Hand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Hand {
    Hour,
    Minute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub(crate) enum NowFormat {
    #[serde(rename = "HH:mm")]
    Minute,
    #[serde(rename = "HH:mm:ss")]
    Second,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Daylight {
    pub zone: Tz,
    pub hours: [u8; 2],
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Animation {
    pub ms: u16,
    #[serde(default)]
    pub ease: Ease,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Ease {
    #[default]
    Linear,
    Spring,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Loop {
    pub kind: LoopKind,
    pub period_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LoopKind {
    Pulse,
    Flow,
    Bob,
    Spin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Burst {
    Confetti,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Hit {
    Action(String),
    Open(String),
    Copy(String),
}

impl Hit {
    pub(super) fn valid(&self) -> bool {
        match self {
            Self::Action(id) => crate::plugin_status::valid_control_id(id),
            Self::Open(link) => !link.is_empty() && link.len() <= 2048,
            Self::Copy(text) => {
                text.chars().count() <= 2000
                    && text
                        .chars()
                        .all(|ch| matches!(ch, '\r' | '\n' | '\t') || !ch.is_control())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Hover {
    pub opacity: Option<Num>,
    pub lift: Option<Num>,
    pub fill: Option<Color>,
    pub hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scroll {
    #[serde(rename = "axis")]
    _axis: ScrollAxis,
    pub content: f32,
}

impl Scroll {
    pub(super) fn valid(&self) -> bool {
        self.content.is_finite() && self.content > 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScrollAxis {
    Y,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tip {
    pub x: f32,
    pub label: String,
    pub value: String,
}
