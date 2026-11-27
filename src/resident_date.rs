//! 收起时与时钟并排的日期，按宿主日期计算，不依赖插件或网络。

use chrono::{Datelike, NaiveDate};
use eframe::egui::{self, Color32};

use crate::{
    lunar_calendar::{self, LunarDate},
    model::ResidentDate,
    paint::{self, Card, H},
    theme::{self, size, W},
};

const CENTER_Y: f32 = 15.0;
const SINGLE_DATE_SIZE: f32 = 12.0;
const DATE_SIZE: f32 = 9.0;
const CLOCK_GAP: f32 = 12.0;
const TERM_GAP: f32 = 3.0;
const TERM_PAD: f32 = 6.0;
const MUTED: Color32 = Color32::from_rgb(190, 191, 200);
const TERM_BG: Color32 = Color32::from_rgb(27, 45, 63);

struct LunarStyle {
    size: f32,
    weight: W,
    color: Color32,
    term_size: f32,
}

const SINGLE_LUNAR: LunarStyle = LunarStyle {
    size: SINGLE_DATE_SIZE,
    weight: W::Semibold,
    color: theme::WHITE,
    term_size: 8.5,
};
const DOUBLE_LUNAR: LunarStyle = LunarStyle {
    size: 7.8,
    weight: W::Regular,
    color: MUTED,
    term_size: 7.0,
};

pub(crate) enum ResidentDay {
    Gregorian {
        date: String,
        short: String,
    },
    LunarOnly(LunarDate),
    Combined {
        date: String,
        short: String,
        lunar: LunarDate,
    },
}

impl ResidentDay {
    pub(crate) fn new(mode: ResidentDate, date: NaiveDate) -> Option<Self> {
        match mode {
            ResidentDate::Off => None,
            ResidentDate::LunarOnly => lunar_calendar::for_date(date).map(Self::LunarOnly),
            ResidentDate::Gregorian | ResidentDate::Lunar => {
                let short = format!("{}/{}", date.month(), date.day());
                let label = format!("{}月{}日", date.month(), date.day());
                if mode == ResidentDate::Lunar {
                    if let Some(lunar) = lunar_calendar::for_date(date) {
                        return Some(Self::Combined {
                            date: label,
                            short,
                            lunar,
                        });
                    }
                }
                Some(Self::Gregorian { date: label, short })
            }
        }
    }

    pub(crate) fn group_width(&self, ctx: &egui::Context, clock: Option<&str>) -> f32 {
        let measure = |text: &str, size: f32, weight: W| {
            ctx.fonts(|fonts| {
                fonts
                    .layout_no_wrap(text.to_owned(), theme::font(size, weight), theme::WHITE)
                    .size()
                    .x
            })
        };
        let lunar_width = |lunar: &LunarDate, style: LunarStyle| {
            measure(&lunar.label, style.size, style.weight)
                + lunar.solar_term.map_or(0.0, |term| {
                    TERM_GAP + TERM_PAD + measure(term, style.term_size, W::Semibold)
                })
        };
        let date_width = match self {
            Self::Gregorian { date, .. } => measure(date, SINGLE_DATE_SIZE, W::Semibold),
            Self::LunarOnly(lunar) => lunar_width(lunar, SINGLE_LUNAR),
            Self::Combined { date, lunar, .. } => {
                measure(date, DATE_SIZE, W::Semibold).max(lunar_width(lunar, DOUBLE_LUNAR))
            }
        };
        let clock_space = clock.map_or(0.0, |clock| {
            let width = if let Some((period, digits)) = clock.split_once(' ') {
                measure(period, size::PILL_CLOCK * 0.72, W::Semibold)
                    + 4.0
                    + measure(digits, size::PILL_CLOCK, W::Bold)
            } else {
                measure(clock, size::PILL_CLOCK, W::Bold)
            };
            width + CLOCK_GAP
        });
        clock_space + date_width
    }

    /// 和设置中的常驻排序预览使用同一套位置和色彩。
    pub(crate) fn draw_group(&self, c: &Card, center_x: f32, clock: Option<&str>) {
        let left = center_x - self.group_width(c.ui.ctx(), clock) / 2.0;
        if let Some(clock) = clock {
            paint::clock(c, left, 21.0, clock, size::PILL_CLOCK);
        }
        self.draw_after_clock(c, left, clock);
    }

