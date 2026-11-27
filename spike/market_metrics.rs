//! 联网集市的统计与离线事件队列。

use super::{bounded, valid_id};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt::Write,
    io::Read,
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use url::Url;

const JOURNAL_LIMIT: u64 = 1024 * 1024;
const STATS_LIMIT: u64 = 256 * 1024;
const QUEUE_LIMIT: usize = 1024;
const BATCH_LIMIT: usize = 32;
const RETRY_INTERVAL: Duration = Duration::from_secs(30);
#[cfg(not(test))]
const METRICS_URL: &str = "https://daozai-market-metrics.xiaotang-daozai.workers.dev";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct Counts {
    pub(super) downloads: u64,
    pub(super) favorites: u64,
}

#[derive(Deserialize, Serialize)]
struct Stats {
    plugins: BTreeMap<String, Counts>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Event {
    event_id: String,
    plugin_id: String,
    #[serde(flatten)]
    action: Action,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Action {
    Download { version: String },
    Favorite { saved: bool },
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct Journal {
    installation_id: String,
    pending: VecDeque<Event>,
    favorites: BTreeSet<String>,
}

enum Response {
    Snapshot(Stats),
    Rejected,
    Unavailable,
}

pub(super) struct MarketMetrics {
    endpoint: Option<Url>,
    channel: &'static str,
    journal_path: PathBuf,
    journal: Journal,
    stats_path: PathBuf,
    stats: Option<Stats>,
    job: Option<(usize, JoinHandle<Response>)>,
    next_send: Instant,
    want_read: bool,
    batch_limit: usize,
}

impl MarketMetrics {
    #[cfg(test)]
    pub(super) fn for_test(dir: &Path, endpoint: Url) -> Self {
        Self::with_endpoint(dir, Some(endpoint), "preview", "preview-test")
    }

    pub(super) fn open(dir: &Path) -> Self {
        let channel = if option_env!("DAOZAI_TEST_BUILD").is_some() {
            "preview"
        } else {
            "stable"
        };
        #[cfg(not(test))]
        if channel == "preview" {
            if let Some(endpoint) = std::env::var("DAOZAI_MARKET_METRICS_TEST_ENDPOINT")
                .ok()
                .and_then(|address| Url::parse(&address).ok())
                .filter(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && (matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
                            || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback()))
                })
            {
                return Self::with_endpoint(dir, Some(endpoint), channel, "preview-test");
            }
        }
        #[cfg(not(test))]
        let endpoint = Some(Url::parse(METRICS_URL).expect("集市统计地址有效"));
        #[cfg(test)]
        let endpoint = None;
        Self::with_endpoint(dir, endpoint, channel, channel)
    }

    fn with_endpoint(
        dir: &Path,
        endpoint: Option<Url>,
        channel: &'static str,
        journal_name: &str,
    ) -> Self {
        let journal_path = dir.join(format!("market-metrics-{journal_name}.json"));
        let journal = bounded(&journal_path, JOURNAL_LIMIT)
            .ok()
            .and_then(|contents| serde_json::from_str::<Journal>(&contents).ok())
            .filter(|journal| {
                journal.pending.len() <= QUEUE_LIMIT
                    && valid_uuid(&journal.installation_id)
                    && journal
                        .pending
                        .iter()
                        .all(|event| valid_uuid(&event.event_id) && valid_id(&event.plugin_id))
                    && journal.favorites.iter().all(|id| valid_id(id))
            })
            .unwrap_or_default();
        let stats_path = dir.join(format!("market-stats-{journal_name}.json"));
        let stats = bounded(&stats_path, STATS_LIMIT)
            .ok()
            .and_then(|contents| serde_json::from_str(&contents).ok());
        Self {
            endpoint,
            channel,
            journal_path,
            journal,
            stats_path,
            stats,
            job: None,
            next_send: Instant::now(),
            want_read: false,
            batch_limit: BATCH_LIMIT,
        }
    }

    pub(super) fn counts(&self, id: &str) -> Option<Counts> {
        self.stats.as_ref()?.plugins.get(id).copied()
    }

    pub(super) fn busy(&self) -> bool {
        let now = Instant::now();
        self.endpoint.is_some()
            && (self.job.is_some()
                || (!self.journal.pending.is_empty() && now >= self.next_send)
                || self.want_read)
    }

    pub(super) fn repaint_delay(&self) -> Option<Duration> {
        self.endpoint.as_ref()?;
        if self.busy() {
            Some(Duration::from_millis(32))
        } else if !self.journal.pending.is_empty() {
            Some(self.next_send.saturating_duration_since(Instant::now()))
        } else {
            None
        }
    }

    pub(super) fn request_refresh(&mut self, force: bool) {
        self.want_read |= force || self.stats.is_none();
    }

    pub(super) fn record_download(&mut self, id: &str, version: String) {
        if self.endpoint.is_none() {
            return;
        }
        let mut journal = self.journal.clone();
        if journal.push(id, Action::Download { version }) {
            self.save(journal);
        }
    }

    pub(super) fn sync_favorites<'a>(
        &mut self,
        local: &BTreeMap<String, String>,
        official_ids: impl Iterator<Item = &'a str>,
    ) {
        if self.endpoint.is_none() {
            return;
        }
        let changes = self
            .journal
            .favorites
            .iter()
            .filter(|id| !local.contains_key(*id))
            .map(|id| (id.clone(), false))
            .chain(
                official_ids
                    .filter(|id| local.contains_key(*id) && !self.journal.favorites.contains(*id))
                    .map(|id| (id.to_owned(), true)),
            );
        let mut journal = None;
        for (id, saved) in changes {
            let journal = journal.get_or_insert_with(|| self.journal.clone());
            if !journal.push(&id, Action::Favorite { saved }) {
                break;
            }
            if saved {
                journal.favorites.insert(id);
            } else {
                journal.favorites.remove(&id);
            }
        }
        if let Some(journal) = journal {
            self.save(journal);
        }
    }

