use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROXY_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const PROXY_RESPONSE_LIMIT: usize = 8 * 1024;

pub(super) fn connect(host: &str, port: u16, use_windows_proxy: bool) -> Result<TcpStream, String> {
    if !use_windows_proxy {
        return connect_tcp((host, port)).map_err(|error| format!("无法连接 {host}：{error}"));
    }
    let proxy = windows_proxy().ok_or("未检测到可用的 Windows 手动 HTTP 代理，请检查系统代理设置")?;
    connect_via_proxy(host, port, &proxy, PROXY_HANDSHAKE_TIMEOUT)
}

fn windows_proxy() -> Option<String> {
    #[cfg(test)]
    if let Some(proxy) = super::proxy_routing_tests::test_proxy() {
        return Some(proxy);
    }
    crate::quota::system_proxy()
}

fn connect_tcp(endpoint: impl ToSocketAddrs) -> io::Result<TcpStream> {
    let addresses: Vec<_> = endpoint.to_socket_addrs()?.collect();
    let deadline = Instant::now() + TCP_CONNECT_TIMEOUT;
    let mut last_error = io::Error::new(io::ErrorKind::AddrNotAvailable, "没有可用的网络地址");
    for (index, address) in addresses.iter().enumerate() {
        let remaining = time_left(deadline)?;
        let timeout = remaining / (addresses.len() - index) as u32;
        match TcpStream::connect_timeout(address, timeout.max(Duration::from_millis(1))) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn connect_via_proxy(host: &str, port: u16, proxy: &str, handshake_timeout: Duration) -> Result<TcpStream, String> {
    let proxy = url::Url::parse(proxy).map_err(|_| "Windows 系统代理地址无效")?;
    if proxy.scheme() != "http" {
        return Err("邮箱目前支持 HTTP 类型的 Windows 系统代理".into());
    }
    if !proxy.username().is_empty() || proxy.password().is_some() {
        return Err("邮箱目前不支持代理认证".into());
    }
    if proxy.host().is_none() || proxy.path() != "/" || proxy.query().is_some() || proxy.fragment().is_some() {
        return Err("Windows 系统代理地址无效".into());
    }
    let connection_error = |error| format!("无法连接 Windows 系统代理：{error}");
    let addresses = proxy.socket_addrs(|| None).map_err(connection_error)?;
    let mut stream = connect_tcp(addresses.as_slice()).map_err(connection_error)?;
    http_connect(&mut stream, host, port, handshake_timeout)?;
    Ok(stream)
}

fn time_left(deadline: Instant) -> io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() { Err(io::ErrorKind::TimedOut.into()) } else { Ok(remaining) }
}

fn handshake_error(error: io::Error) -> String {
    if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) {
        "Windows 系统代理握手超时".into()
    } else {
        format!("Windows 系统代理握手失败：{error}")
    }
}

