//! 关于页感谢墙的后台刷新与离线快照。

use eframe::egui;
use image::{codecs::webp::WebPDecoder, ImageDecoder};
use serde::Deserialize;
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

const BASE_URL: &str = "https://renly1994.github.io/xiaotang-daozai/supporters/v1/";
const CHECK_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
const MANIFEST_LIMIT: usize = 4 * 1024;
const RECENT_LIMIT: usize = 256 * 1024;
const WALL_LIMIT: usize = 2 * 1024 * 1024;
const CACHE_FILE: &str = "snapshot-v1.bin";
const CHECK_FILE: &str = "checked-v1";
const CHECK_FAILED: &[u8] = b"-";
const CACHE_HEADER_SIZE: usize = 8;

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub(crate) version: String,
    pub(crate) count: usize,
    pub(crate) recent: Arc<[u8]>,
    pub(crate) wall: Arc<[u8]>,
}

pub(crate) struct State {
    cache_dir: PathBuf,
    snapshot: Option<Snapshot>,
    pending: Option<mpsc::Receiver<Event>>,
    next_check: Option<Instant>,
}

enum Event {
    Cached(Snapshot),
    Checked(Option<Snapshot>, Instant),
}

impl State {
    pub(crate) fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir, snapshot: None, pending: None, next_check: None }
    }

    pub(crate) fn snapshot(&self) -> Option<&Snapshot> { self.snapshot.as_ref() }

    pub(crate) fn refresh(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(pending) = &self.pending {
            loop {
                match pending.try_recv() {
                    Ok(Event::Cached(snapshot)) => self.snapshot = Some(snapshot),
                    Ok(Event::Checked(snapshot, next)) => {
                        if let Some(snapshot) = snapshot { self.snapshot = Some(snapshot); }
                        self.next_check = Some(next);
                        self.pending = None;
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.next_check = Some(now + RETRY_AFTER);
                        self.pending = None;
                        return;
                    }
                    Err(mpsc::TryRecvError::Empty) => return,
                }
            }
        }
        if cfg!(test) || self.next_check.is_some_and(|next| now < next) { return; }

        let cache_dir = self.cache_dir.clone();
        let mut version = self.snapshot.as_ref().map(|s| s.version.clone());
        let viewport = ctx.viewport_id();
        let ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::Builder::new().name("daozai-supporters".into()).spawn(move || {
            if version.is_none() {
                let cached = fs::File::open(cache_dir.join(CACHE_FILE)).ok()
                    .and_then(|file| bounded_read(file, CACHE_HEADER_SIZE + MANIFEST_LIMIT + RECENT_LIMIT + WALL_LIMIT))
                    .and_then(|bytes| unpack(&bytes));
                if let Some(snapshot) = cached {
                    version = Some(snapshot.version.clone());
                    if tx.send(Event::Cached(snapshot)).is_err() { return; }
                    ctx.request_repaint_of(viewport);
                }
            }
            let known = version.as_deref();
            let (snapshot, next) = check_when_due(&cache_dir, known, || {
                check(&crate::quota::http_agent(), BASE_URL, known, &cache_dir)
            });
            let _ = tx.send(Event::Checked(snapshot, next));
            ctx.request_repaint_of(viewport);
        });
        if worker.is_ok() {
            self.pending = Some(rx);
        } else {
            self.next_check = Some(now + RETRY_AFTER);
        }
    }
}

fn check_when_due(
    cache_dir: &Path,
    known: Option<&str>,
    fetch: impl FnOnce() -> Result<Option<Snapshot>, ()>,
) -> (Option<Snapshot>, Instant) {
    let marker = cache_dir.join(CHECK_FILE);
    if let Some(remaining) = remaining_wait(&marker, known) {
        return (None, Instant::now() + remaining);
    }

    let result = fetch();
    let (snapshot, delay) = match result {
        Ok(snapshot) => (snapshot, CHECK_AFTER),
        Err(()) => (None, RETRY_AFTER),
    };
    // 成功标记必须与当前可用的快照版号一致，失败标记也适用于首次离线。
    let mark = if delay == RETRY_AFTER { Some(CHECK_FAILED) }
        else { snapshot.as_ref().map(|data| data.version.as_str()).or(known).map(str::as_bytes) };
    if let Some(mark) = mark {
        if fs::create_dir_all(cache_dir).is_ok() { let _ = crate::model::atomic_write(&marker, mark); }
    }
    (snapshot, Instant::now() + delay)
}