    fn save(&mut self, journal: Journal) -> bool {
        let Ok(contents) = serde_json::to_vec(&journal) else {
            return false;
        };
        if contents.len() as u64 > JOURNAL_LIMIT
            || crate::model::atomic_write(&self.journal_path, &contents).is_err()
        {
            return false;
        }
        self.journal = journal;
        true
    }

    // 返回队列是否腾出了空间，供尚未入队的本地收藏再次同步。
    pub(super) fn poll(&mut self) -> bool {
        let Some(endpoint) = self.endpoint.clone() else {
            return false;
        };
        let now = Instant::now();
        let mut advanced = false;
        if self.job.as_ref().is_some_and(|(_, job)| job.is_finished()) {
            let (size, job) = self.job.take().unwrap();
            let response = job.join().unwrap_or(Response::Unavailable);
            match response {
                Response::Snapshot(stats) => {
                    if let Ok(contents) = serde_json::to_vec(&stats) {
                        if contents.len() as u64 <= STATS_LIMIT {
                            let _ = crate::model::atomic_write(&self.stats_path, &contents);
                        }
                    }
                    self.stats = Some(stats);
                    self.want_read = false;
                    advanced = self.acknowledge(size);
                }
                Response::Rejected if size > 1 => self.batch_limit = 1,
                Response::Rejected if size == 1 => advanced = self.acknowledge(size),
                _ if size > 0 => self.next_send = now + RETRY_INTERVAL,
                _ => self.want_read = false,
            }
        }
        if self.job.is_some() {
            return advanced;
        }
        let size = if now >= self.next_send {
            self.journal.pending.len().min(self.batch_limit)
        } else {
            0
        };
        if size > 0 || self.want_read {
            let channel = self.channel;
            let installation_id = self.journal.installation_id.clone();
            let events: Vec<_> = self.journal.pending.iter().take(size).cloned().collect();
            self.job = Some((
                size,
                thread::spawn(move || fetch(endpoint, channel, &installation_id, &events)),
            ));
        }
        advanced
    }

    fn acknowledge(&mut self, size: usize) -> bool {
        if size == 0 {
            return false;
        }
        let mut journal = self.journal.clone();
        journal.pending.drain(..size);
        if self.save(journal) {
            self.batch_limit = BATCH_LIMIT;
            return true;
        }
        self.next_send = Instant::now() + RETRY_INTERVAL;
        false
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::{
        fs,
        io::{BufRead, BufReader, Write},
        net::{TcpListener, TcpStream},
    };

    const ID: &str = "example.payday";

    struct DataDir(PathBuf);

    impl DataDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("daozai-market-metrics-{}", uuid().unwrap()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for DataDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn mock_endpoint(listener: &TcpListener) -> Url {
        Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap()
    }

    pub(in crate::sdk_spike::local) fn next_request(
        socket: &mut TcpStream,
    ) -> (String, serde_json::Value) {
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(socket);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut size = 0;
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("Content-Length") {
                    size = value.trim().parse().unwrap();
                }
            }
        }
        let mut body = vec![0; size];
        reader.read_exact(&mut body).unwrap();
        (
            request_line,
            serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
        )
    }

