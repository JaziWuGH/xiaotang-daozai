use super::*;

#[test]
fn canvas_output_uses_a_larger_limit_only_when_present() {
    let output = |canvas, length| json!({"canvas":canvas, "card":"x".repeat(length)}).to_string();
    let canvas = json!({"height": 40, "nodes": []});
    assert!(script_settings::split_output(&output(Value::Null, 20 * 1024)).is_err());
    assert!(script_settings::split_output(&output(canvas.clone(), 20 * 1024)).is_ok());
    assert!(script_settings::split_output(&output(canvas, 64 * 1024)).is_err());
}

#[test]
fn settings_accept_declared_formats_and_reject_invalid_defaults() -> Result<()> {
    let cases = [
        (json!({"type":"text"}), "北京", " "),
        (json!({"type":"date"}), "2026-12-31", "2026-02-29"),
        (json!({"type":"switch"}), "true", "yes"),
        (
            json!({"type":"choice", "choices":[{"value":"a","label":"甲"},{"value":"b","label":"乙"}]}),
            "b",
            "c",
        ),
        (
            json!({"type":"number", "min":0.0,"max":10.0,"step":0.5,"unit":"元","precision":2}),
            "3.5",
            "NaN",
        ),
        (json!({"type":"time"}), "07:30", "7:30"),
        (json!({"type":"weekdays"}), "1,4,7", "1,1,7"),
        (
            json!({"type":"list", "item":[{"key":"city","label":"城市","type":"text","default":"北京"}],"maxItems":1}),
            r#"[{"city":"上海"}]"#,
            r#"[{"city":"北京"},{"city":"上海"}]"#,
        ),
    ];
    for (mut spec, good, bad) in cases {
        spec["key"] = json!("example");
        spec["label"] = json!("设置");
        spec["default"] = json!(good);
        let field: Field = serde_json::from_value(spec.clone())?;
        assert!(valid_fields(&[field.clone()], false), "{spec}");
        assert!(!valid_field(&field, bad), "{spec}:{bad}");
        spec["default"] = json!(bad);
        let field = serde_json::from_value(spec)?;
        assert!(!valid_fields(&[field], false));
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("spike/examples/payday/plugin.json");
    let old = Package::read(&path)?;
    let settings = old.settings_values(&Values::from([("title".into(), "工资".into())]));
    assert_eq!(settings["title"], "工资");
    assert_eq!(settings["target"], "2026-12-25");
    Ok(())
}

#[test]
fn canvas_settings_search_chinese_timezones_and_validate_list_choices() -> Result<()> {
    use crate::plugin_setting_catalog::timezone_cities;

    let cities: Vec<_> = timezone_cities("").collect();
    assert!(cities.len() >= 300);
    assert!(cities.iter().all(|city| city.zone.parse::<chrono_tz::Tz>().is_ok()));
    assert!(timezone_cities("纽约").any(|city| city.zone == "America/New_York"));
    assert!(timezone_cities("美国").any(|city| city.zone == "America/Chicago"));
    assert!(timezone_cities("europe/london").any(|city| city.city == "伦敦"));
    assert!(timezone_cities("不存在的地方").next().is_none());

    let spec = json!({
        "key":"rows", "label":"项目", "type":"list", "default":"[]", "maxItems":20,
        "item":[
            {"key":"zone","label":"时区","type":"timezone","default":"Asia/Shanghai"},
            {"key":"icon","label":"图标","type":"icon","default":"heart"},
            {"key":"color","label":"颜色","type":"color","default":"red"},
            {"key":"enabled","label":"启用","type":"switch","default":"true"}
        ]
    });
    let field: Field = serde_json::from_value(spec)?;
    assert!(valid_fields(std::slice::from_ref(&field), false));
    let good = json!({"zone":"America/New_York","icon":"cake","color":"purple","enabled":"false"});
    assert!(valid_field(&field, &json!([good.clone()]).to_string()));
    for (key, value) in [("zone", "纽约"), ("icon", "missing"), ("color", "#ffffff"), ("enabled", "on")] {
        let mut bad = good.clone();
        bad[key] = json!(value);
        assert!(!valid_field(&field, &json!([bad]).to_string()));
    }
    assert!(!valid_fields(field.item.as_deref().unwrap(), false));
    Ok(())
}

#[test]
fn time_input_resolves_daylight_saving_for_list_values() -> Result<()> {
    let field: Field = serde_json::from_value(
        json!({"key":"cities","label":"城市","type":"list","default":"[]"}),
    )?;
    let mut input = json!({});
    let values = Values::from([(
        "cities".into(),
        r#"[{"zone":"America/New_York"},{"zone":"Asia/Shanghai"}]"#.into(),
    )]);
    for (iso, offset, dst) in [
        ("2026-01-10T00:00:00Z", -300, false),
        ("2026-07-10T00:00:00Z", -240, true),
    ] {
        let now = iso.parse::<chrono::DateTime<chrono::Utc>>()?;
        script_settings::time_input(&mut input, now, true, std::slice::from_ref(&field), &values);
        assert_eq!(input["now"], json!(now.timestamp_millis()));
        assert!(input["tz"]["offset_minutes"].is_i64());
        assert!(!input["tz"]["name"].as_str().unwrap().is_empty());
        assert_eq!(
            input["zones"]["America/New_York"],
            json!({"offset_minutes":offset,"dst":dst})
        );
        assert_eq!(
            input["zones"]["Asia/Shanghai"]["offset_minutes"],
            json!(480)
        );
    }
    Ok(())
}

#[test]
fn schedule_selects_early_refresh_and_clamps_near_past_or_distant_dates() {
    for (requested, delay) in [(500, 60), (1_010, 60), (1_120, 120), (5_000, 600)] {
        assert_eq!(
            script_settings::next_delay(1_000, requested, 600).as_secs(),
            delay
        );
    }
}

#[test]
fn output_and_storage_have_independent_limits() -> Result<()> {
    let raw = json!({"card":"测试", "storage":"a".repeat(256 * 1024 - 2)}).to_string();
    let (output, storage) = script_settings::split_output(&raw)?;
    assert_eq!(output["card"], "测试");
    assert_eq!(storage.as_str().unwrap().len(), 256 * 1024 - 2);
    let raw = json!({"card":"测试", "storage":"a".repeat(256 * 1024)}).to_string();
    assert!(script_settings::split_output(&raw)
        .unwrap_err()
        .to_string()
        .contains("私有存储已满"));
    let raw = json!({"card":"x".repeat(16 * 1024)}).to_string();
    assert!(script_settings::split_output(&raw).is_err());
    Ok(())
}

#[test]
fn package_data_validates_names_limits_and_examples() -> Result<()> {
    for name in [
        "../a.json",
        "dir/a.json",
        r"dir\a.json",
        "a.txt",
        "plugin.json",
        "C:a.json",
        "NUL.json",
    ] {
        assert!(!package_data::valid_names(&[name.into()]));
    }
    assert!(!package_data::valid_names(&[
        "A.json".into(),
        "a.json".into()
    ]));
    let names = (0..5).map(|i| format!("data-{i}.json")).collect::<Vec<_>>();
    assert!(
        package_data::read_data(&names, |_| Ok(format!("\"{}\"", "x".repeat(64 * 1024 - 2))))
            .is_err()
    );
    assert!(package_data::read_data(&names[..1], |_| Ok(" ".repeat(64 * 1024 + 1))).is_err());
    assert!(package_data::read_data(&names[..1], |_| Ok("not json".into())).is_err());
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("spike/examples");
    let holidays = Package::read(&examples.join("holidays/plugin.json"))?;
    let result = holidays.render(&Values::new(), &Value::Null, "2026-10-10")?;
    assert_eq!(result.marker, "调休上班");
    assert!(result.next_refresh_at.is_some());
    let countdown = Package::read(&examples.join("countdown/plugin.json"))?;
    let mut settings = countdown.settings_values(&Values::new());
    let result = countdown.render(&settings, &Value::Null, "2026-12-31")?;
    assert_eq!(result.marker, "纪念日");
    assert_eq!(
        countdown
            .render(&settings, &Value::Null, "2027-01-02")?
            .card,
        "纪念日已到期"
    );
    settings.insert("mode".into(), "since".into());
    assert_eq!(
        countdown
            .render(&settings, &Value::Null, "2027-01-02")?
            .card,
        "纪念日已过 2 天"
    );
    Ok(())
}

#[test]
fn package_data_rejects_hard_links() -> Result<()> {
    let dir = std::env::temp_dir().join(format!(
        "daozai-data-links-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    fs::create_dir(&dir)?;
    let dir = dir.canonicalize()?;
    let file = dir.join("data.json");
    let link = dir.join("link.json");
    fs::write(&file, "[]")?;
    assert_eq!(package_data::read_file(&dir, "data.json")?, "[]");
    fs::hard_link(&file, &link)?;
    let result = package_data::read_file(&dir, "link.json");
    fs::remove_file(link)?;
    fs::remove_file(file)?;
    fs::remove_dir(dir)?;
    assert!(result.is_err());
    Ok(())
}

#[test]
fn records_restore_maximum_package_data_and_storage_after_json_escaping() -> Result<()> {
    const ID: &str = "example.payday";
    let dir = std::env::temp_dir().join(format!(
        "daozai-data-snapshot-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut host = LocalPlugins::open(dir.clone());
    host.install(&Path::new(env!("CARGO_MANIFEST_DIR")).join("spike/examples/payday/plugin.json"))?;
    let mut record = host.records[ID].clone();
    let package = record.package.as_mut().unwrap();
    package.source = format!(
        "/*{}*/{}",
        "\x01".repeat(SCRIPT_SOURCE_LIMIT as usize - package.source.len() - 4),
        package.source
    );
    package.manifest.data = (0..4).map(|i| format!("data-{i}.json")).collect();
    package.data = package_data::read_data(&package.manifest.data, |_| {
        Ok(format!(
            "{}0",
            "\n".repeat(package_data::DATA_FILE_LIMIT as usize - 1)
        ))
    })?;
    record.storage = Value::String("x".repeat(256 * 1024 - 2));
    assert!(record.valid(ID));
    let path = host.path(ID);
    host.put(ID, record)?;
    assert!(fs::metadata(&path)?.len() > 2 * 1024 * 1024);
    drop(host);

    let host = LocalPlugins::open(dir.clone());
    let restored = &host.records[ID];
    assert_eq!(restored.package.as_ref().unwrap().data.len(), 4);
    assert_eq!(restored.storage.as_str().unwrap().len(), 256 * 1024 - 2);
    drop(host);
    fs::remove_file(path)?;
    fs::remove_dir(dir)?;
    Ok(())
}
