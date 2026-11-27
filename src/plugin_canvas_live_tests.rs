use super::*;
use crate::{
    skin_paint::GLASS_DARK,
    theme::{self, W},
};
use eframe::egui::{pos2, vec2, Event, Pos2, RawInput};
use serde_json::json;

const START: i64 = 1_795_500_000;

fn theme() -> CanvasTheme {
    CanvasTheme {
        palette: Some(GLASS_DARK),
        accent: Some(Color32::from_rgb(169, 141, 235)),
        background: Color32::from_rgb(15, 14, 21),
    }
}

fn frame(
    ctx: &egui::Context,
    canvas: &Arc<Canvas>,
    time: f64,
    revision: u64,
    events: Vec<Event>,
    live: bool,
) -> (egui::FullOutput, LiveResult) {
    let panel = Rect::from_min_size(Pos2::ZERO, vec2(419.0, 58.0 + canvas.height));
    let mut result = LiveResult::default();
    let output = ctx.run(
        RawInput {
            screen_rect: Some(panel),
            time: Some(time),
            predicted_dt: 0.0,
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let mut card = Card::new(ui, Pos2::ZERO, panel);
                    card.live = live;
                    card.fill(0.0, 0.0, 419.0, panel.height(), 18.0, theme().background);
                    card.text(
                        16.0,
                        26.0,
                        "画布实时交互",
                        14.0,
                        W::Bold,
                        Color32::WHITE,
                        crate::paint::H::Left,
                    );
                    result = draw_live(
                        &card,
                        "example.canvas-live-demo",
                        canvas,
                        card.rect(16.0, 42.0, 387.0, canvas.height),
                        theme(),
                        LiveOptions {
                            now: DateTime::from_timestamp_millis(
                                START * 1000 + (time * 1000.0).round() as i64,
                            )
                            .unwrap(),
                            revision,
                            allow_action: true,
                            interactive: true,
                        },
                        |_| None,
                    );
                });
        },
    );
    (output, result)
}

#[test]
fn plugin_canvas_text_inside_clickable_group_keeps_rendering() {
    let canvas = Arc::new(Canvas::parse(json!({
        "height": 85,
        "nodes": [{
            "group": [{"text":"打开详情","at":[8,20],"size":12}],
            "hit":{"action":"show"}
        }]
    })).unwrap());
    let ctx = egui::Context::default();
    theme::load_fonts(&ctx);
    let (out, _) = frame(&ctx, &canvas, 0.0, 1, Vec::new(), true);
    assert!(out.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text() == "打开详情")));
}

#[test]
fn plugin_canvas_hover_hint_stays_inside_the_canvas() {
    let canvas = Arc::new(Canvas::parse(json!({
        "height": 80,
        "nodes": [{"rect":[364,0,23,25], "fill":"$card", "hit":{"action":"copy-note"},
            "hover":{"hint":"复制当前便签"}}]
    })).unwrap());
    let ctx = egui::Context::default();
    theme::load_fonts(&ctx);
    let (output, result) = frame(&ctx, &canvas, 1.0, 1, vec![Event::PointerMoved(pos2(397.0, 50.0))], true);
    let hint = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == "复制当前便签" => Some((shape.clip_rect, text.visual_bounding_rect())),
        _ => None,
    }).expect("按钮的单行提示应可见");
    let area = Rect::from_min_max(pos2(16.0, 42.0), pos2(403.0, 122.0));
    assert!(area.contains_rect(hint.0) && hint.0.contains_rect(hint.1));
    assert!(result.hit.is_none());
    let (output, _) = frame(&ctx, &canvas, 1.1, 1, vec![Event::PointerMoved(pos2(411.0, 50.0))], true);
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text() == "复制当前便签")));
    for hint in ["", "换\n行", &"字".repeat(41)] {
        assert!(Canvas::parse(json!({"height":80,"nodes":[{"rect":[0,0,10,10],"hover":{"hint":hint}}]})).is_err());
    }
}

