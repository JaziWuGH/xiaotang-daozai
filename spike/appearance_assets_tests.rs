use super::*;
use crate::pet_expressions::Expression;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" fill="#336699"/></svg>"##;

struct Fixtures(PathBuf);

impl Fixtures {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "daozai-appearance-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn pack(&self, id: &str, kind: Kind, source: Value) -> Result<PathBuf> {
        let manifest = self.0.join(id).join("plugin.json");
        let dir = manifest.parent().unwrap();
        fs::create_dir(dir)?;
        fs::write(
            dir.join("cover.png"),
            include_bytes!("examples/celadon-skin/cover.png"),
        )?;
        fs::write(dir.join(kind.entry()), source.to_string())?;
        fs::write(&manifest, json!({
            "id": id, "name": "外观资源", "author": "岛仔", "version": "1.0.0", "api": 1,
            "runtime": "assets", "entry": kind.entry(), "cover": "cover.png", "owner": "appearance",
            "capabilities": [kind.capability()], "dataVersion": 1, "permissions": {}, "settings": []
        }).to_string())?;
        Ok(manifest)
    }
}

impl Drop for Fixtures {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn example_skin() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/spike/examples/celadon-skin/plugin.json"
    ))
}

fn example_font() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/spike/examples/font-zh/plugin.json"))
}

#[test]
fn sdk_spike_canvas_font_pack_rejects_invalid_assets() -> Result<()> {
    use super::super::font_assets::{self, Entry};

    for path in ["../font.ttf", "/font.ttf", "C:/font.otf", "font.woff2"] {
        let source = json!({"fonts":[{"name":"字型","file":path}]}).to_string();
        assert!(Entry::decode(&source).is_err(), "{path}");
    }
    let assets = example_font().parent().unwrap();
    let entry = Entry::decode(&fs::read_to_string(assets.join("fonts.json"))?)?;
    let bytes = fs::read(assets.join("ZCOOLKuaiLe-Regular.ttf"))?;
    let license = ["OFL.txt"];
    assert!(font_assets::read(&entry, std::iter::empty(), 0, |_, _| Ok(Vec::new()))
        .err().unwrap().to_string().contains("OFL.txt"));
    assert!(font_assets::read(&entry, license.into_iter(), 0, |_, _| Ok(b"non-font".to_vec()))
        .err().unwrap().to_string().contains("TTF"));
    assert!(font_assets::read(&entry, license.into_iter(), 0, |_, limit| Ok(vec![0; limit as usize + 1]))
        .err().unwrap().to_string().contains("8 MiB"));
    assert!(font_assets::read(&entry, license.into_iter(), font_assets::PACKAGE_LIMIT, |_, _| Ok(bytes.clone()))
        .err().unwrap().to_string().contains("16 MiB"));
    Ok(())
}

#[test]
fn sdk_spike_canvas_font_pack_registers_falls_back_and_survives_restart() -> Result<()> {
    use ab_glyph::Font;
    use crate::theme::{self, W};

    let fixture = Fixtures::new()?;
    let dir = fixture.0.join("installed");
    let id = "example.zcool-kuaile";
    let name = "站酷快乐体";
    let family = egui::FontFamily::Name(format!("plugin:{name}").into());
    let mut host = LocalPlugins::open(dir.clone());
    host.install(example_font())?;
    host.enable(id, true)?;
    drop(host);
    let mut host = LocalPlugins::open(dir);
    let ctx = egui::Context::default();
    theme::load_fonts(&ctx);
    let fallback = theme::font(20.0, W::Regular);
    let source = fs::read(example_font().with_file_name("ZCOOLKuaiLe-Regular.ttf"))?;
    assert_eq!(ab_glyph::FontRef::try_from_slice(&source).unwrap().glyph_id('→').0, 0);
    let _ = ctx.run(Default::default(), |ctx| host.sync_canvas_fonts(ctx));
    let _ = ctx.run(Default::default(), |ctx| {
        let installed = ctx.fonts(|fonts| fonts.families());
        let font = theme::canvas_font(20.0, W::Regular, Some(name), &installed);
        assert_eq!(font.family, family);
        ctx.fonts(|fonts| {
            assert!(fonts.has_glyph(&font, '岛'));
            assert!(fonts.has_glyph(&font, '→'));
            assert_eq!(fonts.glyph_width(&font, '→'), fonts.glyph_width(&fallback, '→'));
            let galley = fonts.layout_no_wrap("岛→".into(), font, egui::Color32::WHITE);
            assert_eq!(galley.text(), "岛→");
        });
    });
    if let Some(dir) = std::env::var_os("DAOZAI_CANVAS_SCREENSHOTS") {
        save_font_preview(&host, &PathBuf::from(dir))?;
    }
    host.enable(id, false)?;
    let _ = ctx.run(Default::default(), |ctx| host.sync_canvas_fonts(ctx));
    let _ = ctx.run(Default::default(), |ctx| {
        let installed = ctx.fonts(|fonts| fonts.families());
        assert!(!installed.contains(&family));
        assert_eq!(theme::canvas_font(20.0, W::Regular, Some(name), &installed), fallback);
    });
    host.enable(id, true)?;
    let _ = ctx.run(Default::default(), |ctx| host.sync_canvas_fonts(ctx));
    host.uninstall(id, false)?;
    let _ = ctx.run(Default::default(), |ctx| host.sync_canvas_fonts(ctx));
    let _ = ctx.run(Default::default(), |ctx| assert!(!ctx.fonts(|fonts| fonts.families()).contains(&family)));
    Ok(())
}