fn remaining_wait(marker: &Path, known: Option<&str>) -> Option<Duration> {
    let file = fs::File::open(marker).ok()?;
    let modified = file.metadata().ok()?.modified().ok()?;
    let mark = bounded_read(file, 24)?;
    let delay = if mark == CHECK_FAILED { RETRY_AFTER }
        else if known.is_some_and(|version| version.as_bytes() == mark) { CHECK_AFTER }
        else { return None };
    delay.checked_sub(modified.elapsed().ok()?).filter(|remaining| !remaining.is_zero())
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
struct Manifest {
    version: String,
    count: usize,
    recent: String,
    wall: String,
}

impl Manifest {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let manifest: Self = serde_json::from_slice(bytes).ok()?;
        let version = &manifest.version;
        // 由版号限定文件名，清单无法把请求引到站点外或其他目录。
        (manifest.count > 0
            && version.len() == 24
            && version.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && manifest.recent == format!("recent-{version}.webp")
            && manifest.wall == format!("wall-{version}.webp"))
            .then_some(manifest)
    }

    fn snapshot(&self, recent: &[u8], wall: &[u8]) -> Option<Snapshot> {
        (self.version == bundle_version(self.count, recent, wall)
            && valid_image(recent, 1024, 256)
            && valid_image(wall, 1600, 8000))
            .then(|| Snapshot {
                version: self.version.clone(),
                count: self.count,
                recent: Arc::from(recent),
                wall: Arc::from(wall),
            })
    }
}

fn bundle_version(count: usize, recent: &[u8], wall: &[u8]) -> String {
    use ring::digest::{Context, SHA256};
    let mut hash = Context::new(&SHA256);
    hash.update(&(count as u64).to_le_bytes());
    hash.update(recent);
    hash.update(wall);
    hash.finish().as_ref()[..12].iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_image(bytes: &[u8], max_width: u32, max_height: u32) -> bool {
    let Ok(decoder) = WebPDecoder::new(Cursor::new(bytes)) else { return false };
    let (width, height) = decoder.dimensions();
    width > 0 && height > 0 && width <= max_width && height <= max_height
        && u64::from(width) * u64::from(height) <= 8_000_000
        && !decoder.has_animation()
        && image::DynamicImage::from_decoder(decoder).is_ok()
}

fn bounded_read(input: impl Read, limit: usize) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    input.take((limit + 1) as u64).read_to_end(&mut body).ok()?;
    (body.len() <= limit).then_some(body)
}

fn fetch(agent: &ureq::Agent, url: &str, limit: usize) -> Option<Vec<u8>> {
    let response = agent.get(url).set("User-Agent", concat!("Daozai/", env!("CARGO_PKG_VERSION"))).call().ok()?;
    if response.status() != 200 { return None; }
    bounded_read(response.into_reader(), limit)
}

fn check(agent: &ureq::Agent, base: &str, known: Option<&str>, cache_dir: &Path) -> Result<Option<Snapshot>, ()> {
    let bytes = fetch(agent, &format!("{base}supporters.json"), MANIFEST_LIMIT).ok_or(())?;
    let manifest = Manifest::parse(&bytes).ok_or(())?;
    if known == Some(manifest.version.as_str()) { return Ok(None); }
    let recent = fetch(agent, &format!("{base}{}", manifest.recent), RECENT_LIMIT).ok_or(())?;
    let wall = fetch(agent, &format!("{base}{}", manifest.wall), WALL_LIMIT).ok_or(())?;
    let snapshot = manifest.snapshot(&recent, &wall).ok_or(())?;

    // 先校验整版，再原子替换一个文件，缓存与界面始终使用同一批图。
    if fs::create_dir_all(cache_dir).is_ok() {
        let packed = pack(&bytes, &recent, &wall);
        let _ = crate::model::atomic_write(&cache_dir.join(CACHE_FILE), &packed);
    }
    Ok(Some(snapshot))
}