#[test]
fn plugin_canvas_live_demo_shows_rolling_digits_and_scrolls_inside_the_group() {
    let runtime = rquickjs::Runtime::new().unwrap();
    let js = rquickjs::Context::full(&runtime).unwrap();
    let output: String = js.with(|js| {
        js.eval::<(), _>(include_str!("../spike/examples/canvas-live-demo/main.js"))
            .unwrap();
        js.eval(format!(
            "JSON.stringify(refresh({{now:{}}}).canvas)",
            START * 1000
        ))
        .unwrap()
    });
    let canvas = Arc::new(Canvas::parse(serde_json::from_str(&output).unwrap()).unwrap());
    let ctx = egui::Context::default();
    theme::load_fonts(&ctx);
    let mut raster = crate::preview_raster::Raster::new(419, 278);
    let mut consume = |output: &egui::FullOutput| {
        raster.update_textures(&output.textures_delta);
        raster.free_textures(&output.textures_delta);
    };
    for time in [0.0, 0.03] {
        let (output, result) = frame(&ctx, &canvas, time, 1, vec![], true);
        assert!(!result.animating);
        consume(&output);
    }
    let (output, result) = frame(&ctx, &canvas, 1.0, 1, vec![], true);
    assert!(result.animating);
    assert_eq!(result.repaint_after, Some(Duration::from_nanos(16_666_667)));
    consume(&output);
    let (output, result) = frame(&ctx, &canvas, 1.12, 1, vec![], true);
    assert!(result.animating);
    let has_roll_clip = output.shapes.iter().any(|shape| {
        shape.clip_rect.width() < 22.0 && matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().starts_with(|ch: char| ch.is_ascii_digit()))
    });
    assert!(has_roll_clip);
    consume(&output);
    let (output, result) = frame(
        &ctx,
        &canvas,
        1.4,
        1,
        vec![
            Event::PointerMoved(pos2(270.0, 187.0)),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -6.0),
                modifiers: egui::Modifiers::NONE,
            },
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -6.0),
                modifiers: egui::Modifiers::NONE,
            },
        ],
        true,
    );
    assert!(result.animating);
    assert_eq!(
        result.scroll_area,
        Some(Rect::from_min_size(pos2(222.0, 109.0), vec2(173.0, 150.0)))
    );
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "记录 7" && text.pos.y < 260.0)));
    raster.update_textures(&output.textures_delta);
    if let Some(path) = std::env::var_os("DAOZAI_CANVAS_SCREENSHOTS").map(std::path::PathBuf::from)
    {
        std::fs::create_dir_all(&path).unwrap();
        let points = ctx.tessellate(output.shapes, output.pixels_per_point);
        raster.buf.fill([0.0; 4]);
        raster.draw(&points, output.pixels_per_point);
        let mut rgba = Vec::new();
        raster.to_rgba8(&mut rgba);
        image::RgbaImage::from_raw(419, 278, rgba)
            .unwrap()
            .save(path.join("B-canvas-live.png"))
            .unwrap();
    }
    let (_, hidden) = frame(&ctx, &canvas, 2.0, 1, vec![], false);
    assert_eq!(hidden.repaint_after, None);
    assert!(!hidden.animating);
}

#[test]
fn plugin_canvas_static_frame_and_revision_safe_hit() {
    let canvas = Arc::new(
        Canvas::parse(json!({"height": 80, "nodes": [{
            "rect":[0,0,160,50],"fill":"$accent","hit":{"action":"save"}
        }]}))
        .unwrap(),
    );
    let ctx = egui::Context::default();
    theme::load_fonts(&ctx);
    let pointer = pos2(25.0, 50.0);
    let event = |pressed| Event::PointerButton {
        pos: pointer,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let (_, static_page) = frame(
        &ctx,
        &canvas,
        0.0,
        1,
        vec![Event::PointerMoved(pointer)],
        true,
    );
    assert_eq!(static_page.repaint_after, None);
    frame(&ctx, &canvas, 0.1, 1, vec![event(true)], true);
    let (_, stale) = frame(&ctx, &canvas, 0.2, 2, vec![event(false)], true);
    assert_eq!(stale.hit, None);
    frame(&ctx, &canvas, 0.3, 2, vec![event(true)], true);
    let (_, fresh) = frame(&ctx, &canvas, 0.4, 2, vec![event(false)], true);
    assert_eq!(fresh.hit, Some(Hit::Action("save".into())));
}

#[test]
fn plugin_canvas_host_pet_uses_installed_sheet_and_sleep_stays_idle() {
    let ctx = egui::Context::default();
    theme::load_fonts(&ctx);
    let image = image::RgbaImage::from_pixel(144, 1440, image::Rgba([12, 228, 97, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let mut sheets = std::array::from_fn(|_| None);
    sheets[crate::pet_expressions::Expression::Praised as usize] = Some(bytes.into_inner());
    let sheets = Arc::new(sheets);
    for (mood, animated) in [("praised", true), ("sleeping", false)] {
        let canvas = Arc::new(
            Canvas::parse(json!({"height":80,"nodes":[
                {"pet":{"id":"runner","at":[45,40],"size":28,"mood":mood,"motion":"none"}}
            ]}))
            .unwrap(),
        );
        let panel = Rect::from_min_size(Pos2::ZERO, vec2(180.0, 80.0));
        let mut result = LiveResult::default();
        let out = ctx.run(
            RawInput {
                screen_rect: Some(panel),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let c = Card::new(ui, Pos2::ZERO, panel);
                        result = draw_live_with_pet(
                            &c,
                            "pet-sheet",
                            &canvas,
                            panel,
                            CanvasTheme {
                                palette: None,
                                background: Color32::BLACK,
                                accent: None,
                            },
                            LiveOptions {
                                now: DateTime::from_timestamp(START, 0).unwrap(),
                                revision: 1,
                                allow_action: false,
                                interactive: false,
                            },
                            Some(Arc::clone(&sheets)),
                            |_| None,
                        );
                    });
            },
        );
        if animated {
            assert!(out
                .textures_delta
                .set
                .iter()
                .any(|(_, delta)| match &delta.image {
                    egui::ImageData::Color(image) =>
                        image.size == [144, 1440]
                            && image.pixels.first() == Some(&Color32::from_rgb(12, 228, 97)),
                    _ => false,
                }));
        } else {
            assert!(out
                .shapes
                .iter()
                .any(|entry| matches!(entry.shape, egui::Shape::Path(_))));
            assert_eq!(result.repaint_after, None);
        }
    }
}
