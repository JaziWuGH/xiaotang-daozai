use super::*;
use crate::plugin_status::StatusComponent;
use eframe::egui;

const ID: &str = "example.sticky-notes";
const TODAY: &str = "2026-11-23";
const MANIFEST: &str = include_str!("examples/sticky-notes/plugin.json");

struct Fixture {
    root: PathBuf,
    manifest: PathBuf,
}

impl Fixture {
    fn new() -> Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("daozai-actions-{}-{stamp}", std::process::id()));
        fs::create_dir(&root)?;
        let manifest = root.join("plugin.json");
        fs::write(&manifest, MANIFEST)?;
        fs::write(
            root.join("cover.png"),
            include_bytes!("examples/sticky-notes/cover.png"),
        )?;
        fs::write(
            root.join("main.js"),
            include_str!("examples/sticky-notes/main.js"),
        )?;
        Ok(Self { root, manifest })
    }

    fn host(&self) -> Result<LocalPlugins> {
        let mut host = LocalPlugins::open(self.root.join("installed"));
        host.install(&self.manifest)?;
        host.enable(ID, true)?;
        settle(&mut host);
        Ok(host)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn settle(host: &mut LocalPlugins) {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        host.tick(true, true, TODAY);
        if host.jobs.is_empty() {
            break;
        }
        assert!(Instant::now() < until, "等待插件刷新超时");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn revision(host: &LocalPlugins) -> u64 {
    host.status[ID].as_ref().unwrap().0
}

fn shown(host: &LocalPlugins) -> &str {
    &host.status_page(ID).unwrap().1.unwrap().label
}

fn queue_note(host: &mut LocalPlugins, current: u64, note: &str) -> bool {
    host.interactions
        .forms
        .insert(ID.into(), Values::from([("note".into(), note.into())]));
    host.submit_action(
        &egui::Context::default(),
        ID,
        current,
        ClickTarget::Button(0),
    )
}

#[test]
fn canvas_action_dispatch_discards_the_previous_revision() -> Result<()> {
    let fixture = Fixture::new()?;
    let id = "example.canvas-live-demo";
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("spike/examples/canvas-live-demo/plugin.json");
    let mut host = LocalPlugins::open(fixture.root.join("installed"));
    host.install(&manifest)?;
    host.enable(id, true)?;
    settle(&mut host);
    let current = host.status[id].as_ref().unwrap().0;
    let ctx = egui::Context::default();
    assert!(host.submit_action(&ctx, id, current, ClickTarget::Canvas(Hit::Action("pause".into()))));
    settle(&mut host);
    assert_eq!(host.records[id].storage["paused"], true);
    assert!(!host.submit_action(&ctx, id, current, ClickTarget::Canvas(Hit::Action("resume".into()))));
    Ok(())
}

#[test]
fn invalid_manifest_combinations_and_interaction_fields_are_rejected() -> Result<()> {
    let manifest = Manifest::decode(MANIFEST)?;
    let layout = manifest.status_layout.unwrap();
    assert!(layout.components.contains(&StatusComponent::Actions {}));
    assert_eq!(
        layout.style.density,
        crate::plugin_status::StatusDensity::Comfortable
    );
    let base: Value = serde_json::from_str(MANIFEST)?;
    for (field, value) in [
        (
            "capabilities",
            json!(["action", "event", "notice", "storage"]),
        ),
        (
            "capabilities",
            json!(["status.page", "event", "notice", "storage"]),
        ),
        (
            "capabilities",
            json!(["status.page", "action", "notice", "storage"]),
        ),
        ("events", json!(["unknown"])),
        ("events", json!([])),
        ("events", json!(["focus.finished", "focus.finished"])),
        ("network", json!({"hosts":["127.0.0.1"]})),
        ("network", json!({"hosts":["*.example.com"]})),
        ("network", json!({"hosts":["example.com","EXAMPLE.COM"]})),
    ] {
        let mut invalid = base.clone();
        invalid[field] = value;
        assert!(Manifest::decode(&invalid.to_string()).is_err(), "{invalid}");
    }
    assert!(capabilities::check("js", "tasks", &["card".into(), "action".into()]).is_err());
    assert!(capabilities::check(
        "js",
        "tasks",
        &["card".into(), "status.page".into(), "action".into()],
    )
    .is_ok());

    for (field, value) in [
        ("actions", json!([{"id":"add","label":"一二三四五六七"}])),
        ("actions", json!(vec![json!({"id":"add","label":"加"}); 4])),
        (
            "actions",
            json!([{"id":"add","label":"加"},{"id":"add","label":"增加"}]),
        ),
        ("fields", json!([{"key":"note","label":"备注","max":201}])),
        (
            "fields",
            json!([{"key":"note","label":"备注","max":3,"value":"换行\n"}]),
        ),
        (
            "fields",
            json!([{"key":"note","label":"备注","max":1,"value":"两字"}]),
        ),
        (
            "fields",
            json!(vec![json!({"key":"note","label":"备注","max":3}); 4]),
        ),
        (
            "actions",
            json!([{"id":"copy","label":"复制","copy":"测".repeat(2001)}]),
        ),
        ("items", json!([{"label":"项目","copy":"测".repeat(2001)}])),
        (
            "items",
            json!([{"label":"项目","open":"https://example.com","copy":"文字"}]),
        ),
    ] {
        let mut output = json!({"state":"idle","label":"就绪"});
        output[field] = value;
        assert!(!serde_json::from_value::<PluginStatus>(output)?.valid());
    }
    Ok(())
}

#[test]
fn stale_buttons_and_late_results_are_discarded() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut host = fixture.host()?;
    let old = revision(&host);
    assert!(queue_note(&mut host, old, "初始便签"));
    assert!(!queue_note(&mut host, old, "重复点击"));
    settle(&mut host);
    assert_eq!(shown(&host), "便签 1 / 8");
    assert!(!queue_note(&mut host, old, "旧输出"));

    let current = revision(&host);
    assert!(queue_note(&mut host, current, "停用前点击"));
    host.tick(true, true, TODAY);
    assert!(host.jobs.contains_key(ID));
    host.enable(ID, false)?;
    settle(&mut host);
    assert!(!queue_note(&mut host, current, "插件已停用"));
    assert_eq!(
        host.records[ID].storage["items"].as_array().unwrap().len(),
        1
    );
    host.enable(ID, true)?;
    settle(&mut host);
    assert!(!queue_note(&mut host, current, "停用前输出"));

    let current = revision(&host);
    assert!(queue_note(&mut host, current, "设置修改前点击"));
    host.tick(true, true, TODAY);
    host.save(ID, Values::new())?;
    settle(&mut host);
    assert_eq!(
        host.records[ID].storage["items"].as_array().unwrap().len(),
        1
    );
    assert!(!queue_note(&mut host, current, "设置修改前输出"));
    Ok(())
}

#[test]
fn duplicate_focus_event_is_discarded_and_disabled_plugins_do_not_replay_it() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut host = fixture.host()?;
    let finished = FocusFinished::new(1_700_000_000, 25);
    host.queue_event(finished.clone(), true, true, false);
    host.queue_event(finished.clone(), true, true, false);
    settle(&mut host);
    host.queue_event(finished, true, true, false);
    settle(&mut host);
    assert_eq!(shown(&host), "便签 1 / 8");
    assert_eq!(host.records[ID].storage["items"][0]["text"], "完成一次专注");

    host.set_status_page_enabled(ID, false)?;
    let paused = FocusFinished::new(1_700_001_500, 25);
    host.queue_event(paused.clone(), true, true, false);
    host.set_status_page_enabled(ID, true)?;
    host.queue_event(paused, true, true, false);
    settle(&mut host);
    assert_eq!(shown(&host), "便签 1 / 8");

    let current = revision(&host);
    assert!(queue_note(&mut host, current, "测试便签"));
    host.tick(true, true, TODAY);
    host.queue_event(FocusFinished::new(1_700_003_000, 25), true, true, false);
    settle(&mut host);
    assert_eq!(shown(&host), "便签 3 / 8");
    Ok(())
}

