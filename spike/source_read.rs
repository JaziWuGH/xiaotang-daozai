//! 用户授权的单一数据源。路径、地址和 I/O 始终由宿主管理。
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use url::{Host, Url};

const MAX_BYTES: u64 = 128 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);
const FILE_ERROR: &str = "请选择普通的本地文件。网络路径和重解析点不受支持";
const URL_ERROR: &str = "请输入 http://127.0.0.1、http://[::1] 或 http://localhost 的完整接口地址";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceKind {
    File,
    Loopback,
    #[cfg(windows)]
    Https,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum SourceGrant {
    File(PathBuf),
    Loopback(String),
    #[cfg(windows)]
    Https { url: String },
    #[cfg(windows)]
    HttpsSources { hosts: Vec<String>, sources: Vec<super::source_https::NetworkSource> },
}

impl SourceGrant {
    pub(super) fn kind(&self) -> SourceKind {
        match self {
            Self::File(_) => SourceKind::File,
            Self::Loopback(_) => SourceKind::Loopback,
            #[cfg(windows)]
            Self::Https { .. } | Self::HttpsSources { .. } => SourceKind::Https,
        }
    }

    pub(super) fn read(&self, cancelled: &AtomicBool) -> io::Result<String> {
        check_cancelled(cancelled)?;
        match self {
            Self::File(path) => read_file(path, cancelled),
            Self::Loopback(address) => read_http(address, cancelled),
            #[cfg(windows)]
            Self::Https { .. } | Self::HttpsSources { .. } => Err(io::Error::other("HTTPS 数据源缺少网络配置")),
        }
    }
}

pub(super) fn grant_file(path: &Path) -> Result<SourceGrant, String> {
    is_file_path(path)
        .then(|| SourceGrant::File(path.to_path_buf()))
        .ok_or_else(|| FILE_ERROR.into())
}

pub(super) fn grant_url(raw: &str) -> Result<SourceGrant, String> {
    let (url, _) = loopback_url(raw.trim()).ok_or_else(|| URL_ERROR.to_owned())?;
    Ok(SourceGrant::Loopback(url.into()))
}

fn loopback_url(raw: &str) -> Option<(Url, Vec<SocketAddr>)> {
    if raw.len() > 2048 || raw.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    const V4: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    const V6: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);
    let addresses: &[IpAddr] = match url.host()? {
        Host::Domain("localhost") => &[V4, V6],
        Host::Ipv4(Ipv4Addr::LOCALHOST) => &[V4],
        Host::Ipv6(Ipv6Addr::LOCALHOST) => &[V6],
        _ => return None,
    };
    let port = url.port_or_known_default()?;
    if port == 0 {
        return None;
    }
    Some((
        url,
        addresses
            .iter()
            .map(|ip| SocketAddr::new(*ip, port))
            .collect(),
    ))
}

fn read_http(raw: &str, cancelled: &AtomicBool) -> io::Result<String> {
    let (url, addresses) = loopback_url(raw).ok_or_else(|| io::Error::other(URL_ERROR))?;
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .resolver(move |_: &str| Ok(addresses.clone()))
        .build();
    check_cancelled(cancelled)?;
    let response = agent
        .get(url.as_str())
        .timeout(HTTP_TIMEOUT)
        .set("Accept", "application/json, text/plain")
        .set("Accept-Encoding", "identity")
        .call()
        .map_err(|_| io::Error::other("本机数据源无法读取"))?;
    if !(200..300).contains(&response.status()) {
        return Err(io::Error::other("本机数据源没有直接返回数据"));
    }
    read_text(response.into_reader(), cancelled, MAX_BYTES)
}

pub(super) fn check_cancelled(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "读取已取消"));
    }
    Ok(())
}

pub(super) fn read_text(reader: impl Read, cancelled: &AtomicBool, limit: u64) -> io::Result<String> {
    let mut reader = reader.take(limit + 1);
    let mut bytes = Vec::new();
    let mut buf = [0; 4096];
    loop {
        check_cancelled(cancelled)?;
        let count = reader.read(&mut buf)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..count]);
    }
    if bytes.len() as u64 > limit {
        return Err(io::Error::other(format!("数据源超过 {} KiB", limit / 1024)));
    }
    let mut text =
        String::from_utf8(bytes).map_err(|_| io::Error::other("数据源必须使用 UTF-8"))?;
    if text.starts_with('\u{feff}') {
        text.drain(..'\u{feff}'.len_utf8());
    }
    check_cancelled(cancelled)?;
    Ok(text)
}

#[cfg(windows)]
fn is_file_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    let mut parts = path.components();
    matches!(parts.next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)))
        && matches!(parts.next(), Some(Component::RootDir))
        && path.file_name().is_some() && parts.all(|part| {
        matches!(part, Component::Normal(name) if name.to_str().is_some_and(|text| {
            !text.contains(':') && !text.chars().any(char::is_control)
                && !text.ends_with([' ', '.'])
                && !matches!(text.split('.').next().unwrap().to_ascii_uppercase().as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                    | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
                    | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
                    | "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³")
        }))
    })
}