    fn wait_for_job(metrics: &mut MarketMetrics) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while metrics.busy() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
            metrics.poll();
        }
    }

    #[test]
    fn download_retries_with_same_event_after_restart() {
        let dir = DataDir::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = mock_endpoint(&listener);
        let server = thread::spawn(move || {
            let mut first = None;
            for retry in [false, true] {
                let (mut socket, _) = listener.accept().unwrap();
                let (request, body) = next_request(&mut socket);
                assert_eq!(request, "POST /v1/events HTTP/1.1\r\n");
                assert_eq!(body["channel"], "preview");
                assert!(valid_uuid(body["installation_id"].as_str().unwrap()));
                assert_eq!(body["events"][0]["plugin_id"], ID);
                assert_eq!(body["events"][0]["kind"], "download");
                assert_eq!(body["events"][0]["version"], "1.0.1");
                assert!(valid_uuid(body["events"][0]["event_id"].as_str().unwrap()));
                if retry {
                    assert_eq!(first.as_ref(), Some(&body));
                    let body = r#"{"plugins":{"example.payday":{"downloads":1,"favorites":0}}}"#;
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                } else {
                    first = Some(body);
                    socket
                        .write_all(
                            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                }
            }
        });
        let mut metrics =
            MarketMetrics::with_endpoint(&dir.0, Some(endpoint.clone()), "preview", "preview");
        metrics.record_download(ID, "1.0.1".into());
        metrics.poll();
        wait_for_job(&mut metrics);
        assert_eq!(metrics.journal.pending.len(), 1);
        assert_eq!(metrics.counts(ID), None);
        drop(metrics);

        let mut metrics =
            MarketMetrics::with_endpoint(&dir.0, Some(endpoint), "preview", "preview");
        metrics.poll();
        wait_for_job(&mut metrics);
        assert_eq!(
            metrics.counts(ID),
            Some(Counts {
                downloads: 1,
                favorites: 0,
            })
        );
        assert_eq!(metrics.counts("absent.plugin"), None);
        assert!(metrics.journal.pending.is_empty());
        let stable = MarketMetrics::with_endpoint(&dir.0, None, "stable", "stable");
        assert!(stable.journal.installation_id.is_empty());
        server.join().unwrap();
    }

    #[test]
    fn imports_existing_official_favorites_once_and_queues_removal_after_restart() {
        let dir = DataDir::new();
        let endpoint = Url::parse("http://127.0.0.1").unwrap();
        let mut metrics =
            MarketMetrics::with_endpoint(&dir.0, Some(endpoint.clone()), "preview", "preview");
        let local = BTreeMap::from([
            (ID.into(), "发薪倒计时".into()),
            ("dev.local".into(), "本地插件".into()),
        ]);
        metrics.sync_favorites(&local, [ID].into_iter());
        drop(metrics);

        let mut metrics =
            MarketMetrics::with_endpoint(&dir.0, Some(endpoint), "preview", "preview");
        metrics.sync_favorites(&local, [ID].into_iter());
        assert_eq!(metrics.journal.pending.len(), 1);
        metrics.sync_favorites(&BTreeMap::new(), std::iter::empty());
        metrics.sync_favorites(&BTreeMap::new(), std::iter::empty());
        assert_eq!(metrics.journal.pending.len(), 2);
        assert!(metrics
            .journal
            .pending
            .iter()
            .all(|event| event.plugin_id == ID));
        assert!(matches!(
            metrics.journal.pending[0].action,
            Action::Favorite { saved: true }
        ));
        assert!(matches!(
            metrics.journal.pending[1].action,
            Action::Favorite { saved: false }
        ));
    }
}

impl Journal {
    fn push(&mut self, id: &str, action: Action) -> bool {
        if self.pending.len() >= QUEUE_LIMIT {
            return false;
        }
        if self.installation_id.is_empty() {
            let Some(installation_id) = uuid() else {
                return false;
            };
            self.installation_id = installation_id;
        }
        let Some(event_id) = uuid() else {
            return false;
        };
        self.pending.push_back(Event {
            event_id,
            plugin_id: id.to_owned(),
            action,
        });
        true
    }
}

fn uuid() -> Option<String> {
    let mut bytes = [0u8; 16];
    SystemRandom::new().fill(&mut bytes).ok()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut id = String::with_capacity(36);
    for (i, byte) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            id.push('-');
        }
        write!(id, "{byte:02x}").ok()?;
    }
    Some(id)
}

fn valid_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, byte)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn fetch(endpoint: Url, channel: &str, installation_id: &str, events: &[Event]) -> Response {
    #[derive(Serialize)]
    struct Batch<'a> {
        channel: &'a str,
        installation_id: &'a str,
        events: &'a [Event],
    }
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(endpoint.scheme() == "https")
        .redirects(0)
        .timeout_connect(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .build();
    let response = if events.is_empty() {
        let Ok(mut url) = endpoint.join("/v1/stats") else {
            return Response::Unavailable;
        };
        url.query_pairs_mut().append_pair("channel", channel);
        agent.get(url.as_str()).call()
    } else {
        let Ok(url) = endpoint.join("/v1/events") else {
            return Response::Unavailable;
        };
        let Ok(body) = serde_json::to_string(&Batch {
            channel,
            installation_id,
            events,
        }) else {
            return Response::Unavailable;
        };
        agent
            .post(url.as_str())
            .set("Content-Type", "application/json")
            .send_string(&body)
    };
    match response {
        Ok(response) => {
            let mut body = Vec::new();
            if response
                .into_reader()
                .take(STATS_LIMIT + 1)
                .read_to_end(&mut body)
                .is_ok()
                && body.len() as u64 <= STATS_LIMIT
            {
                if let Ok(stats) = serde_json::from_slice(&body) {
                    return Response::Snapshot(stats);
                }
            }
            Response::Unavailable
        }
        Err(ureq::Error::Status(422, _)) => Response::Rejected,
        Err(_) => Response::Unavailable,
    }
}
