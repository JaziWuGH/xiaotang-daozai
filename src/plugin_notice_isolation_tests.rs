use super::*;

#[test]
fn notice_transport_without_sources_does_not_create_runtime_files() {
    let dir = TestDir::new();
    let bridge = Bridge::start(&dir.0, egui::Context::default()).unwrap();
    assert!(bridge.sources().is_empty());
    assert!(bridge.listener.is_none());
    assert!(bridge.drain().is_empty());
    assert!(!dir.0.exists());
}

#[test]
fn notice_transport_rejects_corrupt_authorizations_without_rewriting_them() {
    let dir = TestDir::new();
    let sources = dir.0.join("plugin-notices/sources.json");
    fs::create_dir_all(sources.parent().unwrap()).unwrap();
    fs::write(&sources, b"[{\"id\":").unwrap();

    assert!(Bridge::start(&dir.0, egui::Context::default()).is_err());
    assert_eq!(fs::read(&sources).unwrap(), b"[{\"id\":");
    assert_eq!(fs::read_dir(sources.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn notice_transport_bad_clients_do_not_block_the_next_source() {
    let dir = TestDir::new();
    let mut bridge = Bridge::start(&dir.0, egui::Context::default()).unwrap();
    bridge.authorize("example.client", "示例客户端").unwrap();
    let connection = descriptor(&bridge, "example.client");
    let addr = (
        Ipv4Addr::LOCALHOST,
        connection["port"].as_u64().unwrap() as u16,
    );

    let mut malformed = TcpStream::connect(addr).unwrap();
    malformed
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    malformed.write_all(b"{invalid}\n").unwrap();
    let mut line = String::new();
    BufReader::new(malformed).read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response, json!({"ok": false, "error": "invalid_request"}));

    let mut interrupted = TcpStream::connect(addr).unwrap();
    interrupted.write_all(b"{\"method\":\"complete\"").unwrap();
    interrupted.shutdown(std::net::Shutdown::Write).unwrap();
    drop(interrupted);

    assert_eq!(
        exchange(&connection, json!({"method": "status"}))["ok"],
        true
    );
    assert!(bridge.drain().is_empty());
    let result = exchange(&connection, json!({"method": "complete", "id": "healthy"}));
    assert_eq!(result["ok"], true);
    assert!(
        matches!(bridge.drain().as_slice(), [Event::Completed(notice)] if notice.id == "healthy")
    );
}