fn save_font_preview(host: &LocalPlugins, dir: &Path) -> Result<()> {
    use crate::{paint::Card, plugin_canvas::Canvas, plugin_canvas_ui};

    let ctx = egui::Context::default();
    crate::theme::load_fonts(&ctx);
    let width = 419;
    let height = 230;
    let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, height as f32));
    let canvas = Arc::new(Canvas::parse(json!({
        "height":172,"nodes":[
            {"rect":[0,0,387,172],"r":12,"fill":"#202A23"},
            {"text":"站酷快乐体","at":[18,66],"size":36,"font":"站酷快乐体","fill":"#F7EACF"},
            {"text":"岛仔画布 · 趋势与进度","at":[18,108],"size":24,"font":"站酷快乐体","fill":"#C8D97B"},
            {"text":"中文至箭头 →","at":[18,145],"size":18,"font":"站酷快乐体","fill":"#F7EACF"}
        ]
    })).map_err(|error| fail(&error))?);
    let first = ctx.run(Default::default(), |ctx| host.sync_canvas_fonts(ctx));
    let output = ctx.run(egui::RawInput { screen_rect: Some(bounds), ..Default::default() }, |ctx| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let card = Card::new(ui, bounds.min, bounds);
            card.painter.rect_filled(bounds, 16.0, egui::Color32::from_rgb(15, 18, 15));
            card.painter.text(egui::pos2(16.0, 21.0), egui::Align2::LEFT_CENTER, "画布字体",
                crate::theme::font(15.0, crate::theme::W::Semibold), egui::Color32::WHITE);
            plugin_canvas_ui::draw(&card, "font-preview", &canvas, card.rect(16.0, 42.0, 387.0, 172.0),
                plugin_canvas_ui::CanvasTheme { palette: None, accent: None, background: egui::Color32::BLACK },
                |_| None);
        });
    });
    let mut raster = crate::preview_raster::Raster::new(width, height);
    raster.update_textures(&first.textures_delta);
    raster.update_textures(&output.textures_delta);
    raster.draw(&ctx.tessellate(output.shapes, output.pixels_per_point), output.pixels_per_point);
    let mut rgba = Vec::new();
    raster.to_rgba8(&mut rgba);
    fs::create_dir_all(dir)?;
    image::save_buffer(dir.join("C-font-zh.png"), &rgba, width as u32, height as u32, image::ColorType::Rgba8)?;
    Ok(())
}

#[cfg(windows)]
#[test]
fn sdk_spike_canvas_font_zip_preserves_assets_and_license() -> Result<()> {
    use super::super::market;

    let fixture = Fixtures::new()?;
    let zip = fixture.0.join("font.zip");
    market::pack(example_font(), &zip)?;
    let mut archive = zip::ZipArchive::new(fs::File::open(&zip)?)?;
    assert_eq!(archive.len(), 5);
    assert!(archive.by_name("OFL.txt")?.size() > 0);
    assert!(archive.by_name("ZCOOLKuaiLe-Regular.ttf")?.size() > 1024 * 1024);
    market::submit(&zip, &fixture.0.join("catalog.json"))?;
    Ok(())
}