#[cfg(windows)]
#[test]
fn notices_respect_disable_deduplication_cooldown_and_restart() -> Result<()> {
    use crate::plugin_notice::Event;
    let fixture = Fixture::new()?;
    let mut host = fixture.host()?;
    let give = |host: &mut LocalPlugins, id: &str| -> Result<()> {
        host.interactions.notices.insert(
            ID.into(),
            serde_json::from_value(json!({
                "id": id, "title": "便签已满", "body": "已记录 8 条便签。",
            }))?,
        );
        Ok(())
    };
    give(&mut host, "today")?;
    assert!(host.take_script_notices(false, 0).is_empty());
    for index in 1..=8 {
        let current = revision(&host);
        assert!(queue_note(&mut host, current, &format!("便签 {index}")));
        settle(&mut host);
    }
    let first = host.take_script_notices(true, 0);
    assert!(
        matches!(&first[..], [Event::Completed(notice)] if notice.title.as_deref() == Some("便签已满"))
    );
    give(&mut host, "next")?;
    assert!(host.take_script_notices(true, 59_999).is_empty());
    give(&mut host, "notes-full")?;
    assert!(host.take_script_notices(true, 60_000).is_empty());
    give(&mut host, "next")?;
    assert_eq!(host.take_script_notices(true, 60_000).len(), 1);

    let mut reopened = LocalPlugins::open(fixture.root.join("installed"));
    give(&mut reopened, "next")?;
    assert!(reopened.take_script_notices(true, 120_000).is_empty());
    Ok(())
}