fn http_connect(stream: &mut TcpStream, host: &str, port: u16, timeout: Duration) -> Result<(), String> {
    let request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n");
    let deadline = Instant::now() + timeout;
    let mut pending = request.as_bytes();
    while !pending.is_empty() {
        let remaining = time_left(deadline).map_err(handshake_error)?;
        stream.set_write_timeout(Some(remaining)).map_err(handshake_error)?;
        let sent = stream.write(pending).map_err(handshake_error)?;
        if sent == 0 {
            return Err("Windows 系统代理未接收 CONNECT 请求".into());
        }
        pending = &pending[sent..];
    }

    // 只读到头部结束，保留隧道中紧跟其后的字节供 TLS 握手使用。
    let mut header = Vec::new();
    let mut byte = [0];
    for _ in 0..PROXY_RESPONSE_LIMIT {
        let remaining = time_left(deadline).map_err(handshake_error)?;
        stream.set_read_timeout(Some(remaining)).map_err(handshake_error)?;
        stream.read_exact(&mut byte).map_err(handshake_error)?;
        header.push(byte[0]);
        if !header.ends_with(b"\r\n\r\n") {
            continue;
        }
        let line = header.split(|&byte| byte == b'\r').next().unwrap_or_default();
        let mut fields = line.split(|&byte| byte == b' ');
        let version = fields.next().unwrap_or_default();
        let status = fields.next().filter(|code| code.len() == 3 && code.iter().all(u8::is_ascii_digit))
            .and_then(|code| std::str::from_utf8(code).ok())
            .and_then(|code| code.parse::<u16>().ok());
        match status.filter(|_| matches!(version, b"HTTP/1.0" | b"HTTP/1.1")) {
            Some(200..=299) => return Ok(()),
            Some(100 | 102..=199) => header.clear(),
            Some(407) => return Err("Windows 系统代理需要认证（407）".into()),
            Some(status) => return Err(format!("Windows 系统代理拒绝连接（{status}）")),
            None => return Err("Windows 系统代理响应无效".into()),
        }
    }
    Err("Windows 系统代理响应头过长".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    fn mock_proxy(response: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut request = String::new();
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            socket.write_all(&response).unwrap();
            request
        });
        (address, worker)
    }

    #[test]
    fn mail_proxy_direct_connection_does_not_need_proxy_settings() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stream = connect("127.0.0.1", address.port(), false).unwrap();
        assert_eq!(stream.peer_addr().unwrap(), address);
        assert_eq!(listener.accept().unwrap().1, stream.local_addr().unwrap());
    }

    #[test]
    fn mail_proxy_connect_uses_target_and_preserves_tunnel_bytes() {
        let (proxy, worker) = mock_proxy(b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 204 Connected\r\n\r\ntunnel".to_vec());
        let mut stream = connect_via_proxy("imap.gmail.com", 993, &proxy, Duration::from_secs(2)).unwrap();
        let mut payload = [0; 6];
        stream.read_exact(&mut payload).unwrap();
        assert_eq!(&payload, b"tunnel");
        assert_eq!(worker.join().unwrap(), "CONNECT imap.gmail.com:993 HTTP/1.1\r\nHost: imap.gmail.com:993\r\n\r\n");
    }

    #[test]
    fn mail_proxy_rejections_have_clear_errors() {
        for (response, message) in [
            (b"HTTP/1.1 403 Forbidden\r\n\r\n".as_slice(), "拒绝连接（403）"),
            (b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n", "需要认证（407）"),
            (b"not-http 200 OK\r\n\r\n", "代理响应无效"),
        ] {
            let (proxy, worker) = mock_proxy(response.to_vec());
            let error = connect_via_proxy("imap.gmail.com", 993, &proxy, Duration::from_secs(2)).unwrap_err();
            assert!(error.contains(message), "{error}");
            worker.join().unwrap();
        }
    }

    #[test]
    fn mail_proxy_response_headers_are_bounded() {
        let mut response = b"HTTP/1.1 200 OK\r\nX-Filler: ".to_vec();
        response.extend(std::iter::repeat_n(b'x', PROXY_RESPONSE_LIMIT));
        response.extend_from_slice(b"\r\n\r\n");
        let (proxy, worker) = mock_proxy(response);
        let error = connect_via_proxy("imap.gmail.com", 993, &proxy, Duration::from_secs(2)).unwrap_err();
        assert!(error.contains("响应头过长"), "{error}");
        worker.join().unwrap();
    }

    #[test]
    fn mail_proxy_handshake_has_an_overall_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nX-Filler: ");
            for _ in 0..30 {
                std::thread::sleep(Duration::from_millis(20));
                if socket.write_all(b"x").is_err() {
                    break;
                }
            }
        });
        let error = connect_via_proxy("imap.gmail.com", 993, &proxy, Duration::from_millis(150)).unwrap_err();
        assert!(error.contains("代理握手超时"), "{error}");
        worker.join().unwrap();
    }

    #[test]
    fn mail_proxy_rejects_unavailable_protocols_and_credentials() {
        for (proxy, message) in [
            ("socks5://127.0.0.1:1080", "支持 HTTP"),
            ("http://user:secret@127.0.0.1:8080", "不支持代理认证"),
            ("http://127.0.0.1:8080/wrong", "代理地址无效"),
        ] {
            let error = connect_via_proxy("imap.gmail.com", 993, proxy, Duration::from_secs(2)).unwrap_err();
            assert!(error.contains(message), "{error}");
            assert!(!error.contains("secret"));
        }
    }
}
