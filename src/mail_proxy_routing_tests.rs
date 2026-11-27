use super::{decode_accounts, encode_accounts, net, secrets, Account, CustomImap, Event, Provider, Service, Status};
use crate::mail_manage::{Action, Folder, Request};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static TEST_PROXY: Mutex<Option<String>> = Mutex::new(None);

pub(super) fn test_proxy() -> Option<String> {
    TEST_PROXY.lock().unwrap().clone()
}

struct TestConnection {
    credential: String,
}

impl Drop for TestConnection {
    fn drop(&mut self) {
        *TEST_PROXY.lock().unwrap() = None;
        secrets::delete(&self.credential);
    }
}

#[test]
fn gmail_proxy_routes_connection_test_background_mail_and_management() {
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let account = Account {
        email: format!("test-{}-{nonce}@gmail.com", std::process::id()),
        provider: Provider::Gmail,
        custom: None,
        enabled: true,
        use_windows_proxy: true,
    };
    let account = decode_accounts(&encode_accounts(&[account])).pop().unwrap();
    let _connection = TestConnection { credential: account.credential_key() };
    let password = "synthetic-imap-password";
    secrets::save(&account.credential_key(), password).unwrap();
    *TEST_PROXY.lock().unwrap() = Some(proxy_url);

    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(8);
        for _ in 0..3 {
            let mut socket = loop {
                match proxy.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "缺少预期的 CONNECT 请求");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("代理测试监听失败：{e}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = BufReader::new(&mut socket);
            let mut line = String::new();
            request.read_line(&mut line).unwrap();
            assert_eq!(line.trim_end(), "CONNECT imap.gmail.com:993 HTTP/1.1");
            while !line.is_empty() && line != "\r\n" {
                line.clear();
                request.read_line(&mut line).unwrap();
                assert!(!line.contains(password));
            }
            socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n").unwrap();
        }
    });

    let service = Service::start();
    service.test(account.clone(), password.into());
    service.sync(std::slice::from_ref(&account), 120);
    service.manage(Request {
        account: account.clone(), folder: Folder::Inbox, validity: None,
        known: vec![], extra: vec![], action: Action::Load,
    });

    let deadline = Instant::now() + Duration::from_secs(8);
    let (mut tested, mut synced, mut managed) = (false, false, false);
    while !(tested && synced && managed) {
        assert!(Instant::now() < deadline, "连接测试、收信或管理缺少回执");
        match service.try_recv() {
            Some(Event::Test { account: email, ok, note }) => {
                assert_eq!(email, account.email);
                assert!(!ok && note.contains("403"));
                tested = true;
            }
            Some(Event::Status { account: email, status: Status::Failed(error) }) => {
                assert_eq!(email, account.email);
                assert!(error.contains("403"));
                synced = true;
            }
            Some(Event::Managed(completed)) => {
                assert_eq!(completed.request.account, account);
                assert!(completed.result.unwrap_err().contains("403"));
                managed = true;
            }
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    service.sync(&[], 120);
    server.join().unwrap();

    let direct = TcpListener::bind("127.0.0.1:0").unwrap();
    let custom = Account {
        provider: Provider::Custom,
        custom: Some(CustomImap { host: "127.0.0.1".into(), port: direct.local_addr().unwrap().port(), username: "test".into() }),
        ..account
    };
    let direct_request = std::thread::spawn(move || {
        direct.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok((mut socket, _)) = direct.accept() {
                socket.set_nonblocking(false).unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut record_type = [0];
                socket.read_exact(&mut record_type).unwrap();
                assert_eq!(record_type, [0x16], "其他邮箱应直接向邮箱服务器发起 TLS 握手");
                break;
            }
            assert!(Instant::now() < deadline, "其他邮箱未走直连");
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    assert!(net::probe(&custom, password).unwrap_err().contains("TLS"));
    direct_request.join().unwrap();
}
