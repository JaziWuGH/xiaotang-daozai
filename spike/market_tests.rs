use super::*;

#[test]
fn market_package_preserves_declared_data() -> Result<()> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("spike/examples/holidays/plugin.json");
    let zip = std::env::temp_dir().join(format!("daozai-package-data-{}.zip", std::process::id()));
    let result = (|| -> Result<()> {
        pack(&manifest, &zip)?;
        let package = decode_zip(&fs::read(&zip)?, true)?;
        assert!(package.data.contains_key("holidays-2026.json"));
        assert_eq!(package.render(&Values::new(), &Value::Null, "2026-06-19")?.marker, "端午节放假");
        Ok(())
    })();
    let _ = fs::remove_file(zip);
    result
}
use super::super::{LocalPlugins, WIRELESS_PLUGIN_ID};
use serde_json::json;

#[test]
fn navigation_icon_rejects_invalid_manifest_values() -> Result<()> {
    let old = include_str!("examples/world-clock/plugin.json");
    let mut manifest: Value = serde_json::from_str(old)?;
    assert!(Manifest::decode(old)?.navigation_icon.is_none());
    let valid = json!({"path": "M0 0 H24 V24 Z M8 8 Q12 4 16 8", "background": "#C45848"});
    manifest["navigationIcon"] = valid.clone();
    assert!(Manifest::decode(&manifest.to_string()).is_ok());

    for invalid in [
        Value::Null,
        json!({"path": "M0 0 H24"}),
        json!({"path": "M0 0 H24", "background": "#fff"}),
        json!({"path": "M0 0 H24", "background": "#C45848", "extra": true}),
        json!({"path": "M0 0 Z", "background": "#C45848"}),
        json!({"path": "M0 0 H25", "background": "#C45848"}),
        json!({"path": "M0 0 h-1", "background": "#C45848"}),
        json!({"path": "M0 0 Q25 3 8 8", "background": "#C45848"}),
        json!({"path": "M0 0 C1 1 2 -1 8 8", "background": "#C45848"}),
        json!({"path": "M0 0 A1 1 0 0 0 8 8", "background": "#C45848"}),
        json!({"path": "M0 0 H24　", "background": "#C45848"}),
        json!({"path": format!("M0 0{}", " L1 1".repeat(64)), "background": "#C45848"}),
        json!({"path": format!("M0 0 H24{}", " ".repeat(1024)), "background": "#C45848"}),
    ] {
        manifest["navigationIcon"] = invalid;
        assert!(Manifest::decode(&manifest.to_string()).is_err(), "{manifest}");
    }
    manifest["navigationIcon"] = valid;
    manifest["owner"] = json!("tasks");
    manifest["capabilities"].as_array_mut().unwrap().push(json!("card"));
    assert!(Manifest::decode(&manifest.to_string()).is_err());
    Ok(())
}

