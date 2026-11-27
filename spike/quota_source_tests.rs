use super::*;
use std::thread;

const TODAY: &str = "2026-11-22";
const FILE_SOURCE: &str = "example.file-quota";
const OTHER_SOURCE: &str = "example.other-quota";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("daozai-quota-{}-{unique}", std::process::id()));
        fs::create_dir(&root)?;
        Ok(Self(root))
    }

    fn plugin(
        &self,
        id: &str,
        owner: &str,
        capabilities: &[&str],
        permissions: Value,
        code: &str,
    ) -> Result<PathBuf> {
        let dir = self.0.join(id);
        fs::create_dir(&dir)?;
        fs::write(
            dir.join(COVER_FILE),
            include_bytes!("examples/payday/cover.png"),
        )?;
        fs::write(dir.join("main.js"), code)?;
        let path = dir.join("plugin.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "id": id, "name": "模拟额度", "author": "示例", "version": "1.0.0",
                "api": 1, "runtime": "js", "entry": "main.js", "owner": owner, "cover": COVER_FILE,
                "capabilities": capabilities, "dataVersion": 1, "permissions": permissions, "settings": []
            }))?,
        )?;
        Ok(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn settle(host: &mut LocalPlugins, quota_on: bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        host.tick_features(false, false, quota_on, TODAY);
        if host.jobs.is_empty() {
            return;
        }
        assert!(Instant::now() < deadline, "等待额度来源刷新超时");
        thread::sleep(Duration::from_millis(2));
    }
}

fn allowance<'a>(host: &'a LocalPlugins, id: &str) -> Option<(Option<f64>, Option<&'a str>)> {
    let (_, _, quota) = host.quota_sources().find(|(source, _, _)| *source == id)?;
    let window = quota.windows.first()?;
    Some((window.remaining, window.unit.as_deref()))
}

#[test]
fn quota_sources_keep_independent_results_and_follow_the_module_switch() -> Result<()> {
    let fixture = Fixture::new()?;
    let path = fixture.plugin(
        FILE_SOURCE,
        "quota",
        &["quota.source", "timer"],
        json!({"source.read":"file"}),
        r#"function refresh(i) {
            if (i.source.status === 'ready') return JSON.parse(i.source.text);
            return {quota:{windows:[{label:'后备数据',remaining:1,unit:'点'}]}};
        }"#,
    )?;
    let other = fixture.plugin(
        OTHER_SOURCE, "quota", &["quota.source", "timer"], json!({}),
        r#"function refresh() { return {quota:{windows:[{label:'另一来源',remaining:54,unit:'次'}]}}; }"#,
    )?;
    let file = fixture.0.join("quota.json");
    fs::write(
        &file,
        r#"{"quota":{"windows":[{"label":"模拟积分","remaining":80,"limit":100,"unit":"点"}]}}"#,
    )?;
    let mut host = LocalPlugins::open(fixture.0.join("installed"));
    host.install(&path)?;
    host.install(&other)?;
    host.enable(FILE_SOURCE, true)?;
    host.enable(OTHER_SOURCE, true)?;
    settle(&mut host, false);
    assert_eq!(host.quota_sources().count(), 0);

    settle(&mut host, true);
    assert_eq!(allowance(&host, FILE_SOURCE), None);
    assert_eq!(
        allowance(&host, OTHER_SOURCE),
        Some((Some(54.0), Some("次")))
    );

    host.set_source_grant(FILE_SOURCE, Some(source_read::grant_file(&file)?))?;
    settle(&mut host, true);
    assert_eq!(
        allowance(&host, FILE_SOURCE),
        Some((Some(80.0), Some("点"))),
        "{:?}",
        host.errors
    );
    assert_eq!(
        allowance(&host, OTHER_SOURCE),
        Some((Some(54.0), Some("次")))
    );

    let disconnected = fixture.0.join("disconnected.json");
    fs::rename(&file, &disconnected)?;
    host.next.remove(FILE_SOURCE);
    settle(&mut host, true);
    assert_eq!(allowance(&host, FILE_SOURCE), None);
    assert_eq!(
        allowance(&host, OTHER_SOURCE),
        Some((Some(54.0), Some("次")))
    );

    fs::rename(disconnected, file)?;
    host.next.remove(FILE_SOURCE);
    settle(&mut host, true);
    assert_eq!(
        allowance(&host, FILE_SOURCE),
        Some((Some(80.0), Some("点")))
    );

    host.next.remove(FILE_SOURCE);
    host.tick_features(false, false, true, TODAY);
    assert!(host.jobs.contains_key(FILE_SOURCE));
    host.enable(FILE_SOURCE, false)?;
    settle(&mut host, true);
    assert_eq!(allowance(&host, FILE_SOURCE), None);
    assert_eq!(
        allowance(&host, OTHER_SOURCE),
        Some((Some(54.0), Some("次")))
    );

    settle(&mut host, false);
    assert_eq!(host.quota_sources().count(), 0);
    assert!(host.jobs.is_empty());
    settle(&mut host, true);
    assert_eq!(allowance(&host, FILE_SOURCE), None);
    assert_eq!(
        allowance(&host, OTHER_SOURCE),
        Some((Some(54.0), Some("次")))
    );
    Ok(())
}

#[test]
fn quota_sources_reject_invalid_payloads_and_undeclared_outputs() -> Result<()> {
    let fixture = Fixture::new()?;
    let denied = fixture.plugin(
        "example.not-quota",
        "quota",
        &["card"],
        json!({}),
        "function refresh(){return {card:'内容'};}",
    )?;
    assert!(Package::read(&denied).is_err());

    let values = Values::new();
    let foreign = fixture.plugin(
        "example.foreign-quota", "tasks", &["card"], json!({}),
        r#"function refresh(){return {card:'卡片',quota:{windows:[{label:'嵌入额度',remaining_pct:50}]}};}"#,
    )?;
    assert!(Package::read(&foreign)?
        .render(&values, &Value::Null, TODAY)
        .is_err());

    for (id, payload) in [
        ("example.empty-quota", json!({"quota":{"windows":[]}})),
        (
            "example.percentage-quota",
            json!({"quota":{"windows":[{"label":"比例","remaining_pct":101}]}}),
        ),
        (
            "example.negative-quota",
            json!({"quota":{"windows":[{"label":"积分","remaining":-1,"limit":100,"unit":"点"}]}}),
        ),
    ] {
        let path = fixture.plugin(
            id,
            "quota",
            &["quota.source"],
            json!({}),
            &format!("function refresh(){{return {payload};}}"),
        )?;
        assert!(
            Package::read(&path)?
                .render(&values, &Value::Null, TODAY)
                .is_err(),
            "{id}"
        );
    }
    Ok(())
}
