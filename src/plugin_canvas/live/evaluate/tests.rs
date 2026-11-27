use super::*;
use crate::plugin_canvas::Token;
use serde_json::{json, Value};

fn canvas(binds: Value, nodes: Value) -> Canvas {
    Canvas::parse(json!({"height":100,"binds":binds,"nodes":nodes})).unwrap()
}

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn seconds(value: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(value, 0).unwrap()
}

fn parse_binding(value: Value) -> Binding {
    serde_json::from_value(value).unwrap()
}

#[test]
fn sdk_spike_canvas_counter_starts_and_stops_at_its_declared_times() {
    let counter = parse_binding(json!({
        "counter":{"start_at":100,"until":106,"start_value":1,"per_second":0.25,"decimals":2}
    }));
    for (now, value, text, next) in [
        (99, 1.0, "1.00", Some(101)),
        (100, 1.0, "1.00", Some(101)),
        (104, 2.0, "2.00", Some(105)),
        (106, 2.5, "2.50", None),
        (107, 2.5, "2.50", None),
    ] {
        let sample = counter.sample(seconds(now));
        assert_eq!(sample.numeric, Some(value));
        assert_eq!(sample.text, text);
        assert_eq!(sample.next_second, next);
    }
}

#[test]
fn sdk_spike_canvas_countdown_shares_standard_formats_and_deadline() {
    let countdown = parse_binding(json!({"countdown":{"target_at":200,"format":"d天 HH:mm"}}));
    for (now, value, text, next) in [
        (139, 61.0, "0天 00:02", Some(140)),
        (140, 60.0, "0天 00:01", Some(200)),
        (199, 1.0, "0天 00:01", Some(200)),
        (200, 0.0, "已结束", None),
        (201, 0.0, "已结束", None),
    ] {
        let sample = countdown.sample(seconds(now));
        assert_eq!(sample.numeric, Some(value));
        assert_eq!(sample.text, text);
        assert_eq!(sample.next_second, next);
    }
    let second = parse_binding(json!({"countdown":{"target_at":200,"format":"HH:mm:ss"}}))
        .sample(seconds(189));
    assert_eq!(second.text, "00:00:11");
    assert_eq!(second.next_second, Some(190));
}

#[test]
fn sdk_spike_canvas_clock_respects_midnight_and_daylight_saving() {
    let hour = parse_binding(json!({"clock_angle":{"zone":"America/New_York","hand":"hour"}}));
    let minute = parse_binding(json!({"clock_angle":{"zone":"America/New_York","hand":"minute"}}));
    let clock = parse_binding(json!({"now":"HH:mm","zone":"America/New_York"}));
    for (when, hhmm, h, m) in [
        ("2026-03-08T06:59:59Z", "01:59", 59.5, 354.0),
        ("2026-03-08T07:00:00Z", "03:00", 90.0, 0.0),
        ("2026-11-01T05:59:59Z", "01:59", 59.5, 354.0),
        ("2026-11-01T06:00:00Z", "01:00", 30.0, 0.0),
        ("2026-11-24T04:59:59Z", "23:59", 359.5, 354.0),
        ("2026-11-24T05:00:00Z", "00:00", 0.0, 0.0),
    ] {
        let now = utc(when);
        assert_eq!(hour.sample(now).numeric, Some(h));
        assert_eq!(minute.sample(now).numeric, Some(m));
        assert_eq!(clock.sample(now).text, hhmm);
        let next = now.timestamp() + 60 - i64::from(now.second());
        assert_eq!(hour.sample(now).next_second, Some(next));
        assert_eq!(clock.sample(now).next_second, Some(next));
    }
    let clock = parse_binding(json!({"now":"HH:mm:ss","zone":"Asia/Kathmandu"}));
    let now = utc("2026-11-24T04:15:30Z");
    assert_eq!(clock.sample(now).text, "10:00:30");
    assert_eq!(clock.sample(now).next_second, Some(now.timestamp() + 1));
}