fn pack(manifest: &[u8], recent: &[u8], wall: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(CACHE_HEADER_SIZE + manifest.len() + recent.len() + wall.len());
    bytes.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(recent.len() as u32).to_le_bytes());
    bytes.extend_from_slice(manifest);
    bytes.extend_from_slice(recent);
    bytes.extend_from_slice(wall);
    bytes
}

fn unpack(bytes: &[u8]) -> Option<Snapshot> {
    let manifest_len = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
    let recent_len = u32::from_le_bytes(bytes.get(4..CACHE_HEADER_SIZE)?.try_into().ok()?) as usize;
    if manifest_len > MANIFEST_LIMIT || recent_len > RECENT_LIMIT { return None; }
    let (manifest, images) = bytes.get(CACHE_HEADER_SIZE..)?.split_at_checked(manifest_len)?;
    let (recent, wall) = images.split_at_checked(recent_len)?;
    if wall.len() > WALL_LIMIT { return None; }
    Manifest::parse(manifest)?.snapshot(recent, wall)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Manifest, Vec<u8>, Vec<u8>) {
        fn webp(color: u8) -> Vec<u8> {
            let mut bytes = Vec::new();
            image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
                .encode(&[color; 12], 2, 2, image::ExtendedColorType::Rgb8).unwrap();
            bytes
        }
        let recent = webp(120);
        let wall = webp(140);
        (manifest_for(107, &recent, &wall), recent, wall)
    }

    fn manifest_for(count: usize, recent: &[u8], wall: &[u8]) -> Manifest {
        let version = bundle_version(count, recent, wall);
        Manifest { count, recent: format!("recent-{version}.webp"), wall: format!("wall-{version}.webp"), version }
    }

    #[test]
    fn supporters_reject_off_origin_paths() {
        let (mut manifest, _, _) = fixture();
        manifest.wall = "https://example.org/wall.webp".to_owned();
        assert!(Manifest::parse(&serde_json::to_vec(&manifest).unwrap()).is_none());
        manifest.wall = "../wall.webp".to_owned();
        assert!(Manifest::parse(&serde_json::to_vec(&manifest).unwrap()).is_none());
    }

    #[test]
    fn supporters_reject_incomplete_or_mixed_cache() {
        let (manifest, recent, wall) = fixture();
        let json = serde_json::to_vec(&manifest).unwrap();
        let bytes = pack(&json, &recent, &wall);
        let snapshot = unpack(&bytes).unwrap();
        assert_eq!(snapshot.count, 107);
        assert_eq!(&*snapshot.recent, recent);
        assert_eq!(&*snapshot.wall, wall);
        assert!(unpack(&bytes[..bytes.len() - 1]).is_none());
        assert!(unpack(&pack(&json, &recent, &recent)).is_none());
    }

    fn serve(replies: Vec<(String, Option<Vec<u8>>)>) -> (String, std::thread::JoinHandle<()>) {
        use std::{io::{BufRead, BufReader, Write}, net::TcpListener};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (path, body) in replies {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("未收到 {path} 请求：{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut headers = BufReader::new(&stream).lines();
                assert!(headers.next().unwrap().unwrap().starts_with(&format!("GET /{path} ")));
                for line in headers { if line.unwrap().is_empty() { break; } }
                let status = if body.is_some() { "200 OK" } else { "404 Not Found" };
                let body = body.unwrap_or_default();
                write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        (base, server)
    }

    fn test_agent() -> ureq::Agent {
        ureq::AgentBuilder::new().try_proxy_from_env(false).redirects(0).timeout(Duration::from_secs(5)).build()
    }

    #[test]
    fn supporters_unchanged_manifest_does_not_request_images() {
        let (manifest, _, _) = fixture();
        let (base, server) = serve(vec![("supporters.json".into(), Some(serde_json::to_vec(&manifest).unwrap()))]);
        assert!(matches!(check(&test_agent(), &base, Some(&manifest.version), Path::new("")), Ok(None)));
        server.join().unwrap();
    }

    #[test]
    fn supporters_network_commits_only_a_complete_snapshot() {
        let (manifest, recent, wall) = fixture();
        let dir = std::env::temp_dir().join(format!("daozai-supporters-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let (base, server) = serve(vec![
            ("supporters.json".into(), Some(serde_json::to_vec(&manifest).unwrap())),
            (manifest.recent.clone(), Some(recent.clone())),
            (manifest.wall.clone(), Some(wall.clone())),
        ]);
        let snapshot = check(&test_agent(), &base, None, &dir).unwrap().unwrap();
        server.join().unwrap();
        let cache = dir.join(CACHE_FILE);
        let original = fs::read(&cache).unwrap();
        assert_eq!(snapshot.version, manifest.version);
        let cached = unpack(&original).unwrap();
        assert_eq!(cached.version, manifest.version);
        assert_eq!(&*cached.recent, recent);
        assert_eq!(&*cached.wall, wall);

        let next = manifest_for(108, &recent, &wall);
        let (base, server) = serve(vec![
            ("supporters.json".into(), Some(serde_json::to_vec(&next).unwrap())),
            (next.recent, Some(recent)),
            (next.wall, None),
        ]);
        assert!(check(&test_agent(), &base, Some(&snapshot.version), &dir).is_err());
        server.join().unwrap();
        assert_eq!(fs::read(&cache).unwrap(), original);
        fs::remove_file(cache).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn supporters_restart_respects_saved_deadlines() {
        use std::{fs::FileTimes, time::SystemTime};

        let (manifest, recent, wall) = fixture();
        let dir = std::env::temp_dir().join(format!("daozai-supporters-check-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let snapshot = manifest.snapshot(&recent, &wall).unwrap();
        assert!(check_when_due(&dir, None, || Ok(Some(snapshot))).0.is_some());
        let marker = dir.join(CHECK_FILE);
        let opened = fs::OpenOptions::new().write(true).open(&marker).unwrap();
        let age = Duration::from_secs(60 * 60);
        opened.set_times(FileTimes::new().set_modified(SystemTime::now() - age)).unwrap();
        drop(opened);
        let (_, next) = check_when_due(&dir, Some(&manifest.version), || panic!("有效缓存应跳过网络"));
        let remaining = next.saturating_duration_since(Instant::now());
        assert!(remaining <= CHECK_AFTER - age);
        assert!(remaining > CHECK_AFTER - age - Duration::from_secs(60));

        // 成功标记没有对应快照时，即使同进程刚启动也要去取数据。
        let (_, retry) = check_when_due(&dir, None, || Err(()));
        assert!(retry.saturating_duration_since(Instant::now()) <= RETRY_AFTER);
        assert_eq!(fs::read(&marker).unwrap(), CHECK_FAILED);
        check_when_due(&dir, None, || panic!("首次离线后的重启应遵守失败退避"));
        fs::remove_file(marker).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn supporters_failed_refresh_keeps_the_previous_snapshot() {
        let (manifest, recent, wall) = fixture();
        let mut state = State::new(PathBuf::new());
        let (tx, rx) = mpsc::channel();
        tx.send(Event::Cached(manifest.snapshot(&recent, &wall).unwrap())).unwrap();
        tx.send(Event::Checked(None, Instant::now() + RETRY_AFTER)).unwrap();
        state.pending = Some(rx);
        state.refresh(&egui::Context::default());
        assert_eq!(state.snapshot().unwrap().version, manifest.version);
        assert!(state.pending.is_none());
        assert!(state.next_check.is_some_and(|next| next > Instant::now()));
    }
}
