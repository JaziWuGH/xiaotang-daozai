//! 按画布实际使用的绑定安排前台刷新。

use super::{Binding, Daylight, Hand, NowFormat};
use crate::plugin_canvas::{
    Bounds, Canvas, Color, Glow, Kind, Node, Num, Paint, COORD_LIMIT, MAX_TEXT,
};
use chrono::{DateTime, Days, Local, MappedLocalTime, TimeDelta, TimeZone, Timelike, Utc};
use chrono_tz::GapInfo;
use std::{borrow::Cow, collections::BTreeMap, time::Duration};

impl Canvas {
    pub(crate) fn evaluate(
        &self,
        now: DateTime<Utc>,
        pixels: f32,
    ) -> (Cow<'_, Self>, Option<Duration>) {
        if self.binds.is_empty() {
            return (Cow::Borrowed(self), None);
        }
        let mut frame = Frame {
            canvas: self,
            now,
            pixels: f64::from(pixels).max(0.1),
            seen: BTreeMap::new(),
            deadline: None,
            text_left: MAX_TEXT.saturating_sub(text_count(&self.nodes)),
        };
        let mut canvas = self.clone();
        frame.nodes(&mut canvas.nodes);
        let delay = frame
            .deadline
            .map(|at| Duration::from_millis((at - now.timestamp_millis()).max(1) as u64));
        let canvas = if frame.seen.is_empty() {
            Cow::Borrowed(self)
        } else {
            Cow::Owned(canvas)
        };
        (canvas, delay)
    }
}

struct Sample {
    numeric: Option<f64>,
    text: String,
    next_second: Option<i64>,
}

impl Sample {
    fn numeric(number: f64, next_second: Option<i64>) -> Self {
        Self {
            numeric: Some(number),
            text: number.to_string(),
            next_second,
        }
    }
}

impl Binding {
    fn sample(&self, now: DateTime<Utc>) -> Sample {
        let second = now.timestamp();
        match self {
            Self::Counter { counter } => {
                let stop = counter.until.unwrap_or(i64::MAX);
                let elapsed = (second.clamp(counter.start_at, stop) - counter.start_at) as f64;
                let value = counter.per_second.mul_add(elapsed, counter.start_value);
                Sample {
                    numeric: Some(value),
                    text: format!("{value:.prec$}", prec = counter.decimals),
                    next_second: (counter.per_second != 0.0
                        && counter.start_at < stop
                        && second < stop)
                        .then_some(second.max(counter.start_at) + 1),
                }
            }
            Self::Countdown { countdown } => {
                let left = (countdown.target_at - second).max(0);
                Sample {
                    numeric: Some(left as f64),
                    text: countdown.format.value(left, None),
                    next_second: countdown.format.repaint_at(second, left),
                }
            }
            Self::Progress {
                progress: [start, end],
            } => {
                let value = ((now.timestamp_millis() - start * 1000) as f64
                    / ((end - start) * 1000) as f64)
                    .clamp(0.0, 1.0);
                Sample {
                    numeric: Some(value),
                    text: format!("{:.2}", (value * 100.0).floor() / 100.0),
                    next_second: None,
                }
            }
            Self::ClockAngle { clock_angle } => {
                let local = now.with_timezone(&clock_angle.zone);
                let minute = f64::from(local.minute());
                let degrees = match clock_angle.hand {
                    Hand::Hour => f64::from(local.hour() % 12) * 30.0 + minute / 2.0,
                    Hand::Minute => minute * 6.0,
                };
                Sample::numeric(degrees, Some(second + 60 - i64::from(local.second())))
            }
            Self::Now { now: format, zone } => match zone {
                Some(zone) => clock(now.with_timezone(zone), *format),
                None => clock(now.with_timezone(&Local), *format),
            },
            Self::Daylight { daylight } => {
                let active = daylight.active(now.with_timezone(&daylight.zone).hour());
                Sample::numeric(f64::from(u8::from(active)), daylight.next(now))
            }
        }
    }
}

fn clock<T: TimeZone>(local: DateTime<T>, format: NowFormat) -> Sample {
    let (hour, minute, second) = (local.hour(), local.minute(), local.second());
    let (text, wait) = match format {
        NowFormat::Minute => (format!("{hour:02}:{minute:02}"), 60 - second),
        NowFormat::Second => (format!("{hour:02}:{minute:02}:{second:02}"), 1),
    };
    Sample {
        numeric: None,
        text,
        next_second: Some(local.timestamp() + i64::from(wait)),
    }
}

impl Daylight {
    fn active(&self, hour: u32) -> bool {
        let [from, to] = self.hours.map(u32::from);
        if from <= to {
            (from..to).contains(&hour)
        } else {
            hour >= from || hour < to
        }
    }

