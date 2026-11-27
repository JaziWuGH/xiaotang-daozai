use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::atomic::AtomicUsize,
    thread,
};

const TODAY: &str = "2026-11-22";
const READ: &str = "example.read";
const OTHER: &str = "example.other";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("daozai-read-{}-{unique}", std::process::id()));
        fs::create_dir(&root)?;
        Ok(Self(root))
    }

    fn host(&self) -> LocalPlugins {
        LocalPlugins::open(self.0.join("installed"))
    }

    fn plugin(&self, id: &str, permissions: Value, code: &str) -> Result<PathBuf> {
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
                "id": id, "name": "测试卡片", "author": "示例", "version": "1.0.0",
                "api": 1, "runtime": "js", "entry": "main.js", "owner": "tasks", "cover": COVER_FILE,
                "capabilities": ["card", "timer"], "dataVersion": 1, "permissions": permissions, "settings": []
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

fn wait(mut until: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !until() {
        assert!(Instant::now() < deadline, "等待后台插件完成超时");
        thread::sleep(Duration::from_millis(2));
    }
}

fn settle(host: &mut LocalPlugins, tasks_on: bool) {
    wait(|| {
        host.tick(tasks_on, false, TODAY);
        host.jobs.is_empty()
    });
}

const SOURCE_CARD: &str =
    "function refresh(i) { return { card: i.source.text ?? i.source.status }; }";

#[test]
fn user_file_grant_reads_only_after_enable_and_is_revoked_on_uninstall() -> Result<()> {
    let fixture = Fixture::new()?;
    let path = fixture.plugin(READ, json!({"source.read":"file"}), SOURCE_CARD)?;
    let file = fixture.0.join("数据.txt");
    fs::write(&file, "\u{feff}文件已读")?;
    let mut host = fixture.host();
    host.install(&path)?;
    host.enable(READ, true)?;
    settle(&mut host, true);
    assert_eq!(host.cards[READ], "unconfigured");

    assert!(host
        .set_source_grant(
            READ,
            Some(source_read::grant_url("http://127.0.0.1/status")?)
        )
        .is_err());
    host.enable(READ, false)?;
    host.set_source_grant(READ, Some(source_read::grant_file(&file)?))?;
    settle(&mut host, true);
    assert!(!host.cards.contains_key(READ));
    drop(host);

    let mut host = fixture.host();
    host.enable(READ, true)?;
    settle(&mut host, true);
    assert_eq!(host.cards[READ], "文件已读");
    fs::rename(&file, fixture.0.join("已移走.txt"))?;
    host.next.remove(READ);
    settle(&mut host, true);
    assert_eq!(
        host.cards.get(READ).map(String::as_str),
        Some("offline"),
        "{:?}",
        host.errors
    );
    host.uninstall(READ, true)?;
    host.install(&path)?;
    assert!(host.source_grant(READ).is_none());
    Ok(())
}

#[test]
fn disabling_an_in_flight_local_request_discards_it_and_preserves_other_cards() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut host = fixture.host();
    host.install(&fixture.plugin(READ, json!({"source.read":"loopback"}), SOURCE_CARD)?)?;
    host.install(&fixture.plugin(
        OTHER,
        json!({}),
        "function refresh() { return { card: '普通卡片' }; }",
    )?)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let url = format!("http://{}/status", listener.local_addr()?);
    let requests = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(AtomicBool::new(false));
    let worker_requests = Arc::clone(&requests);
    let worker_release = Arc::clone(&release);
    let server = thread::spawn(move || -> std::io::Result<TcpListener> {
        let mut connection = None;
        wait(|| match listener.accept() {
            Ok((stream, _)) => {
                connection = Some(stream);
                true
            }
            Err(_) => false,
        });
        let mut stream = connection.unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut request = String::new();
        let mut reader = BufReader::new(&mut stream);
        reader.read_line(&mut request)?;
        assert_eq!(request.trim_end(), "GET /status HTTP/1.1");
        loop {
            request.clear();
            if reader.read_line(&mut request)? == 0 || request == "\r\n" {
                break;
            }
        }
        worker_requests.fetch_add(1, Ordering::Release);
        wait(|| worker_release.load(Ordering::Acquire));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nlate")?;
        Ok(listener)
    });

    host.set_source_grant(READ, Some(source_read::grant_url(&url)?))?;
    host.enable(OTHER, true)?;
    settle(&mut host, true);
    assert_eq!(requests.load(Ordering::Acquire), 0);
    assert_eq!(host.cards[OTHER], "普通卡片");

    host.enable(READ, true)?;
    host.next.remove(OTHER);
    wait(|| {
        host.tick(true, false, TODAY);
        requests.load(Ordering::Acquire) == 1 && !host.jobs.contains_key(OTHER)
    });
    assert!(host.jobs.contains_key(READ));
    assert_eq!(host.cards[OTHER], "普通卡片");
    host.enable(READ, false)?;
    release.store(true, Ordering::Release);
    let listener = server.join().unwrap()?;
    settle(&mut host, true);
    assert!(!host.cards.contains_key(READ));
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    drop(listener);

    host.enable(READ, true)?;
    settle(&mut host, true);
    assert_eq!(host.cards[READ], "offline");
    assert_eq!(host.cards[OTHER], "普通卡片");
    host.set_source_grant(READ, None)?;
    settle(&mut host, true);
    assert_eq!(host.cards[READ], "unconfigured");
    Ok(())
}