#[test]
fn status_page_egui_button_submits_form_and_updates_output() -> Result<()> {
    use super::super::management_test::{click, click_at, frame, has_text};
    use crate::paint::Card;

    let fixture = Fixture::new()?;
    let mut host = fixture.host()?;
    let current = revision(&host);
    let ctx = egui::Context::default();
    crate::theme::load_fonts(&ctx);
    let mut draw = |ui: &mut egui::Ui| {
        let card = Card::new(ui, egui::Pos2::ZERO, ui.max_rect());
        host.draw_status_controls(
            &card,
            ID,
            crate::plugin_status_ui::StatusBackdrop { palette: None, island: eframe::egui::Color32::BLACK },
            false,
        );
    };
    let before = frame(&ctx, vec![], &mut draw);
    assert!(has_text(&before, "便签 0 / 8"));
    let note_id = egui::Id::new(("plugin-field", ID, current, "note", true)).with("value");
    let pos = ctx.read_response(note_id).unwrap().rect.center();
    click_at(&ctx, pos, &mut draw);
    assert!(ctx.memory(|memory| memory.has_focus(note_id)));
    frame(
        &ctx,
        vec![egui::Event::Text("今天整理素材".into())],
        &mut draw,
    );
    click(&ctx, "添加", draw);
    settle(&mut host);

    let after = frame(&ctx, vec![], |ui| {
        let card = Card::new(ui, egui::Pos2::ZERO, ui.max_rect());
        host.draw_status_controls(
            &card,
            ID,
            crate::plugin_status_ui::StatusBackdrop { palette: None, island: eframe::egui::Color32::BLACK },
            false,
        );
    });
    assert!(has_text(&after, "便签 1 / 8"));
    assert!(has_text(&after, "今天整理素材"));
    assert!(has_text(&after, "复制"));
    assert_eq!(host.records[ID].storage["items"][0]["text"], "今天整理素材");
    let record: Record = serde_json::from_str(&fs::read_to_string(host.path(ID))?)?;
    assert_eq!(record.storage["items"][0]["text"], "今天整理素材");
    click(&ctx, "今天整理素材", |ui| {
        let card = Card::new(ui, egui::Pos2::ZERO, ui.max_rect());
        host.draw_status_controls(
            &card,
            ID,
            crate::plugin_status_ui::StatusBackdrop { palette: None, island: eframe::egui::Color32::BLACK },
            false,
        );
    });
    settle(&mut host);
    assert_eq!(shown(&host), "便签 0 / 8");
    assert_eq!(host.records[ID].storage["items"], json!([]));
    Ok(())
}