    fn next(&self, now: DateTime<Utc>) -> Option<i64> {
        let [from, to] = self.hours;
        if from == to || [from, to] == [0, 24] {
            return None;
        }
        let today = now.with_timezone(&self.zone).date_naive();
        let mut earliest = None;
        for offset in 0..=2 {
            let midnight = today
                .checked_add_days(Days::new(offset))?
                .and_hms_opt(0, 0, 0)?;
            for hour in [from, to] {
                let local = midnight.checked_add_signed(TimeDelta::hours(i64::from(hour)))?;
                let candidates = match self.zone.from_local_datetime(&local) {
                    MappedLocalTime::Single(at) => [Some(at), None],
                    MappedLocalTime::Ambiguous(first, second) => [Some(first), Some(second)],
                    MappedLocalTime::None => [
                        GapInfo::new(&local, &self.zone).and_then(|gap| gap.end),
                        None,
                    ],
                };
                for at in candidates.into_iter().flatten() {
                    let Some(before) = DateTime::from_timestamp(at.timestamp() - 1, 0) else {
                        continue;
                    };
                    if at.timestamp() > now.timestamp()
                        && self.active(before.with_timezone(&self.zone).hour())
                            != self.active(at.hour())
                    {
                        earliest = Some(
                            earliest.map_or(at.timestamp(), |old: i64| old.min(at.timestamp())),
                        );
                    }
                }
            }
        }
        earliest
    }
}

#[derive(Clone, Copy)]
enum Usage {
    Text,
    Number(f64),
    Color,
}

impl Usage {
    fn progress(self, start: i64, end: i64, now: i64) -> Option<i64> {
        let begin = start * 1000;
        let end = end * 1000;
        if now >= end {
            return None;
        }
        let span = (end - begin) as f64;
        let elapsed = (now - begin).max(0) as f64;
        let steps = match self {
            Self::Color if elapsed * 2.0 >= span => return None,
            Self::Color => 2.0,
            Self::Number(steps) if steps <= 0.0 => return None,
            Self::Number(steps) => steps.ceil().max(1.0),
            Self::Text => 100.0,
        };
        let next_step = (elapsed * steps / span).floor() + 1.0;
        let checkpoint = (span * next_step / steps).ceil() as i64;
        Some((begin + checkpoint).clamp(now + 1, end))
    }
}

struct Frame<'a> {
    canvas: &'a Canvas,
    now: DateTime<Utc>,
    pixels: f64,
    seen: BTreeMap<&'a str, Sample>,
    deadline: Option<i64>,
    text_left: usize,
}

