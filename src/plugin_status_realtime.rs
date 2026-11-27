//! 由宿主按 Unix 秒计算状态页的实时值。

use super::{valid_text, valid_timestamp, MAX_TIMESTAMP};
use serde::{Deserialize, Deserializer};

pub(super) fn one_or_many<'de, D, T>(input: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value<T> {
        Many(Vec<T>),
        One(T),
    }
    Ok(match Value::deserialize(input)? {
        Value::One(value) => vec![value],
        Value::Many(values) => values,
    })
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusCounter {
    pub start_value: f64,
    per_second: f64,
    start_at: i64,
    until: Option<i64>,
    decimals: usize,
    prefix: Option<String>,
    suffix: Option<String>,
    #[serde(default)]
    pub sensitive: bool,
}

impl StatusCounter {
    pub(crate) fn valid(&self) -> bool {
        self.start_value.is_finite()
            && self.per_second.is_finite()
            && valid_timestamp(self.start_at)
            && self
                .until
                .is_none_or(|at| valid_timestamp(at) && at >= self.start_at)
            && self.decimals <= 4
            && self
                .prefix
                .as_deref()
                .is_none_or(|text| valid_text(text, 12))
            && self
                .suffix
                .as_deref()
                .is_none_or(|text| valid_text(text, 12))
            && self.number_at(0).is_finite()
            && self
                .number_at(self.until.unwrap_or(MAX_TIMESTAMP))
                .is_finite()
    }

    fn number_at(&self, now: i64) -> f64 {
        let at = self.until.map_or(now, |until| now.min(until));
        self.per_second
            .mul_add((at - self.start_at) as f64, self.start_value)
    }

    pub(crate) fn value_at(&self, now: i64) -> String {
        format!(
            "{}{:.*}{}",
            self.prefix.as_deref().unwrap_or_default(),
            self.decimals,
            self.number_at(now),
            self.suffix.as_deref().unwrap_or_default()
        )
    }

    pub(crate) fn reveal_identity(&self) -> impl std::hash::Hash + '_ {
        (
            self.start_value.to_bits(),
            self.per_second.to_bits(),
            self.start_at,
            self.until,
            self.decimals,
            &self.prefix,
            &self.suffix,
        )
    }

    pub(crate) fn repaint_at(&self, now: i64) -> Option<i64> {
        (self.per_second != 0.0 && self.until.is_none_or(|until| now < until)).then_some(now + 1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusClock {
    pub label: String,
    utc_offset_minutes: i16,
    format: ClockFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
enum ClockFormat {
    #[serde(rename = "HH:mm")]
    Minute,
    #[serde(rename = "HH:mm:ss")]
    Second,
}

impl StatusClock {
    pub(super) fn valid(&self) -> bool {
        valid_text(&self.label, 28) && (-840..=840).contains(&self.utc_offset_minutes)
    }

    pub(crate) fn value_at(&self, now: i64) -> String {
        let local = (now + i64::from(self.utc_offset_minutes) * 60).rem_euclid(86_400);
        let hour = local / 3600;
        let minute = local / 60 % 60;
        match self.format {
            ClockFormat::Minute => format!("{hour:02}:{minute:02}"),
            ClockFormat::Second => format!("{hour:02}:{minute:02}:{:02}", local % 60),
        }
    }

    pub(crate) fn repaint_at(&self, now: i64) -> i64 {
        match self.format {
            ClockFormat::Minute => now + 60 - now.rem_euclid(60),
            ClockFormat::Second => now + 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusCountdown {
    pub label: String,
    target_at: i64,
    format: CountdownFormat,
    done_label: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub(crate) enum CountdownFormat {
    #[serde(rename = "d天 HH:mm")]
    DayMinute,
    #[serde(rename = "HH:mm:ss")]
    Second,
}

impl StatusCountdown {
    pub(crate) fn valid(&self) -> bool {
        valid_text(&self.label, 28)
            && valid_timestamp(self.target_at)
            && self
                .done_label
                .as_deref()
                .is_none_or(|label| valid_text(label, 28))
    }

    pub(crate) fn value_at(&self, now: i64) -> String {
        self.format
            .value(self.target_at - now, self.done_label.as_deref())
    }

    pub(crate) fn repaint_at(&self, now: i64) -> Option<i64> {
        self.format.repaint_at(now, self.target_at - now)
    }
}

impl CountdownFormat {
    pub(crate) fn value(self, left: i64, done_label: Option<&str>) -> String {
        if left <= 0 {
            return done_label.unwrap_or("已结束").into();
        }
        match self {
            CountdownFormat::DayMinute => {
                // 分钟向上取整，目标时间到达时才显示完成。
                let minutes = (left + 59) / 60;
                format!(
                    "{}天 {:02}:{:02}",
                    minutes / 1440,
                    minutes / 60 % 24,
                    minutes % 60
                )
            }
            CountdownFormat::Second => {
                format!("{:02}:{:02}:{:02}", left / 3600, left / 60 % 60, left % 60)
            }
        }
    }

    pub(crate) fn repaint_at(self, now: i64, left: i64) -> Option<i64> {
        if left <= 0 {
            return None;
        }
        Some(match self {
            CountdownFormat::DayMinute => now + 1 + (left - 1) % 60,
            CountdownFormat::Second => now + 1,
        })
    }
}