#[test]
fn sdk_spike_canvas_daylight_finds_local_boundaries() {
    let daylight = parse_binding(json!({"daylight":{"zone":"America/New_York","hours":[2,4]}}));
    let before_jump = utc("2026-03-08T06:59:59Z");
    let jump = utc("2026-03-08T07:00:00Z");
    let sample = daylight.sample(before_jump);
    assert_eq!(sample.numeric, Some(0.0));
    assert_eq!(sample.next_second, Some(jump.timestamp()));
    let sample = daylight.sample(jump);
    assert_eq!(sample.numeric, Some(1.0));
    assert_eq!(
        sample.next_second,
        Some(utc("2026-03-08T08:00:00Z").timestamp())
    );

    let night = parse_binding(json!({"daylight":{"zone":"Asia/Shanghai","hours":[18,6]}}));
    let before_night = utc("2026-11-24T09:59:59Z");
    let night_starts = utc("2026-11-24T10:00:00Z");
    let night_ends = utc("2026-11-24T22:00:00Z");
    assert_eq!(night.sample(before_night).numeric, Some(0.0));
    assert_eq!(
        night.sample(before_night).next_second,
        Some(night_starts.timestamp())
    );
    assert_eq!(night.sample(night_starts).numeric, Some(1.0));
    assert_eq!(
        night.sample(night_starts).next_second,
        Some(night_ends.timestamp())
    );
    assert_eq!(night.sample(night_ends).numeric, Some(0.0));
}

#[test]
fn sdk_spike_canvas_progress_maps_and_schedules_at_pixel_precision() {
    let canvas = canvas(
        json!({"work":{"progress":[100,200]}}),
        json!([{"rect":[0,0,{"bind":"work","map":[0,20]},8],
            "fill":{"bind":"work","pick":["#123456","$accent"]}}]),
    );
    for (now, width, fill, next) in [
        (99, 0.0, Color::Rgba([0x12, 0x34, 0x56, 255]), Some(3500)),
        (100, 0.0, Color::Rgba([0x12, 0x34, 0x56, 255]), Some(2500)),
        (125, 5.0, Color::Rgba([0x12, 0x34, 0x56, 255]), Some(2500)),
        (150, 10.0, Color::Token(Token::Accent), Some(2500)),
        (200, 20.0, Color::Token(Token::Accent), None),
        (201, 20.0, Color::Token(Token::Accent), None),
    ] {
        let (result, delay) = canvas.evaluate(seconds(now), 2.0);
        assert!(
            matches!(&result.nodes[0].kind, Kind::Rect { rect, .. } if rect[2].initial() == width)
        );
        assert_eq!(result.nodes[0].style.fill, Some(Paint::Solid(fill)));
        assert_eq!(delay, next.map(Duration::from_millis));
    }
}

#[test]
fn sdk_spike_canvas_only_schedules_used_bindings_and_keeps_metadata() {
    let canvas = canvas(
        json!({
            "now":{"now":"HH:mm","zone":"Asia/Shanghai"},
            "unused":{"now":"HH:mm:ss","zone":"Asia/Shanghai"}
        }),
        json!([{"text":"{now} {unknown}","at":[0,12],"size":14,"id":"watch",
            "roll":true,"animate":{"ms":220},"hover":{"opacity":1}}]),
    );
    let (result, delay) = canvas.evaluate(utc("2026-11-24T04:01:20.250Z"), 1.0);
    assert_eq!(delay, Some(Duration::from_millis(39_750)));
    let node = &result.nodes[0];
    assert!(matches!(&node.kind, Kind::Text { text, .. } if text == "12:01 {unknown}"));
    assert_eq!(node.id.as_deref(), Some("watch"));
    assert!(node.roll && node.animate.is_some() && node.hover.is_some());
    drop(result);

    let static_canvas = Canvas {
        nodes: Vec::new(),
        ..canvas
    };
    let (result, delay) = static_canvas.evaluate(seconds(120), 1.0);
    assert!(matches!(result, Cow::Borrowed(_)));
    assert_eq!(delay, None);
}

#[test]
fn sdk_spike_canvas_numeric_countdown_advances_each_second_and_clamps() {
    let canvas = canvas(
        json!({"left":{"countdown":{"target_at":30_000,"format":"d天 HH:mm"}}}),
        json!([
            {"rect":[0,0,{"bind":"left"},10]},
            {"text":"{left}","at":[0,24],"size":{"bind":"left"}}
        ]),
    );
    let (result, delay) = canvas.evaluate(seconds(100), 1.0);
    assert!(
        matches!(&result.nodes[0].kind, Kind::Rect { rect, .. } if rect[2].initial() == 2000.0)
    );
    assert!(matches!(&result.nodes[1].kind, Kind::Text { size, .. } if size.initial() == 96.0));
    assert_eq!(delay, Some(Duration::from_secs(1)));
}