impl Frame<'_> {
    fn read(&mut self, name: &str, usage: Usage) -> Option<&Sample> {
        let (key, binding) = self.canvas.binds.get_key_value(name)?;
        let sample = self
            .seen
            .entry(key)
            .or_insert_with(|| binding.sample(self.now));
        let next = match binding {
            Binding::Progress {
                progress: [start, end],
            } => usage.progress(*start, *end, self.now.timestamp_millis()),
            Binding::Countdown { countdown } if !matches!(usage, Usage::Text) => {
                (self.now.timestamp() < countdown.target_at)
                    .then_some((self.now.timestamp() + 1) * 1000)
            }
            _ => sample.next_second.map(|second| second * 1000),
        };
        if let Some(next) = next {
            self.deadline = Some(self.deadline.map_or(next, |old| old.min(next)));
        }
        Some(sample)
    }

    fn literal(&mut self, number: &mut Num, min: f32, max: f32, precision: f64) {
        let Num::Bind { bind, map } = number else {
            return;
        };
        let steps = map.map_or(1.0, |[a, b]| f64::from((b - a).abs())) * precision;
        let numeric = self
            .read(bind, Usage::Number(steps))
            .and_then(|sample| sample.numeric);
        let value = match (numeric, map) {
            (Some(value), Some([start, end])) => {
                f64::from(*start) + f64::from(*end - *start) * value.clamp(0.0, 1.0)
            }
            (Some(value), None) => value,
            (None, Some([start, _])) => f64::from(*start),
            (None, None) => 0.0,
        };
        *number = Num::Literal(value.clamp(f64::from(min), f64::from(max)) as f32);
    }

    fn coord(&mut self, coord: &mut Num) {
        self.literal(coord, -COORD_LIMIT, COORD_LIMIT, self.pixels);
    }

    fn coords(&mut self, coords: &mut [Num]) {
        coords.iter_mut().for_each(|coord| self.coord(coord));
    }

    fn length(&mut self, value: &mut Num) {
        self.literal(value, 0.0, COORD_LIMIT, self.pixels);
    }

    fn bounds(&mut self, rect: &mut Bounds) {
        self.coords(&mut rect[..2]);
        rect[2..].iter_mut().for_each(|value| self.length(value));
    }

    fn color(&mut self, color: &mut Color) {
        if let Color::Pick { bind, pick } = color {
            let index = usize::from(
                self.read(bind, Usage::Color)
                    .and_then(|sample| sample.numeric)
                    .unwrap_or_default()
                    >= 0.5,
            );
            *color = pick[index].clone();
        }
    }

    fn paint(&mut self, paint: &mut Paint) {
        let stops = match paint {
            Paint::Solid(color) => return self.color(color),
            Paint::Linear { from, to, stops } => {
                self.coords(from);
                self.coords(to);
                stops
            }
            Paint::Radial {
                center,
                radius,
                stops,
            } => {
                self.coords(center);
                self.length(radius);
                stops
            }
        };
        for (_, color) in stops {
            self.color(color);
        }
    }

    fn text(&mut self, text: &mut String) {
        if !text.contains('{') {
            return;
        }
        let mut replaced = String::with_capacity(text.len());
        for segment in text.split_inclusive('}') {
            let binding = segment
                .strip_suffix('}')
                .and_then(|body| body.rsplit_once('{'));
            if let Some((prefix, name)) = binding {
                if let Some(sample) = self.read(name, Usage::Text) {
                    replaced.push_str(prefix);
                    replaced.push_str(&sample.text);
                    continue;
                }
            }
            replaced.push_str(segment);
        }
        let limit = text.chars().count() + self.text_left;
        if let Some((end, _)) = replaced.char_indices().nth(limit) {
            replaced.truncate(end);
        }
        self.text_left = limit - replaced.chars().count();
        *text = replaced;
    }

    fn nodes(&mut self, nodes: &mut [Node]) {
        for node in nodes {
            match &mut node.kind {
                Kind::Rect { rect, r } => {
                    self.bounds(rect);
                    r.iter_mut().for_each(|radius| self.length(radius));
                }
                Kind::Circle([x, y, r]) => {
                    self.coord(x);
                    self.coord(y);
                    self.length(r);
                }
                Kind::Arc { c, r, start, sweep } => {
                    self.coords(c);
                    self.length(r);
                    self.coord(start);
                    self.coord(sweep);
                }
                Kind::Line(line) => self.coords(line),
                Kind::Polyline { points, area } => {
                    for point in points {
                        self.coords(point);
                    }
                    if let Some(y) = area {
                        self.coord(y);
                    }
                }
                Kind::Path(_) => {}
                Kind::Text {
                    text,
                    at,
                    size,
                    max_width,
                    ..
                } => {
                    self.text(text);
                    self.coords(at);
                    self.literal(size, 6.0, 96.0, self.pixels);
                    if let Some(width) = max_width {
                        self.length(width);
                    }
                }
                Kind::Image { rect, .. } | Kind::Icon { rect, .. } => self.bounds(rect),
                Kind::Pet { at, size, .. } => {
                    self.coords(at);
                    self.literal(size, 12.0, 96.0, self.pixels);
                }
                Kind::Group {
                    group,
                    clip,
                    translate,
                    rotate,
                    scale,
                } => {
                    if let Some(clip) = clip {
                        self.bounds(clip);
                    }
                    self.coords(translate);
                    self.coord(rotate);
                    self.length(scale);
                    self.nodes(group);
                }
            }

            for paint in [&mut node.style.fill, &mut node.style.stroke]
                .into_iter()
                .flatten()
            {
                self.paint(paint);
            }
            if let Some(Glow::Color(color)) = &mut node.style.glow {
                self.color(color);
            }
            self.length(&mut node.style.width);
            self.literal(&mut node.style.opacity, 0.0, 1.0, 255.0);
            if let Some(dash) = &mut node.style.dash {
                dash.iter_mut().for_each(|number| self.length(number));
            }
            if let Some(hover) = &mut node.hover {
                if let Some(opacity) = &mut hover.opacity {
                    self.literal(opacity, 0.0, 1.0, 255.0);
                }
                if let Some(lift) = &mut hover.lift {
                    self.coord(lift);
                }
                if let Some(color) = &mut hover.fill {
                    self.color(color);
                }
            }
        }
    }
}

fn text_count(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|node| {
            let hint = node.hover.as_ref().and_then(|hover| hover.hint.as_ref())
                .map_or(0, |hint| hint.chars().count());
            hint + match &node.kind {
                Kind::Text { text, .. } => text.chars().count(),
                Kind::Group { group, .. } => text_count(group),
                _ => 0,
            }
        })
        .sum()
}

#[cfg(test)]
mod tests;