#[test]
fn appearance_theme_rejects_invalid_colors_fields_and_size() -> Result<()> {
    for invalid in [
        r##"{"base":"cream","colors":{"accent":"#ff09"}}"##,
        r##"{"base":"cream","colors":{"accent":null}}"##,
        r##"{"base":"cream","colors":{"background":"#112233"}}"##,
        r##"{"base":"cream","colors":{"ink":"#112233"},"script":"alert(1)"}"##,
        r##"{"base":"invalid","colors":{"ink":"#112233"}}"##,
        r##"{"base":"glass_light","colors":{"paper":"#112233"}}"##,
        r##"{"base":"fluid","colors":{"ink":"#112233"}}"##,
    ] {
        assert!(Pack::decode(Kind::Skin, invalid).is_err(), "{invalid}");
    }
    let valid = r##"{"base":"cream","colors":{"accent":"#113355"}}"##;
    let Pack::Skin(skin) = Pack::decode(Kind::Skin, valid)? else {
        panic!("应解析出皮肤包")
    };
    assert_eq!(skin.base, Skin::Cream);
    assert_eq!(
        skin.palette,
        Palette {
            accent: Some([17, 51, 85]),
            ..CREAM
        }
    );
    assert!(Pack::decode(Kind::Skin, &(valid.to_owned() + &" ".repeat(4096))).is_err());
    Ok(())
}

#[test]
fn appearance_package_rejects_scripts_and_invalid_manifest_combinations() -> Result<()> {
    let fixture = Fixtures::new()?;
    let path = fixture.pack(
        "example.skin",
        Kind::Skin,
        json!({"base":"cream","colors":{"accent":"#113355"}}),
    )?;
    let mut host = LocalPlugins::open(fixture.0.join("installed"));
    let script = path.with_file_name("ignored.js");
    fs::write(&script, "function refresh() {}")?;
    assert!(host.install(&path).is_err());
    fs::remove_file(script)?;
    let valid: Value = serde_json::from_slice(&fs::read(&path)?)?;
    for (key, value) in [
        ("owner", json!("pages")),
        ("runtime", json!("js")),
        ("entry", json!("main.js")),
        ("capabilities", json!(["appearance.skin", "settings"])),
        (
            "capabilities",
            json!(["appearance.skin", "appearance.icon"]),
        ),
        ("capabilities", json!(["appearance.skin", "card"])),
        ("permissions", json!({"source.read": "file"})),
    ] {
        let mut invalid = valid.clone();
        invalid[key] = value;
        assert!(Manifest::decode(&invalid.to_string()).is_err(), "{invalid}");
    }
    assert!(host.install(&path).is_ok());
    Ok(())
}

#[test]
fn appearance_selects_one_skin_and_deselects_on_disable_and_uninstall() -> Result<()> {
    let fixture = Fixtures::new()?;
    let path = fixture.pack(
        "example.second-skin",
        Kind::Skin,
        json!({"base":"classic","colors":{"paper":"#222F3B"}}),
    )?;
    let storage = fixture.0.join("installed");
    let mut host = LocalPlugins::open(storage.clone());
    let first = "example.celadon-skin";
    let second = "example.second-skin";
    host.install(example_skin())?;
    host.install(&path)?;
    assert!(host.select_appearance(Kind::Skin, Some(first)).is_err());
    host.enable(first, true)?;
    host.enable(second, true)?;
    assert_eq!(host.appearance_choices(Kind::Skin).count(), 2);
    host.select_appearance(Kind::Skin, Some(first))?;
    assert_eq!(host.active_skin().unwrap().palette.paper, [238, 245, 236]);
    host.select_appearance(Kind::Skin, Some(second))?;
    let mut host = LocalPlugins::open(storage.clone());
    assert_eq!(host.appearance_id(Kind::Skin), Some(second));
    assert_eq!(host.active_skin().unwrap().palette.paper, [34, 47, 59]);
    host.enable(second, false)?;
    assert!(host.active_skin().is_none());
    host.enable(second, true)?;
    assert!(host.active_skin().is_none());
    host.select_appearance(Kind::Skin, Some(second))?;
    host.uninstall(second, false)?;
    assert!(host.active_skin().is_none());
    host.select_appearance(Kind::Skin, Some(first))?;
    host.uninstall(first, true)?;
    assert!(LocalPlugins::open(storage).active_skin().is_none());
    Ok(())
}