#[test]
fn navigation_icon_survives_local_install_market_zip_and_restart() -> Result<()> {
    const ID: &str = "example.world-clock";
    let fixture = Fixture::new()?;
    let manifest = fixture.0.join("plugin.json");
    let mut body: Value = serde_json::from_str(include_str!("examples/world-clock/plugin.json"))?;
    let declaration = json!({"path": "M3 3 H21 V21 H3 Z", "background": "#C45848"});
    body["navigationIcon"] = declaration.clone();
    fs::write(&manifest, serde_json::to_vec(&body)?)?;
    fs::write(fixture.0.join("main.js"), include_str!("examples/world-clock/main.js"))?;
    fs::write(fixture.0.join(COVER_FILE), include_bytes!("examples/world-clock/cover.png"))?;

    let local = fixture.0.join("local-install");
    let mut host = LocalPlugins::open(local.clone());
    host.install(&manifest)?;
    let icon = host.navigation_icon(ID).expect("开发包保留图标");
    assert_eq!(icon.background(), [196, 88, 72]);
    let commands = icon.commands().to_vec();
    assert_eq!(commands.len(), 5);
    let reopened = LocalPlugins::open(local);
    assert_eq!(reopened.navigation_icon(ID).unwrap().commands(), commands);

    let archive = fixture.0.join("bundle.zip");
    let index = fixture.0.join("catalog.json");
    pack(&manifest, &archive)?;
    submit(&archive, &index)?;

    let storage = fixture.0.join("market-install");
    let mut host = LocalPlugins::open(storage.clone());
    host.set_market_catalog(&index)?;
    host.market_action(false, ID)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while host.market_busy() {
        host.poll_market();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(host.navigation_icon(ID).unwrap().commands(), commands);
    let installed = &host.records[ID].package.as_ref().unwrap().manifest;
    assert_eq!(serde_json::to_value(installed)?["navigationIcon"], declaration);
    let reopened = LocalPlugins::open(storage);
    assert_eq!(reopened.navigation_icon(ID).unwrap().background(), [196, 88, 72]);
    assert_eq!(reopened.navigation_icon(ID).unwrap().commands(), commands);
    Ok(())
}

#[test]
fn gitee_contents_accepts_only_the_requested_bounded_file() -> Result<()> {
    let path = "stable/catalog.json";
    let valid = json!({
        "type": "file",
        "encoding": "base64",
        "size": 3,
        "path": path,
        "content": "YW\r\nJj"
    });
    assert_eq!(
        decode_gitee_file(&serde_json::to_vec(&valid)?, path, 3)?,
        b"abc"
    );
    for (key, value) in [
        ("type", json!("symlink")),
        ("encoding", json!("utf8")),
        ("path", json!("preview/catalog.json")),
        ("size", json!(4)),
        ("size", json!(2)),
        ("content", json!("******")),
        ("content", json!("****")),
    ] {
        let mut invalid = valid.clone();
        invalid[key] = value;
        assert!(decode_gitee_file(&serde_json::to_vec(&invalid)?, path, 3).is_err());
    }
    Ok(())
}

#[test]
fn raw_fallback_requires_rate_limiting_and_the_same_gitee_file() -> Result<()> {
    assert!(gitee_rate_limited(429, std::io::empty()));
    assert!(gitee_rate_limited(
        403,
        &b"Forbidden (Rate Limit Exceeded)"[..]
    ));
    assert!(!gitee_rate_limited(403, &b"Forbidden"[..]));
    assert!(!gitee_rate_limited(404, &b"Rate Limit Exceeded"[..]));
    assert!(!gitee_rate_limited(403, &vec![b'x'; 4096][..]));

    let base = "https://gitee.com/api/v5/repos/xiaotangtoken/xiaotang-daozai-plugins/contents/";
    let source = Url::parse(&format!("{base}preview/catalog.json?ref=master"))?;
    let raw = gitee_raw_url(&source).unwrap();
    let raw_path = "/xiaotangtoken/xiaotang-daozai-plugins/raw/master/preview/catalog.json";
    assert_eq!(raw.as_str(), format!("https://gitee.com{raw_path}"));
    assert!(gitee_raw_url(&Url::parse(&format!(
        "{base}stable/avatar-author.png?ref=master"
    ))?)
    .is_some());
    assert!(gitee_raw_url(&Url::parse(&format!(
        "{base}stable/plugin-example.payday-1.0.0.zip?ref=master"
    ))?)
    .is_some());
    for invalid in [
        format!("{base}preview/catalog.json?ref=other"),
        format!("{base}preview/catalog.json?ref=master&ref=other"),
        format!("{base}unapproved/catalog.json?ref=master"),
        format!("{base}preview/nested/catalog.json?ref=master"),
        format!("{base}preview/index.json?ref=master"),
        format!("{base}preview/catalog.json?ref=master#fragment"),
        source
            .as_str()
            .replace("gitee.com/", "gitee.com.evil.invalid/"),
        source.as_str().replace("xiaotangtoken/", "another-owner/"),
    ] {
        assert!(gitee_raw_url(&Url::parse(&invalid)?).is_none(), "{invalid}");
    }
    let redirect = format!("https://raw.giteeusercontent.com{raw_path}?signature=abc");
    assert_eq!(
        gitee_signed_location(&raw, &redirect),
        Some(Url::parse(&redirect)?)
    );
    for invalid in [
        redirect.replace("https:", "http:"),
        redirect.replace(
            "raw.giteeusercontent.com",
            "raw.giteeusercontent.com.evil.invalid",
        ),
        redirect.replace("raw.giteeusercontent.com", "user@raw.giteeusercontent.com"),
        redirect.replace("preview", "stable"),
        redirect.replace("signature=abc", "token=abc"),
        redirect.replace("signature=abc", "signature="),
        redirect.replace("catalog.json", "avatar-author.png"),
    ] {
        assert!(gitee_signed_location(&raw, &invalid).is_none(), "{invalid}");
    }
    Ok(())
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "daozai-market-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn project(&self, version: &str) -> Result<PathBuf> {
        let path = self.0.join("plugin.json");
        let mut manifest: Value =
            serde_json::from_str(include_str!("examples/payday/plugin.json"))?;
        manifest["version"] = json!(version);
        fs::write(&path, serde_json::to_vec(&manifest)?)?;
        fs::write(
            self.0.join("main.js"),
            include_str!("examples/payday/main.js"),
        )?;
        fs::write(
            self.0.join(COVER_FILE),
            include_bytes!("examples/payday/cover.png"),
        )?;
        Ok(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn market_author_template_is_valid_and_preserves_existing_files() -> Result<()> {
    let fixture = Fixture::new()?;
    assert!(create_template(&fixture.0, "../escape", "发薪倒计时", "小明").is_err());
    assert!(create_template(&fixture.0, "xiaoming.payday", " ", "小明").is_err());
    assert!(create_template(&fixture.0, "xiaoming.payday", "发薪倒计时", " ").is_err());
    assert_eq!(fs::read_dir(&fixture.0)?.count(), 0);

    let path = create_template(&fixture.0, "xiaoming.payday", "自定义倒计时", "小明")?;
    assert_eq!(path, fixture.0.join("xiaoming.payday/plugin.json"));
    assert_eq!(validate_project(&path)?, "xiaoming.payday v1.0.0 校验通过");
    let original = fs::read_to_string(&path)?;
    let manifest = Manifest::decode(&original)?;
    assert_eq!(manifest.name, "自定义倒计时");
    assert_eq!(manifest.author, "小明");
    assert_eq!(
        fs::read(path.with_file_name("LICENSE"))?.as_slice(),
        include_bytes!("examples/payday/LICENSE")
    );

    assert!(create_template(&fixture.0, "xiaoming.payday", "另一个插件", "另一位作者").is_err());
    assert_eq!(fs::read_to_string(&path)?, original);
    assert_eq!(fs::read_dir(&fixture.0)?.count(), 1);
    fs::remove_file(path.with_file_name(COVER_FILE))?;
    assert!(validate_project(&path).is_err());
    Ok(())
}

#[test]
fn market_author_packages_and_submits_immutable_versions() -> Result<()> {
    let fixture = Fixture::new()?;
    let catalog_path = fixture.0.join("catalog.json");
    let first = fixture.0.join("first.zip");
    pack(&fixture.project("1.2.9")?, &first)?;
    assert!(pack(&fixture.project("1.2.9")?, &first).is_err());
    let submitted_on = chrono::Local::now().date_naive();
    submit(&first, &catalog_path)?;

    let catalog = Catalog::open(&catalog_path)?;
    let original = &catalog.entries[0];
    let first_presentation = original.presentation.as_ref().expect("提交应记录更新日期");
    let first_date = first_presentation.updated_on.as_deref().expect("更新日期缺失");
    let date = chrono::NaiveDate::parse_from_str(first_date, "%Y-%m-%d")?;
    assert!((submitted_on..=chrono::Local::now().date_naive()).contains(&date));
    assert_eq!(
        serde_json::to_value(first_presentation)?,
        json!({"updated_on": first_date})
    );
    assert_eq!(catalog.load(original)?.manifest.version, "1.2.9");
    assert_eq!(
        catalog.cover_png(original)?.as_slice(),
        include_bytes!("examples/payday/cover.png")
    );
    let original_path = original.path_in(&catalog.path);
    let original_bytes = fs::read(&original_path)?;
    let original_catalog = fs::read(&catalog_path)?;
    assert!(submit(&first, &catalog_path).is_err());
    assert_eq!(fs::read(&catalog_path)?, original_catalog);

    let mut index: Value = serde_json::from_slice(&original_catalog)?;
    let mut presentation = json!({
        "description": "发薪日提醒",
        "updated_on": "2026-11-21",
        "official": true,
        "sample": true,
        "maintainer": "Renly1994",
        "author_avatar": "avatar-renly1994.png",
        "downloads": 22,
        "favorites": 0
    });
    index["plugins"][0]["presentation"] = presentation.clone();
    fs::write(&catalog_path, serde_json::to_vec(&index)?)?;

    let second = fixture.0.join("second.zip");
    pack(&fixture.project("1.2.10")?, &second)?;
    fs::copy(&second, fixture.0.join("plugin-example.payday-1.2.10.zip"))?;
    let submitted_on = chrono::Local::now().date_naive();
    submit(&second, &catalog_path)?;
    let catalog = Catalog::open(&catalog_path)?;
    assert_eq!(catalog.entries.len(), 1);
    assert_eq!(
        catalog.load(&catalog.entries[0])?.manifest.version,
        "1.2.10"
    );
    assert_eq!(fs::read(&original_path)?, original_bytes);
    assert!(submit(&first, &catalog_path).is_err());

    let entry = &catalog.entries[0];
    let updated_date = entry
        .presentation
        .as_ref()
        .and_then(|details| details.updated_on.as_deref())
        .expect("新版本更新日期缺失");
    let date = chrono::NaiveDate::parse_from_str(updated_date, "%Y-%m-%d")?;
    assert!((submitted_on..=chrono::Local::now().date_naive()).contains(&date));
    presentation["updated_on"] = json!(updated_date);
    assert_eq!(serde_json::to_value(&entry.presentation)?, presentation);
    let mut raw = fs::read(entry.path_in(&catalog.path))?;
    raw[0] ^= 1;
    fs::write(entry.path_in(&catalog.path), raw)?;
    assert!(catalog
        .load(entry)
        .err()
        .expect("损坏的版本包应被拒绝")
        .to_string()
        .contains("SHA-256"));
    assert!(catalog.cover_png(entry).is_err());
    Ok(())
}

#[test]
fn local_market_author_avatar_preserves_the_plugin_package() -> Result<()> {
    let fixture = Fixture::new()?;
    let catalog_path = fixture.0.join("catalog.json");
    let bundle = fixture.0.join("first.zip");
    pack(&fixture.project("1.0.0")?, &bundle)?;
    submit(&bundle, &catalog_path)?;
    let mut index: Value = serde_json::from_slice(&fs::read(&catalog_path)?)?;
    index["plugins"][0]["presentation"] = json!({"description": "发薪日提醒"});
    fs::write(&catalog_path, serde_json::to_vec(&index)?)?;

    let avatar = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        16, 16, image::Rgba([46, 101, 170, 255]),
    ));
    let mut png = Cursor::new(Vec::new());
    avatar.write_to(&mut png, image::ImageFormat::Png)?;
    let bytes = png.into_inner();
    set_author_avatar(&catalog_path, "example.payday", &bytes)?;

    let catalog = Catalog::open(&catalog_path)?;
    let entry = &catalog.entries[0];
    assert_eq!(entry.sha256, sha256(&fs::read(&bundle)?));
    assert_eq!(catalog.load(entry)?.manifest.version, "1.0.0");
    assert_eq!(entry.presentation.as_ref().unwrap().description.as_deref(), Some("发薪日提醒"));
    let name = entry.avatar_name().expect("目录写入作者头像");
    assert_eq!(catalog.avatar_png(name).unwrap().bytes(), bytes.as_slice());
    let saved = fs::read(&catalog_path)?;
    assert!(set_author_avatar(&catalog_path, "example.missing", &bytes).is_err());
    assert!(set_author_avatar(&catalog_path, "example.payday", b"invalid png").is_err());
    assert_eq!(fs::read(&catalog_path)?, saved);
    fs::remove_file(fixture.0.join(name))?;
    assert!(catalog.avatar_png(name).is_none());
    assert!(catalog.avatar_png("../avatar-test.png").is_none());
    Ok(())
}

#[test]
fn avatar_profile_must_match_the_requested_github_account() -> Result<()> {
    let profile = |login, url| {
        serde_json::to_vec(&json!({"login": login, "id": 42, "avatar_url": url})).unwrap()
    };
    let avatar = "https://avatars.githubusercontent.com/u/42?v=4";
    let url = github_avatar_url(&profile("cayww", avatar), "CayWW")?;
    assert_eq!(url.host_str(), Some("avatars.githubusercontent.com"));
    assert!(url.query_pairs().any(|(name, _)| name == "s"));
    assert!(github_avatar_url(&profile("other", avatar), "cayww").is_err());
    assert!(github_avatar_url(&profile("cayww", "http://avatars.githubusercontent.com/u/42"), "cayww").is_err());
    assert!(github_avatar_url(&profile("cayww", "https://avatars.githubusercontent.com.example.com/u/42"), "cayww").is_err());
    Ok(())
}

fn publish_wireless(root: &Path, version: &str, data_version: &str) -> Result<()> {
    let manifest_path = root.join("plugin.json");
    fs::write(
        root.join(COVER_FILE),
        include_bytes!("examples/payday/cover.png"),
    )?;
    let manifest = json!({
        "id": WIRELESS_PLUGIN_ID,
        "name": "无线设备兼容包",
        "author": "岛仔",
        "version": version,
        "api": 1,
        "runtime": "assets",
        "entry": "wireless-receivers.json",
        "cover": COVER_FILE,
        "owner": "wireless",
        "capabilities": ["wireless.receivers"],
        "dataVersion": 1,
        "permissions": {},
        "settings": []
    });
    let mut receivers = vec![json!({"vid": "046D", "pid": "C542", "name": "罗技 Nano 接收器"})];
    if version != "1.0.0" {
        receivers.push(json!({"vid": "373B", "pid": "10C9", "name": "VXE 蜻蜓 R1 无线接收器"}));
    }
    let data = json!({
        "id": WIRELESS_PLUGIN_ID,
        "api_version": 1,
        "version": data_version,
        "receivers": receivers
    });
    fs::write(&manifest_path, serde_json::to_vec(&manifest)?)?;
    fs::write(root.join("wireless-receivers.json"), serde_json::to_vec(&data)?)?;
    let bundle = root.join(format!("wireless-{version}.zip"));
    pack(&manifest_path, &bundle)?;
    submit(&bundle, &root.join("catalog.json"))
}

fn apply_wireless_market_action(host: &mut LocalPlugins) -> Result<()> {
    host.market_action(false, WIRELESS_PLUGIN_ID)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while host.market_busy() {
        host.poll_market();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

impl LocalPlugins {
    pub(crate) fn test_market_install_wireless_compat(&mut self) -> Result<()> {
        let root = self.dir.join("test-wireless-market");
        fs::create_dir_all(&root)?;
        publish_wireless(&root, "1.0.0", "1.0.0")?;
        self.set_market_catalog(&root.join("catalog.json"))?;
        apply_wireless_market_action(self)
    }

    pub(crate) fn test_uninstall_wireless_compat(&mut self) -> Result<()> {
        self.uninstall(WIRELESS_PLUGIN_ID, true)
    }

    pub(crate) fn test_market_update_wireless_compat(&mut self) -> Result<()> {
        let root = self.dir.join("test-wireless-market");
        publish_wireless(&root, "1.0.1", "1.0.1")?;
        self.refresh_market_catalog()?;
        apply_wireless_market_action(self)
    }
}

#[test]
fn market_installs_wireless_compatibility_as_a_resource_package() -> Result<()> {
    use super::super::market_state::MarketStatus;
    let fixture = Fixture::new()?;
    let root = &fixture.0;
    let id = WIRELESS_PLUGIN_ID;
    let catalog_path = root.join("catalog.json");

    assert!(publish_wireless(root, "1.1.0", "1.0.0").is_err());
    publish_wireless(root, "1.0.0", "1.0.0")?;
    let catalog = Catalog::open(&catalog_path)?;
    let entry = &catalog.entries[0];
    assert_eq!(entry.owner(), Some("wireless"));
    assert!(catalog.load(entry)?.manifest.is_wireless());

    let storage = fixture.0.join("installed");
    let mut host = LocalPlugins::open(storage.clone());
    let wireless = host.wireless_store();
    assert!(wireless.package().is_none());
    assert!(host.install(&root.join("plugin.json")).is_err());
    host.set_market_catalog(&catalog_path)?;
    apply_wireless_market_action(&mut host)?;
    assert_eq!(host.market_status(&catalog, entry), MarketStatus::Installed);
    assert!(!host.records[id].enabled);
    assert!(wireless.package().is_none());
    host.enable(id, true)?;
    assert_eq!(wireless.package().unwrap().version(), "1.0.0");
    host.enable(id, false)?;
    assert!(wireless.package().is_none());
    host.enable(id, true)?;

    publish_wireless(root, "1.1.0", "1.1.0")?;
    host.refresh_market_catalog()?;
    apply_wireless_market_action(&mut host)?;
    assert_eq!(wireless.package().unwrap().version(), "1.1.0");
    let mut reopened = LocalPlugins::open(storage);
    let record = &reopened.records[id];
    assert!(record.enabled);
    assert_eq!(record.package.as_ref().unwrap().manifest.version, "1.1.0");
    let wireless = reopened.wireless_store();
    assert_eq!(wireless.package().unwrap().version(), "1.1.0");
    reopened.uninstall(id, false)?;
    assert!(!reopened.records.contains_key(id));
    assert!(wireless.package().is_none());
    Ok(())
}

#[test]
fn market_rejects_invalid_archives_and_catalogs() -> Result<()> {
    let fixture = Fixture::new()?;
    let manifest = fixture.project("1.0.0")?;
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("plugin.json", SimpleFileOptions::default())?;
    zip.write_all(&fs::read(&manifest)?)?;
    zip.add_symlink("main.js", "outside.js", SimpleFileOptions::default())?;
    zip.start_file(COVER_FILE, SimpleFileOptions::default())?;
    zip.write_all(include_bytes!("examples/payday/cover.png"))?;
    assert!(decode_zip(&zip.finish()?.into_inner(), true).is_err());

    let archive = |add_cover: bool, link: bool| -> Result<Vec<u8>> {
        let options = SimpleFileOptions::default();
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("plugin.json", options)?;
        zip.write_all(&fs::read(&manifest)?)?;
        zip.start_file("main.js", options)?;
        zip.write_all(include_bytes!("examples/payday/main.js"))?;
        if link {
            zip.add_symlink(COVER_FILE, "../outside.png", options)?;
        } else if add_cover {
            zip.start_file(COVER_FILE, options)?;
            zip.write_all(b"broken png")?;
        }
        Ok(zip.finish()?.into_inner())
    };
    for (add_cover, link) in [(false, false), (true, false), (true, true)] {
        assert!(decode_zip(&archive(add_cover, link)?, true).is_err());
    }

    let bundle = fixture.0.join("package.zip");
    let catalog = fixture.0.join("catalog.json");
    pack(&fixture.project("1.0.0")?, &bundle)?;
    submit(&bundle, &catalog)?;
    let mut data: Value = serde_json::from_slice(&fs::read(&catalog)?)?;
    let entry = data["plugins"][0].clone();
    data["plugins"] = json!([entry.clone(), entry]);
    fs::write(&catalog, serde_json::to_vec(&data)?)?;
    assert!(Catalog::open(&catalog).is_err());
    data["plugins"] = json!([entry]);
    for presentation in [
        json!({"updated_on": "2026-13-01"}),
        json!({"author_avatar": "https://example.invalid/avatar.png"}),
        json!({"author_avatar": "../avatar-renly1994.png"}),
        json!({"maintainer": "@island"}),
        json!({"downloads": -1}),
    ] {
        data["plugins"][0]["presentation"] = presentation;
        fs::write(&catalog, serde_json::to_vec(&data)?)?;
        assert!(Catalog::open(&catalog).is_err());
    }
    assert!(compare_versions("1.02.0", "1.2.0").is_none());
    assert!(compare_versions("1.4294967296.0", "1.2.0").is_none());
    Ok(())
}

#[test]
fn market_revocation_disables_installed_version_and_offers_newer_package() -> Result<()> {
    use super::super::market_state::MarketStatus;

    let fixture = Fixture::new()?;
    let id = "example.payday";
    let catalog_path = fixture.0.join("catalog.json");
    let first = fixture.0.join("first.zip");
    pack(&fixture.project("1.0.1")?, &first)?;
    submit(&first, &catalog_path)?;

    let storage = fixture.0.join("installed");
    let mut host = LocalPlugins::open(storage.clone());
    host.set_market_catalog(&catalog_path)?;
    host.market_action(false, id)?;
    wait_for_catalog_action(&mut host);
    host.enable(id, true)?;
    let mut index: Value = serde_json::from_slice(&fs::read(&catalog_path)?)?;
    index["plugins"][0]["revoked"] = json!(["invalid-version"]);
    fs::write(&catalog_path, serde_json::to_vec(&index)?)?;
    assert!(host.refresh_market_catalog().is_err());
    assert!(host.records[id].enabled);
    index["plugins"][0]["revoked"] = json!(["1.0.1"]);
    fs::write(&catalog_path, serde_json::to_vec(&index)?)?;
    host.refresh_market_catalog()?;
    assert!(!host.records[id].enabled);
    assert!(host.records[id].market_revoked);
    assert!(host.enable(id, true).is_err());
    assert!(host.market_action(false, id).is_err());
    let catalog = Catalog::open(&catalog_path)?;
    assert!(catalog.load(&catalog.entries[0]).is_err());
    let mut clean = LocalPlugins::open(fixture.0.join("clean"));
    clean.set_market_catalog(&catalog_path)?;
    assert!(clean.market_action(false, id).is_err());
    drop(host);

    let mut host = LocalPlugins::open(storage);
    assert!(host.records[id].market_revoked);
    let next = fixture.0.join("next.zip");
    pack(&fixture.project("1.0.2")?, &next)?;
    submit(&next, &catalog_path)?;
    let catalog = Catalog::open(&catalog_path)?;
    assert!(catalog.entries[0].revokes("1.0.1"));
    host.refresh_market_catalog()?;
    assert_eq!(host.market_status(&catalog, &catalog.entries[0]), MarketStatus::Update);
    host.market_action(false, id)?;
    wait_for_catalog_action(&mut host);
    assert!(!host.records[id].market_revoked);
    assert!(!host.records[id].enabled);
    host.enable(id, true)?;
    Ok(())
}

fn wait_for_catalog_action(host: &mut LocalPlugins) {
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    while host.market_busy() {
        assert!(std::time::Instant::now() < deadline, "等待集市操作超时");
        host.poll_market();
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn online_market_material_is_limited_to_preview_or_the_reviewed_stable_package() -> Result<()> {
    let preview = Url::parse(&format!("https://gitee.com{}?ref=master", official_index_path("preview")))?;
    let stable = Url::parse(&format!("https://gitee.com{}?ref=master", official_index_path("stable")))?;
    let approved: Entry = serde_json::from_value(json!({
        "id": "cayww.liquid-glass",
        "name": "液态玻璃",
        "author": "轻九九九（@cayww）",
        "version": "1.0.2",
        "size": 180301,
        "sha256": "01619b45b3ba6cf0a71c8d952c8a2aada5fad25379ced21e481784d561d197d8",
        "owner": "appearance"
    }))?;
    assert!(allows_market_material(None, false, &approved));
    assert!(allows_market_material(Some(&preview), true, &approved));
    assert!(!allows_market_material(Some(&preview), false, &approved));
    assert!(allows_market_material(Some(&stable), false, &approved));
    assert!(!allows_market_material(Some(&stable), true, &approved));

    for unreviewed in [
        Entry { id: "example.other-material".into(), ..approved.clone() },
        Entry { version: "1.0.3".into(), ..approved.clone() },
        Entry { sha256: "0".repeat(64), ..approved.clone() },
        Entry { size: approved.size + 1, ..approved.clone() },
    ] {
        assert!(allows_market_material(Some(&preview), true, &unreviewed));
        assert!(!allows_market_material(Some(&stable), false, &unreviewed));
    }

    for (trusted, preview_build) in [(&preview, true), (&stable, false)] {
        for untrusted in [
            trusted.as_str().replace("ref=master", "ref=main"),
            trusted.as_str().replace("https://", "http://"),
            trusted.as_str().replace("gitee.com/", "example.com/"),
            trusted.as_str().replace("gitee.com/", "gitee.com:8443/"),
            trusted.as_str().replace(GITEE_REPOSITORY, "another/daozai-plugins"),
            trusted.as_str().replace("catalog.json", "alternate.json"),
        ] {
            let index = Url::parse(&untrusted)?;
            assert!(!allows_market_material(Some(&index), preview_build, &approved), "{untrusted}");
        }
    }
    Ok(())
}

#[cfg(all(windows, feature = "sdk-material"))]
#[test]
fn market_material_packs_and_installs_liquid_glass_locally() -> Result<()> {
    let fixture = Fixture::new()?;
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("spike/examples/liquid-glass/plugin.json");
    let bundle = fixture.0.join("liquid-glass.zip");
    let catalog_path = fixture.0.join("catalog.json");
    pack(&project, &bundle)?;
    let raw = fs::read(&bundle)?;
    assert!(decode_zip(&raw, false).is_err());
    let preview = Url::parse(&format!("https://gitee.com{}?ref=master", official_index_path("preview")))?;
    submit(&bundle, &catalog_path)?;

    let catalog = Catalog::open(&catalog_path)?;
    let entry = &catalog.entries[0];
    let stable = Url::parse(&format!("https://gitee.com{}?ref=master", official_index_path("stable")))?;
    let preview_package = decode_zip(&raw, allows_market_material(Some(&preview), true, entry))?;
    assert!(preview_package.manifest.is_material());
    assert_eq!(preview_package.cover_png().as_deref(), Some(include_bytes!("examples/liquid-glass/cover.png").as_slice()));
    assert!(decode_zip(&raw, allows_market_material(Some(&stable), false, entry)).is_err());
    let id = "cayww.liquid-glass";
    assert_eq!(entry.id, id);
    assert_eq!(entry.author, "轻九九九（@cayww）");
    assert_eq!(catalog.cover_png(entry)?, include_bytes!("examples/liquid-glass/cover.png"));

    let storage = fixture.0.join("installed");
    let mut host = LocalPlugins::open(storage.clone());
    host.set_market_catalog(&catalog_path)?;
    host.market_action(false, id)?;
    wait_for_catalog_action(&mut host);
    let installed = &host.records[id];
    assert!(installed.from_market && !installed.official_market);
    assert!(!installed.enabled && !host.material_permission_granted(id));
    assert!(host.activate_material(Some(id), false).is_err());

    host.activate_material(Some(id), true)?;
    assert_eq!(host.selected_material_id(), Some(id));
    assert!(LocalPlugins::startup_material(&storage));
    let mut index: Value = serde_json::from_slice(&fs::read(&catalog_path)?)?;
    index["plugins"][0]["revoked"] = json!([entry.version]);
    fs::write(&catalog_path, serde_json::to_vec(&index)?)?;
    host.refresh_market_catalog()?;
    assert!(host.selected_material_id().is_none());
    assert!(host.activate_material(Some(id), true).is_err());
    assert!(!LocalPlugins::startup_material(&storage));

    let update_dir = fixture.0.join("update");
    fs::create_dir(&update_dir)?;
    let mut manifest: Value = serde_json::from_slice(&fs::read(&project)?)?;
    manifest["version"] = json!("1.0.3");
    let update = update_dir.join("plugin.json");
    fs::write(&update, serde_json::to_vec(&manifest)?)?;
    for name in [COVER_FILE, "material.wgsl"] {
        fs::copy(project.with_file_name(name), update_dir.join(name))?;
    }
    let next = fixture.0.join("next.zip");
    pack(&update, &next)?;
    submit(&next, &catalog_path)?;
    host.refresh_market_catalog()?;
    host.market_action(false, id)?;
    wait_for_catalog_action(&mut host);
    assert!(!host.records[id].market_revoked);
    assert!(host.selected_material_id().is_none());
    assert!(!LocalPlugins::startup_material(&storage));
    Ok(())
}

#[cfg(all(windows, feature = "sdk-material"))]
#[test]
fn market_material_rejects_invalid_wgsl_in_source_and_zip() -> Result<()> {
    let fixture = Fixture::new()?;
    let manifest = include_str!("examples/liquid-glass/plugin.json");
    let cover = include_bytes!("examples/liquid-glass/cover.png");
    let invalid = "fn material() { 123 }";
    let project = fixture.0.join("plugin.json");
    fs::write(&project, manifest)?;
    fs::write(fixture.0.join(COVER_FILE), cover)?;
    fs::write(fixture.0.join("material.wgsl"), invalid)?;
    let bundle = fixture.0.join("invalid.zip");
    assert!(validate_project(&project).is_err());
    assert!(pack(&project, &bundle).is_err());
    assert!(!bundle.exists());

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [("plugin.json", manifest.as_bytes()), (COVER_FILE, cover.as_slice()),
        ("material.wgsl", invalid.as_bytes())] {
        zip.start_file(name, SimpleFileOptions::default())?;
        zip.write_all(bytes)?;
    }
    fs::write(&bundle, zip.finish()?.into_inner())?;
    let catalog = fixture.0.join("catalog.json");
    assert!(submit(&bundle, &catalog).is_err());
    assert!(!catalog.exists());
    Ok(())
}

#[test]
fn update_removes_https_grant_when_host_is_removed() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut manifest: Value =
        serde_json::from_str(include_str!("examples/https-repository/plugin.json"))?;
    let path = fixture.0.join("plugin.json");
    fs::write(fixture.0.join("main.js"), include_str!("examples/https-repository/main.js"))?;
    fs::write(fixture.0.join(COVER_FILE), include_bytes!("examples/https-repository/cover.png"))?;
    fs::write(&path, serde_json::to_vec(&manifest)?)?;
    let catalog = fixture.0.join("catalog.json");
    let first = fixture.0.join("first.zip");
    pack(&path, &first)?;
    submit(&first, &catalog)?;
    let mut host = LocalPlugins::open(fixture.0.join("installed"));
    let id = "example.https-repository";
    host.set_market_catalog(&catalog)?;
    host.market_action(false, id)?;
    wait_for_catalog_action(&mut host);

    let grant = host.records[id].network().unwrap().grant("https://api.github.com/repos/rust-lang/rust")?;
    host.set_source_grant(id, Some(grant.clone()))?;
    assert_eq!(host.source_grant(id), Some(&grant));
    manifest["version"] = json!("1.0.1");
    manifest["network"] = json!({"hosts": ["example.org"]});
    fs::write(&path, serde_json::to_vec(&manifest)?)?;
    let next = fixture.0.join("next.zip");
    pack(&path, &next)?;
    submit(&next, &catalog)?;
    host.refresh_market_catalog()?;
    host.market_action(false, id)?;
    wait_for_catalog_action(&mut host);
    assert_eq!(host.records[id].package.as_ref().unwrap().manifest.version, "1.0.1");
    assert!(host.source_grant(id).is_none());
    assert!(host.records[id].source_grant.is_none());
    Ok(())
}