#[cfg(windows)]
fn read_file(path: &Path, cancelled: &AtomicBool) -> io::Result<String> {
    use std::{
        fs,
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
    };
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{
                GetDriveTypeW, GetFileType, GetFinalPathNameByHandleW,
                FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_TYPE_DISK,
                VOLUME_NAME_DOS,
            },
        },
    };

    let invalid = || io::Error::other(FILE_ERROR);
    if !is_file_path(path) {
        return Err(invalid());
    }
    let text = path.to_str().ok_or_else(invalid)?.replace('/', "\\");
    let root: Vec<_> = text[..3].encode_utf16().chain([0]).collect();
    if !matches!(
        unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) },
        2 | 3 | 5 | 6
    ) {
        return Err(invalid());
    }
    // 先检查目录组件，避免沿常见的重解析点访问远程位置。
    let components: Vec<_> = path.ancestors().collect();
    for part in components.iter().rev().skip(1) {
        check_cancelled(cancelled)?;
        let metadata = fs::symlink_metadata(part)?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(invalid());
        }
    }
    check_cancelled(cancelled)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    let handle = HANDLE(file.as_raw_handle());
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || unsafe { GetFileType(handle) } != FILE_TYPE_DISK
    {
        return Err(invalid());
    }
    // 校验已打开的句柄，文件必须对应用户批准的路径。
    let mut resolved = vec![0; 32768];
    let count =
        unsafe { GetFinalPathNameByHandleW(handle, &mut resolved, VOLUME_NAME_DOS) } as usize;
    if count == 0 || count >= resolved.len() {
        return Err(invalid());
    }
    let resolved = String::from_utf16(&resolved[..count]).map_err(|_| invalid())?;
    if !resolved
        .strip_prefix(r"\\?\")
        .is_some_and(|name| name.eq_ignore_ascii_case(&text))
    {
        return Err(invalid());
    }
    read_text(file, cancelled, MAX_BYTES)
}

#[cfg(not(windows))]
fn is_file_path(_: &Path) -> bool {
    false
}

#[cfg(not(windows))]
fn read_file(_: &Path, _: &AtomicBool) -> io::Result<String> {
    Err(io::Error::other(FILE_ERROR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_explicit_loopback_addresses_and_bounded_utf8() {
        for addr in [
            "http://127.0.0.1:8080/status?item=2",
            "http://localhost/status",
            "http://[::1]/status",
        ] {
            assert!(grant_url(addr).is_ok());
        }
        for addr in [
            "https://127.0.0.1/status",
            "http://192.168.1.2/",
            "http://localhost.evil.test/",
            "http://user:pass@127.0.0.1/",
            "http://127.0.0.1/#fragment",
            "http://127.0.0.1:0/",
        ] {
            assert!(grant_url(addr).is_err(), "{addr}");
        }
        let cancelled = AtomicBool::new(false);
        assert!(read_text(io::repeat(b'x'), &cancelled, MAX_BYTES).is_err());
        assert!(read_text(&[0xff][..], &cancelled, MAX_BYTES).is_err());
        cancelled.store(true, Ordering::Release);
        assert_eq!(
            read_text(&b"ignored"[..], &cancelled, MAX_BYTES).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn reads_only_the_approved_endpoint_without_following_redirects() {
        use std::{io::Write, net::TcpListener, thread};
        let chosen = TcpListener::bind("127.0.0.1:0").unwrap();
        let redirected = TcpListener::bind("127.0.0.1:0").unwrap();
        chosen.set_nonblocking(true).unwrap();
        redirected.set_nonblocking(true).unwrap();
        let location = redirected.local_addr().unwrap();
        let grant = grant_url(&format!("http://{}/status", chosen.local_addr().unwrap())).unwrap();
        let responder = thread::spawn(move || {
            let timeout = std::time::Instant::now() + HTTP_TIMEOUT;
            let mut stream = loop {
                match chosen.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < timeout);
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(HTTP_TIMEOUT)).unwrap();
            let mut lines = io::BufRead::lines(io::BufReader::new(&mut stream));
            assert_eq!(lines.next().unwrap().unwrap(), "GET /status HTTP/1.1");
            for line in lines {
                if line.unwrap().is_empty() {
                    break;
                }
            }
            write!(stream, "HTTP/1.1 302 Found\r\nLocation: http://{location}/other\r\nContent-Length: 0\r\n\r\n").unwrap();
        });
        let result = grant.read(&AtomicBool::new(false));
        responder.join().unwrap();
        assert_eq!(result.unwrap_err().to_string(), "本机数据源没有直接返回数据");
        assert!(
            matches!(redirected.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
        );
    }

    #[cfg(windows)]
    #[test]
    fn rejects_path_aliases_and_devices() {
        for path in [
            r"D:\logs\..\secrets.json",
            r"\\server\share\data.json",
            r"\\.\pipe\data",
            r"C:\logs\data.json:stream",
            r"C:\logs\NUL.txt",
            "data.json",
        ] {
            assert!(grant_file(Path::new(path)).is_err(), "{path}");
        }
    }
}