#[test]
fn appearance_icons_are_static_and_fall_back_per_module() -> Result<()> {
    for disallowed in [
        r#"<script>alert(1)</script>"#,
        r#"<foreignObject/>"#,
        r#"<image href="https://example.com/image.png"/>"#,
        r#"<path onclick="alert(1)"/>"#,
        r#"<path fill="url(https://example.com/image.svg)"/>"#,
        r#"<style>@import url(https://example.com/icon.css);</style>"#,
    ] {
        let svg = ICON.replace("</svg>", &format!("{disallowed}</svg>"));
        assert!(validate_svg(&svg).is_err(), "{svg}");
    }
    assert!(validate_svg(&(ICON.to_owned() + &" ".repeat(8 * 1024))).is_err());
    assert!(parse_icons(&json!({"icons":{"unknown":ICON}}).to_string()).is_err());
    let fixture = Fixtures::new()?;
    let id = "example.icon";
    let path = fixture.pack(id, Kind::Icon, json!({"icons":{"music":ICON}}))?;
    let mut host = LocalPlugins::open(fixture.0.join("installed"));
    host.install(&path)?;
    host.enable(id, true)?;
    host.select_appearance(Kind::Icon, Some(id))?;
    assert!(host.appearance_icon(crate::app::MUSIC_PAGE, true).is_some());
    assert!(host
        .appearance_icon(crate::app::MUSIC_PAGE, false)
        .is_some());
    assert!(host.appearance_icon(crate::app::FOCUS_PAGE, true).is_none());
    host.enable(id, false)?;
    assert!(host.appearance_icon(crate::app::MUSIC_PAGE, true).is_none());
    Ok(())
}

#[test]
fn appearance_expressions_require_a_bounded_png_atlas_and_keep_other_expressions() -> Result<()> {
    const SHEET: &[u8] = include_bytes!("../assets/pet/expressions/00-wink.png");
    let trailing = [SHEET, b"<script/>"].concat();
    for bad in [
        json!({"expressions":{"unknown":STANDARD.encode(SHEET)}}),
        json!({"expressions":{"wink":"invalid"}}),
        json!({"expressions":{"wink":STANDARD.encode(trailing)}}),
        json!({"expressions":{"wink":STANDARD.encode(include_bytes!("examples/celadon-skin/cover.png"))}}),
        json!({"expressions":{"wink":STANDARD.encode(vec![0;64 * 1024 + 1])}}),
    ] {
        assert!(parse_expressions(&bad.to_string()).is_err());
    }
    let fixture = Fixtures::new()?;
    let id = "example.expressions";
    let path = fixture.pack(
        id,
        Kind::Pet,
        json!({"expressions":{"wink":STANDARD.encode(SHEET)}}),
    )?;
    let mut host = LocalPlugins::open(fixture.0.join("installed"));
    host.install(&path)?;
    host.enable(id, true)?;
    host.select_appearance(Kind::Pet, Some(id))?;
    let sheets = host.appearance_pet().unwrap();
    assert_eq!(sheets[Expression::Wink as usize].as_deref(), Some(SHEET));
    assert!(sheets[Expression::Blush as usize].is_none());
    host.uninstall(id, false)?;
    assert!(host.appearance_pet().is_none());
    Ok(())
}

#[cfg(windows)]
#[test]
fn appearance_market_installs_and_updates_selected_resources() -> Result<()> {
    use super::super::market;
    use std::time::{Duration, Instant};

    let fixture = Fixtures::new()?;
    let id = "example.market-skin";
    let source = fixture.pack(
        id,
        Kind::Skin,
        json!({"base":"cream","colors":{"accent":"#113355"}}),
    )?;
    let catalog = fixture.0.join("catalog.json");
    let mut host = LocalPlugins::open(fixture.0.join("installed"));
    for (version, rgb) in [("1.0.0", [17, 51, 85]), ("1.1.0", [11, 22, 33])] {
        let mut manifest: Value = serde_json::from_slice(&fs::read(&source)?)?;
        manifest["version"] = json!(version);
        fs::write(&source, manifest.to_string())?;
        let [red, green, blue] = rgb;
        let accent = format!("#{red:02x}{green:02x}{blue:02x}");
        fs::write(
            source.with_file_name(Kind::Skin.entry()),
            json!({"base":"cream","colors":{"accent":accent}}).to_string(),
        )?;
        let zip = fixture.0.join(format!("source-{version}.zip"));
        market::pack(&source, &zip)?;
        market::submit(&zip, &catalog)?;
        host.set_market_catalog(&catalog)?;
        host.market_action(false, id)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.market_busy() {
            host.poll_market();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(host.installed(id).is_ok(), "{}", host.notice);
        if version == "1.0.0" {
            host.enable(id, true)?;
            host.select_appearance(Kind::Skin, Some(id))?;
        }
        assert_eq!(host.active_skin().unwrap().palette.accent, Some(rgb));
    }
    Ok(())
}
