use super::*;
use crate::quota::{relay, Account, Provider};
use serde_json::json;
use std::{
    fs,
    time::{Duration, Instant},
};

fn install(app: &mut App, id: &str, name: &str, quota: serde_json::Value) {
    let folder = crate::model::config_dir().join(id);
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("cover.png"),
        include_bytes!("../spike/examples/payday/cover.png"),
    )
    .unwrap();
    fs::write(
        folder.join("main.js"),
        format!("function refresh() {{ return {{quota:{quota}}}; }}"),
    )
    .unwrap();
    let manifest = folder.join("plugin.json");
    fs::write(&manifest, json!({
        "id":id, "name":name, "author":"验收", "version":"1.0.0", "api":1,
        "runtime":"js", "entry":"main.js", "cover":"cover.png", "owner":"quota",
        "capabilities":["quota.source","timer"], "dataVersion":1, "permissions":{}, "settings":[]
    }).to_string()).unwrap();
    app.plugins.install(&manifest).unwrap();
    app.plugins.enable(id, true).unwrap();
}

fn shows(h: &H, label: &str) -> bool {
    h.shapes
        .iter()
        .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label))
}

#[test]
fn plugin_quota_uses_the_existing_panel_without_changing_builtin_accounts() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let mut h = H::new();
    let baseline = "native-account";
    h.app.quota.config.accounts.push(Account {
        id: baseline.into(),
        alias: "内置来源".into(),
        provider: Provider::Sub2Api,
        source: "https://example.invalid".into(),
        slot: None,
        enabled: true,
        pinned: false,
    });
    h.app.quota.config.selected = baseline.into();
    h.app.quota.rows.insert(
        baseline.into(),
        Ok(relay::parse_sub2api(&json!({
            "mode":"unrestricted","isValid":true,"unit":"USD","balance":23
        }))
        .unwrap()),
    );
    let original = h.app.quota.config.clone();
    install(
        &mut h.app,
        "example.panel-first",
        "模拟积分源",
        json!({
            "windows":[{"label":"当月模拟积分","remaining":680,"limit":1000,"unit":"积分"}]
        }),
    );
    install(
        &mut h.app,
        "example.panel-second",
        "模拟周期源",
        json!({
            "windows":[{"label":"本周模拟额度","remaining_pct":42}]
        }),
    );
    h.app.settings.visible_pages[crate::app::QUOTA_PAGE] = true;
    h.app.show_page(crate::app::QUOTA_PAGE);
    let deadline = Instant::now() + Duration::from_secs(4);
    while h.app.plugins.quota_sources().count() != 2 {
        h.app
            .plugins
            .tick_features(false, false, true, "2026-10-28");
        assert!(Instant::now() < deadline, "模拟额度未加载");
        std::thread::sleep(Duration::from_millis(2));
    }
    h.hover(H::local(230.0, 65.0), 0.8);
    assert!(shows(&h, "内置来源"));
    assert!(shows(&h, "模拟积分源"));
    h.click_text("模拟积分源");
    h.hover(H::local(230.0, 65.0), 0.25);
    assert!(shows(&h, "当月模拟积分"));
    assert!(shows(&h, "680 / 1000 积分"));
    assert_eq!(h.app.quota.config, original);
    assert_eq!(h.app.quota.rows.len(), 1);
    if let Some(path) = std::env::var_os("DAOZAI_PLUGIN_QUOTA_PREVIEW") {
        let path = std::path::PathBuf::from(path);
        fs::create_dir_all(&path).unwrap();
        let mut raster = feature_raster();
        shoot_feature(
            &mut h,
            &mut raster,
            &path,
            "插件额度面板",
            25,
            Some(H::local(230.0, 65.0)),
            "2026-10-28",
        );
    }

    h.app.plugins.enable("example.panel-first", false).unwrap();
    h.frame(vec![]);
    assert!(!shows(&h, "模拟积分源"));
    assert!(shows(&h, "23 USD"));
    assert!(shows(&h, "模拟周期源"));
    h.click_text("模拟周期源");
    h.hover(H::local(230.0, 65.0), 0.25);
    assert!(shows(&h, "本周模拟额度"));
    assert!(shows(&h, "42%"));
    assert_eq!(h.app.quota.config, original);
}