    pub(crate) fn draw_after_clock(&self, c: &Card, left: f32, clock: Option<&str>) {
        let x = if let Some(clock) = clock {
            let right = left + paint::clock_width(c, clock, size::PILL_CLOCK);
            c.seg(
                (right + 6.0, 7.0),
                (right + 6.0, 23.0),
                0.6,
                Color32::from_gray(68),
            );
            right + CLOCK_GAP
        } else {
            left
        };
        match self {
            Self::Gregorian { date, .. } => c.centered_text(
                x,
                CENTER_Y,
                date,
                SINGLE_DATE_SIZE,
                W::Semibold,
                theme::WHITE,
                H::Left,
            ),
            Self::LunarOnly(lunar) => Self::draw_lunar(c, x, CENTER_Y, lunar, SINGLE_LUNAR),
            Self::Combined { date, lunar, .. } => {
                c.centered_text(x, 9.5, date, DATE_SIZE, W::Semibold, theme::WHITE, H::Left);
                Self::draw_lunar(c, x, 20.5, lunar, DOUBLE_LUNAR);
            }
        }
    }

    fn draw_lunar(c: &Card, x: f32, y: f32, lunar: &LunarDate, style: LunarStyle) {
        c.centered_text(
            x,
            y,
            &lunar.label,
            style.size,
            style.weight,
            style.color,
            H::Left,
        );
        if let Some(term) = lunar.solar_term {
            let left = x + c.text_w(&lunar.label, style.size, style.weight) + TERM_GAP;
            Self::term_badge(c, left, y, term, style.term_size);
        }
    }

    fn term_badge(c: &Card, left: f32, y: f32, term: &str, size: f32) {
        let width = c.text_w(term, size, W::Semibold) + TERM_PAD;
        let height = size + 1.8;
        c.fill(left, y - height / 2.0, width, height, 3.0, TERM_BG);
        c.centered_text(
            left + width / 2.0,
            y,
            term,
            size,
            W::Semibold,
            theme::WHITE,
            H::Center,
        );
    }

    /// 侧停靠的窄条使用月/日，过宽的农历按月和日拆行。
    pub(crate) fn draw_side(&self, c: &Card, x: f32, y: f32) {
        let (gregorian, lunar, style, step) = match self {
            Self::Gregorian { short, .. } => {
                Self::side_text(c, x, y, short, SINGLE_DATE_SIZE, W::Semibold, theme::WHITE);
                return;
            }
            Self::LunarOnly(lunar) => (None, lunar, SINGLE_LUNAR, 13.0),
            Self::Combined { short, lunar, .. } => {
                (Some(short.as_str()), lunar, DOUBLE_LUNAR, 10.0)
            }
        };
        let split = if c.text_w(&lunar.label, style.size, style.weight) > 30.0 {
            lunar
                .label
                .find('月')
                .map(|i| lunar.label.split_at(i + '月'.len_utf8()))
        } else {
            None
        };
        let extra_rows = u8::from(gregorian.is_some())
            + u8::from(split.is_some())
            + u8::from(lunar.solar_term.is_some());
        let mut row = y - step * f32::from(extra_rows) / 2.0;
        if let Some(date) = gregorian {
            Self::side_text(c, x, row, date, DATE_SIZE, W::Semibold, theme::WHITE);
            row += step;
        }
        let (month, day) = split.unwrap_or((&lunar.label, ""));
        Self::side_text(c, x, row, month, style.size, style.weight, style.color);
        row += step;
        if !day.is_empty() {
            Self::side_text(c, x, row, day, style.size, style.weight, style.color);
            row += step;
        }
        if let Some(term) = lunar.solar_term {
            let left = x - (c.text_w(term, style.term_size, W::Semibold) + TERM_PAD) / 2.0;
            Self::term_badge(c, left, row, term, style.term_size);
        }
    }

    fn side_text(c: &Card, x: f32, y: f32, text: &str, size: f32, weight: W, color: Color32) {
        let fitted_size = size * (30.0 / c.text_w(text, size, weight)).min(1.0);
        c.centered_text(x, y, text, fitted_size, weight, color, H::Center);
    }
}