#[test]
fn canvas_fields_stay_in_the_canvas_and_submit_the_focused_draft() -> Result<()> {
    use super::super::management_test::{click_at, frame, has_text};
    use crate::paint::Card;

    let fixture = Fixture::new()?;
    let mut manifest: Value = serde_json::from_str(MANIFEST)?;
    manifest["capabilities"].as_array_mut().unwrap().push(json!("status.canvas"));
    manifest["statusLayout"] = json!({
        "components": [{"type": "canvas"}, {"type": "fields"}, {"type": "actions"}],
        "style": {"surface": "plain"},
        "canvasFields": [
            {"key": "note", "rect": [4, 207, 130, 22], "placeholder": "记一笔…", "action": "add"},
            {"key": "search", "rect": [202, 7, 182, 24], "placeholder": "搜索便签", "action": "search"}
        ]
    });
    fs::write(&fixture.manifest, manifest.to_string())?;
    fs::write(fixture.root.join("main.js"), r#"
function refresh({storage}) {
    return {canvas: {height: 230, nodes: [{rect:[0,0,387,230],fill:"$card"}]},
        storage,
        status: {
            state:"idle",label:"便签",
            fields: [{key:"note",label:"新便签",max:80}, {key:"search",label:"搜索便签",max:80}],
            actions: [{id:"add",label:"添加"}, {id:"search",label:"搜索"}]
        }
    };
}
function action({action}) {
    return refresh({storage: {id: action.id, form: action.form}});
}
"#)?;
    let mut host = fixture.host()?;
    let current = revision(&host);
    let ctx = egui::Context::default();
    crate::theme::load_fonts(&ctx);
    let mut draw = |ui: &mut egui::Ui| {
        let card = Card::new(ui, egui::Pos2::ZERO, ui.max_rect());
        host.draw_status_controls(
            &card, ID,
            crate::plugin_status_ui::StatusBackdrop { palette: None, island: egui::Color32::BLACK },
            false,
        );
    };
    let page = frame(&ctx, vec![], &mut draw);
    for label in ["新便签", "添加", "搜索"] {
        assert!(!has_text(&page, label), "{label} 不应在画布以外重复显示");
    }
    let rect = |key| {
        let id = egui::Id::new(("plugin-field", ID, current, key, true)).with("value");
        ctx.read_response(id).unwrap().rect
    };
    let note = rect("note");
    let search = rect("search");
    assert!(note.left() < search.left());
    assert!((note.top() - search.top() - 200.0).abs() < 1.0);
    click_at(&ctx, search.center(), &mut draw);
    frame(&ctx, vec![
        egui::Event::Ime(egui::ImeEvent::Enabled),
        egui::Event::Ime(egui::ImeEvent::Preedit("素材".into())),
        egui::Event::Ime(egui::ImeEvent::Commit("素材".into())),
    ], &mut draw);
    frame(&ctx, vec![egui::Event::Key {
        key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false,
        modifiers: egui::Modifiers::NONE,
    }], draw);
    settle(&mut host);
    assert_eq!(host.records[ID].storage["id"], "search");
    assert_eq!(host.records[ID].storage["form"]["search"], "素材");
    Ok(())
}

#[test]
fn host_actions_validate_links_and_dispatch_without_running_script() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut host = fixture.host()?;
    let current = revision(&host);
    assert!(queue_note(&mut host, current, "记得整理素材"));
    settle(&mut host);

    let status: PluginStatus = serde_json::from_value(json!({
        "state": "idle", "label": "就绪",
        "actions": [{"id": "web", "label": "打开", "open": "https://example.com/help#tips"}],
        "items": [{"label": "便签", "action": "note-1", "copy": "记得整理素材"}]
    }))?;
    assert!(status.valid());
    let mut manifest: Value = serde_json::from_str(MANIFEST)?;
    let package = host.records.get_mut(ID).unwrap().package.as_mut().unwrap();
    assert!(package.manifest.check_action_links(&status).is_err());
    manifest["network"] = json!({"hosts": ["example.com"]});
    package.manifest = Manifest::decode(&manifest.to_string())?;
    assert!(package.manifest.check_action_links(&status).is_ok());
    for url in [
        "http://example.com/help",
        "https://example.com.evil.invalid",
        "https://example.com:444",
        "https://user@example.com",
        "https:///example.com",
    ] {
        let mut invalid = status.clone();
        invalid.actions[0].open = Some(url.into());
        assert!(
            package.manifest.check_action_links(&invalid).is_err(),
            "{url}"
        );
        invalid.items[0].open = invalid.actions[0].open.take();
        assert!(
            package.manifest.check_action_links(&invalid).is_err(),
            "{url}"
        );
    }
    let current = revision(&host);
    host.status.insert(ID.into(), Ok((current, status)));
    let ctx = egui::Context::default();
    let out = ctx.run(Default::default(), |ctx| {
        assert!(host.submit_action(ctx, ID, current, ClickTarget::Button(0)));
        assert!(host.submit_action(ctx, ID, current, ClickTarget::CopyItem(0)));
        assert!(!host.submit_action(ctx, ID, current - 1, ClickTarget::CopyItem(0)));
    });
    assert!(!host.interactions.has_call(ID));
    assert!(out.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::OpenUrl(url) if url.url == "https://example.com/help#tips")
    }));
    assert!(out.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == "记得整理素材")
    }));
    assert!(host.submit_action(&ctx, ID, current, ClickTarget::Item(0)));
    settle(&mut host);
    assert_eq!(host.records[ID].storage["items"], json!([]));
    Ok(())
}
