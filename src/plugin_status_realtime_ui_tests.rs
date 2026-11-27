use super::*;
use serde_json::json;

struct Fixture {
    ctx: egui::Context,
    status: PluginStatus,
    layout: StatusLayout,
}

impl Fixture {
    fn new(status: serde_json::Value, layout: serde_json::Value) -> Self {
        let ctx = egui::Context::default();
        theme::load_fonts(&ctx);
        Self {
            ctx,
            status: serde_json::from_value(status).unwrap(),
            layout: serde_json::from_value(layout).unwrap(),
        }
    }

    fn draw(
        &self,
        time: f64,
        events: Vec<egui::Event>,
        live: bool,
        blocked: bool,
    ) -> egui::FullOutput {
        let canvas =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(419.0, MAX_PAGE_HEIGHT));
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(canvas),
                time: Some(time),
                predicted_dt: 0.0,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let mut c = Card::new(ui, egui::Pos2::ZERO, canvas);
                        c.live = live;
                        draw_status_page(
                            &c,
                            "test.status",
                            Some(&self.status),
                            StatusHealth::Ready,
                            Some(&self.layout),
                            StatusBackdrop {
                                palette: None,
                                island: Color32::BLACK,
                            },
                            blocked,
                            None,
                        );
                    });
            },
        )
    }
}

fn click(fixture: &Fixture, mouse: egui::Pos2) -> egui::FullOutput {
    let event = |pressed| egui::Event::PointerButton {
        pos: mouse,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    fixture.draw(
        0.1,
        vec![egui::Event::PointerMoved(mouse), event(true)],
        true,
        false,
    );
    fixture.draw(0.2, vec![event(false)], true, false)
}

fn painted(output: &egui::FullOutput, value: &str) -> bool {
    output.shapes.iter().any(
        |clipped| matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == value),
    )
}

#[test]
fn plugin_status_sensitive_values_are_masked_until_clicked_and_for_private_frames() {
    let fixture = Fixture::new(
        json!({
            "state":"running","label":"实时",
            "metrics":[{"label":"工资","value":"¥4321.25","sensitive":true}],
            "items":[{"label":"私密便签","detail":"便签正文","sensitive":true}],
            "counter":{"start_value":9.5,"per_second":0,"start_at":0,"decimals":2,"prefix":"¥","sensitive":true}
        }),
        json!({
            "components":[{"type":"metrics"},{"type":"list"},{"type":"counter"}]
        }),
    );
    let initial = fixture.draw(0.0, vec![], true, false);
    assert!(painted(&initial, MASKED_VALUE));
    for text in ["¥4321.25", "私密便签", "便签正文", "¥9.50"] {
        assert!(!painted(&initial, text), "{text}");
    }

    let revealed = click(&fixture, egui::pos2(90.0, 58.0));
    assert!(painted(&revealed, "¥4321.25"));
    for (env, exclude) in [
        (
            crate::app::Env {
                screen_locked: true,
                ..Default::default()
            },
            false,
        ),
        (
            crate::app::Env {
                recording: true,
                ..Default::default()
            },
            false,
        ),
        (crate::app::Env::default(), true),
    ] {
        let blocked = sensitive_values_blocked(exclude, &env);
        assert!(blocked);
        let output = fixture.draw(0.3, vec![], true, blocked);
        assert!(painted(&output, MASKED_VALUE));
        assert!(!painted(&output, "¥4321.25"));
    }
    let masked = fixture.draw(30.3, vec![], true, false);
    assert!(!painted(&masked, "¥4321.25"));
}

#[test]
fn plugin_status_sensitive_counter_is_hidden_after_its_rate_changes() {
    let status = |rate| json!({
        "state":"running","label":"实时",
        "counter":{"start_value":9.5,"per_second":rate,"start_at":0,"decimals":2,"prefix":"¥","sensitive":true}
    });
    let mut fixture = Fixture::new(status(0), json!({"components":[{"type":"counter"}]}));
    assert!(painted(&fixture.draw(0.0, vec![], true, false), MASKED_VALUE));
    assert!(painted(&click(&fixture, egui::pos2(90.0, 58.0)), "¥9.50"));
    fixture.status = serde_json::from_value(status(1)).unwrap();
    assert!(painted(&fixture.draw(0.3, vec![], true, false), MASKED_VALUE));
}

#[test]
fn plugin_status_sensitive_list_requires_reveal_before_actions() {
    let fixture = Fixture::new(
        json!({
            "state":"running","label":"便签",
            "items":[{"label":"私密便签","action":"delete-1","copy":"私密正文","sensitive":true}]
        }),
        json!({"components":[{"type":"list"}]}),
    );
    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(419.0, MAX_PAGE_HEIGHT));
    let frame = |time, events, blocked| {
        let mut clicked = None;
        let output = fixture.ctx.run(
            egui::RawInput { screen_rect: Some(canvas), time: Some(time), events, ..Default::default() },
            |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    let card = Card::new(ui, egui::Pos2::ZERO, canvas);
                    let mut form = std::collections::BTreeMap::new();
                    let mut interaction = Interaction::new("test.status", 1, &mut form, true);
                    draw_interactive_status_page(
                        &card, "test.status", &fixture.status, Some(&fixture.layout),
                        StatusBackdrop { palette: None, island: Color32::BLACK }, blocked, &mut interaction, None,
                    );
                    clicked = interaction.finish().1;
                });
            },
        );
        (output, clicked)
    };
    let click = |time, pos, blocked| {
        let event = |pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
        };
        assert!(frame(time, vec![egui::Event::PointerMoved(pos), event(true)], blocked).1.is_none());
        frame(time + 0.05, vec![event(false)], blocked)
    };

    let inactive = frame(0.0, vec![], false).0;
    assert!(painted(&inactive, MASKED_VALUE));
    assert!(!painted(&inactive, "复制"));
    let copy = egui::pos2(340.0, 58.0);
    let (shown, first_click) = click(0.1, copy, false);
    assert!(painted(&shown, "私密便签") && first_click.is_none());
    assert!(painted(&frame(0.2, vec![], false).0, "复制"));
    assert!(matches!(click(0.3, copy, false).1, Some(ClickTarget::CopyItem(0))));
    let body = egui::pos2(95.0, 58.0);
    assert!(matches!(click(0.4, body, false).1, Some(ClickTarget::Item(0))));

    let (blocked, clicked) = click(0.5, copy, true);
    assert!(painted(&blocked, MASKED_VALUE) && clicked.is_none());
    assert!(!painted(&blocked, "复制"));
    assert!(painted(&frame(31.0, vec![], false).0, MASKED_VALUE));
}

#[test]
fn plugin_status_real_time_requests_repaint_only_for_visible_values() {
    let fixture = Fixture::new(
        json!({
            "state":"running","label":"时钟",
            "clock":[
                {"label":"时钟甲","utc_offset_minutes":480,"format":"HH:mm:ss"},
                {"label":"时钟乙","utc_offset_minutes":540,"format":"HH:mm"}
            ]
        }),
        json!({
            "components":[{"type":"clock"},{"type":"clock"},{"type":"clock"}]
        }),
    );
    fixture.draw(0.0, vec![], true, false);
    fixture.draw(0.1, vec![], true, false);
    let active = fixture.draw(0.2, vec![], true, false);
    assert!(painted(&active, "时钟甲") && painted(&active, "时钟乙"));
    let delay = active.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    assert!(
        delay <= Duration::from_secs(1) && delay > Duration::ZERO,
        "{delay:?}"
    );
    let inactive = fixture.draw(0.3, vec![], false, false);
    assert_eq!(
        inactive.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
        Duration::MAX
    );
}