#[test]
fn https_manifest_combination_and_public_sample() -> Result<()> {
    let manifest = include_str!("examples/https-repository/plugin.json");
    let package = Package::from_entry(
        Manifest::decode(manifest)?,
        include_str!("examples/https-repository/main.js").into(),
        CoverPng::decode(include_bytes!("examples/https-repository/cover.png").to_vec())?,
    )?;
    let input = SourceInput::Ready { text: r#"{"stargazers_count":123}"#.into() };
    assert_eq!(
        package.render_input(&Values::new(), &Value::Null, TODAY, input, Call::Refresh)?.card,
        "Rust · 123 星标"
    );
    let original: Value = serde_json::from_str(manifest)?;
    let mut wrong = original.clone();
    wrong.as_object_mut().unwrap().remove("network");
    assert!(Manifest::decode(&wrong.to_string()).is_err());
    wrong = original.clone();
    wrong["permissions"]["source.read"] = json!("loopback");
    assert!(Manifest::decode(&wrong.to_string()).is_err());
    wrong = original.clone();
    wrong["network"]["hosts"] = json!(["example.com", "EXAMPLE.COM"]);
    assert!(Manifest::decode(&wrong.to_string()).is_err());
    wrong = original;
    wrong["network"]["url"] = json!("https://elsewhere.example.com/status");
    assert!(Manifest::decode(&wrong.to_string()).is_err());
    Ok(())
}

#[test]
fn gold_example_charts_fresh_prices_and_notifies_once_per_crossing() -> Result<()> {
    let package = Package::from_entry(
        Manifest::decode(include_str!("examples/gold-price/plugin.json"))?,
        include_str!("examples/gold-price/main.js").into(),
        CoverPng::decode(include_bytes!("examples/gold-price/cover.png").to_vec())?,
    )?;
    let now = chrono::Utc::now();
    let settings = Values::from([("threshold".into(), "750".into())]);
    let at = (now - chrono::TimeDelta::seconds(30)).to_rfc3339();
    let mut quote = json!({
        "xau": {"price": 751.5, "currency": "CNY", "unit": "gram"},
        "updated_at": at, "data_state": {"status": "fresh", "as_of": at}, "fx_stale": false
    });
    let run = |quote: &Value, storage: &Value| {
        let source = SourceInput::Multiple(BTreeMap::from([(
            "gold".into(), SourceInput::Ready { text: quote.to_string() },
        )]));
        package.render_input(&settings, storage, TODAY, source, Call::Refresh)
    };
    let storage = json!({"points": (31..=78).rev().map(|age| {
        json!({"price": 749.0, "at": now.timestamp_millis() - age * 1000})
    }).collect::<Vec<_>>()});
    let rising = run(&quote, &storage)?;
    let status = rising.status.as_ref().unwrap();
    assert_eq!(status.metrics[0].value, "751.50 元");
    let series = status.series.as_ref().unwrap();
    assert_eq!(series.len(), 48);
    assert_eq!(series[47], 751.5);
    assert_eq!(rising.storage["points"].as_array().unwrap().len(), 48);
    assert!(rising.notice.is_some());
    assert!(rising.pinned.is_some());
    assert_eq!(status.alert.as_deref(), Some("金价达到 750 元/克"));
    let repeated = run(&quote, &rising.storage)?;
    assert_eq!(repeated.storage, rising.storage);
    assert!(repeated.notice.is_none());
    assert!(repeated.status.as_ref().unwrap().alert.is_none());
    assert!(repeated.pinned.is_some());

    quote["price_as_of"] = json!((now - chrono::TimeDelta::seconds(20)).to_rfc3339());
    quote["xau"]["price"] = json!(749);
    let falling = run(&quote, &rising.storage)?;
    assert!(falling.notice.is_some());
    assert!(falling.pinned.is_some());
    assert_eq!(falling.status.as_ref().unwrap().alert.as_deref(), Some("金价跌至 750 元/克"));
    quote["data_state"]["status"] = json!("stale");
    let stale = run(&quote, &falling.storage)?;
    assert_eq!(stale.storage, falling.storage);
    assert!(stale.status.is_none());
    assert!(stale.notice.is_none());
    assert!(stale.pinned.is_none());

    let source = SourceInput::Multiple(BTreeMap::from([("gold".into(), SourceInput::Offline)]));
    let offline = package.render_input(&settings, &falling.storage, TODAY, source, Call::Refresh)?;
    assert_eq!(offline.storage, falling.storage);
    assert!(offline.status.is_none());
    assert!(offline.notice.is_none());
    assert!(offline.pinned.is_none());
    Ok(())
}

#[test]
fn adjustable_https_refresh_is_validated_persisted_and_kept_out_of_script_settings() -> Result<()> {
    let root = Fixture::new()?;
    let path = root.0.join("plugin.json");
    let mut manifest: Value = serde_json::from_str(include_str!("examples/gold-price/plugin.json"))?;
    manifest["capabilities"] = json!(["status.page", "settings", "timer", "storage"]);
    manifest.as_object_mut().unwrap().remove("statusLayout");
    fs::write(&path, serde_json::to_vec(&manifest)?)?;
    fs::write(root.0.join("main.js"), "function refresh() { return {status: null}; }")?;
    fs::write(root.0.join(COVER_FILE), include_bytes!("examples/gold-price/cover.png"))?;
    let mut host = root.host();
    host.install(&path)?;
    let id = "example.gold-price";
    assert_eq!(host.records[id].refresh_interval(), Duration::from_secs(300));
    for (seconds, valid) in [(59, false), (60, true), (86400, true), (86401, false)] {
        let mut values = host.records[id].values.clone();
        values.insert(source_https::REFRESH_FIELD.into(), seconds.to_string());
        assert_eq!(host.save(id, values).is_ok(), valid);
    }
    let host = root.host();
    assert_eq!(host.records[id].refresh_interval(), Duration::from_secs(86400));
    assert!(!host.records[id].values.contains_key(source_https::REFRESH_FIELD));
    for invalid in [59, 86401] {
        let mut manifest = manifest.clone();
        manifest["network"]["minRefreshSeconds"] = json!(invalid);
        assert!(Manifest::decode(&manifest.to_string()).is_err());
    }
    Ok(())
}

#[test]
fn https_credential_is_external_and_revoke_stops_reads() -> Result<()> {
    let fixture = Fixture::new()?;
    let id = format!("example.https-{}", fixture.0.file_name().unwrap().to_string_lossy());
    let path = fixture.plugin(&id, json!({"source.read":"https"}), SOURCE_CARD)?;
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path)?)?;
    manifest["network"] = json!({
        "hosts": ["example.com"], "url": "https://example.com/data",
        "credential": {"label":"测试密钥", "header":"X-Test-Key", "prefix":"Bearer "}
    });
    fs::write(&path, serde_json::to_vec(&manifest)?)?;
    struct Cleanup<'a>(&'a str);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = source_https::delete_secret(self.0);
        }
    }
    let _cleanup = Cleanup(&id);
    let mut host = fixture.host();
    host.install(&path)?;
    let grant = host.records[&id].network().unwrap().grant("https://example.com/data")?;
    let secret = "nonproduction-credential-for-local-test";
    assert!(host.set_source_grant(&id, Some(grant.clone())).is_err());
    host.set_source_grant_with_secret(&id, Some(grant.clone()), Some(secret))?;
    assert_eq!(host.source_grant(&id), Some(&grant));
    assert_eq!(source_https::load_secret(&id)?.as_deref(), Some(secret));
    assert!(!fs::read_to_string(host.path(&id))?.contains(secret));
    drop(host);

    let mut host = fixture.host();
    assert_eq!(host.source_grant(&id), Some(&grant));
    host.set_source_grant(&id, None)?;
    assert!(source_https::load_secret(&id)?.is_none());
    host.enable(&id, true)?;
    settle(&mut host, true);
    assert_eq!(host.cards[&id], "unconfigured");

    host.set_source_grant_with_secret(&id, Some(grant), Some(secret))?;
    host.uninstall(&id, true)?;
    assert!(source_https::load_secret(&id)?.is_none());
    host.install(&path)?;
    assert!(host.source_grant(&id).is_none());
    Ok(())
}
