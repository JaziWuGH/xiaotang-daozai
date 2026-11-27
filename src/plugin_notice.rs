use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    fs,
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const VERSION: u8 = 1;
pub(crate) const MAX_SOURCES: usize = 16;
const MAX_FRAME: usize = 128 * 1024;
const MAX_MARKDOWN: usize = 32 * 1024;
const MAX_EVENTS: usize = 64;
const MAX_SOURCE_EVENTS: usize = MAX_EVENTS / MAX_SOURCES;
const MAX_SOURCE_RECENT: usize = 32;
const MAX_OPENS: usize = 16;
const NOTICE_WINDOW: Duration = Duration::from_secs(1);
const STATUS_INTERVAL: Duration = Duration::from_millis(250);
const UPDATE_DELAY: Duration = Duration::from_millis(50);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Source {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Completion {
    pub source: String,
    pub name: String,
    pub id: String,
    pub session: Option<String>,
    pub markdown: String,
    pub title: Option<String>,
    pub action_label: Option<String>,
}

#[derive(Debug)]
pub(crate) enum Event {
    Completed(Completion),
    StatusChanged {
        source: String,
        card: Option<Completion>,
    },
    Opened {
        source: String,
        id: String,
        ok: bool,
    },
}

impl Event {
    fn source(&self) -> &str {
        match self {
            Self::Completed(completion) => &completion.source,
            Self::Opened { source, .. } | Self::StatusChanged { source, .. } => source,
        }
    }
}

#[derive(Deserialize)]
struct Request {
    version: u8,
    source: String,
    token: String,
    #[serde(flatten)]
    action: Action,
}

#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
enum Action {
    Status,
    Complete {
        id: String,
        session: Option<String>,
        markdown: Option<String>,
    },
    Notify {
        #[serde(flatten)]
        card: CardInput,
    },
    StatusUpdate {
        #[serde(flatten)]
        card: CardInput,
        ttl_seconds: u64,
    },
    StatusClear {
        id: String,
    },
    PollOpen,
    AckOpen {
        request_id: String,
        opened: bool,
    },
}

#[derive(Deserialize)]
struct CardInput {
    id: String,
    title: String,
    markdown: Option<String>,
    session: Option<String>,
    action_label: Option<String>,
}

impl CardInput {
    fn into_card(self, source: &str, name: String) -> Option<Completion> {
        if !valid_text(&self.id, 128)
            || !valid_text(&self.title, 120)
            || self
                .markdown
                .as_ref()
                .is_some_and(|text| text.len() > MAX_MARKDOWN)
            || self
                .session
                .as_ref()
                .is_some_and(|session| !valid_text(session, 512))
            || self.action_label.as_ref().is_some_and(|label| {
                self.session.is_none() || label.chars().count() > 6 || !valid_text(label, 24)
            })
        {
            return None;
        }
        Some(Completion {
            source: source.to_owned(),
            name,
            id: self.id,
            title: Some(self.title),
            markdown: self.markdown.unwrap_or_default(),
            session: self.session,
            action_label: self.action_label,
        })
    }
}

struct Authorized {
    name: String,
    token: String,
    notice_times: VecDeque<Instant>,
    next_status: Option<Instant>,
}

struct LiveStatus {
    key: String,
    card: Completion,
    expires: Instant,
}

struct Received {
    source: String,
    id: String,
    session: Option<String>,
}

struct PendingOpen {
    source: String,
    id: String,
    session: String,
    request_id: String,
    expires: Instant,
    delivered: bool,
}

#[derive(Default)]
struct State {
    sources: HashMap<String, Authorized>,
    events: VecDeque<Event>,
    recent: VecDeque<Received>,
    opens: VecDeque<PendingOpen>,
    statuses: HashMap<String, LiveStatus>,
    dirty_statuses: BTreeSet<String>,
    status_serial: u64,
    preview: bool,
    muted: bool,
}

impl State {
    fn has_room(&self, source: &str) -> bool {
        self.events.len() + self.opens.len() < MAX_EVENTS
            && self
                .events
                .iter()
                .filter(|event| event.source() == source)
                .count()
                + self
                    .opens
                    .iter()
                    .filter(|open| open.source == source)
                    .count()
                < MAX_SOURCE_EVENTS
    }

    fn pending(&self) -> bool {
        !self.events.is_empty() || !self.dirty_statuses.is_empty()
    }

    fn queue_notice(&mut self, mut card: Completion, now: Instant) -> Result<(), &'static str> {
        let matches_source = |entry: &Received| entry.source == card.source;
        if self
            .recent
            .iter()
            .any(|entry| matches_source(entry) && entry.id == card.id)
        {
            return Ok(());
        }
        if !self.has_room(&card.source) {
            return Err("busy");
        }
        let times = &mut self.sources.get_mut(&card.source).unwrap().notice_times;
        times.retain(|sent| now.saturating_duration_since(*sent) < NOTICE_WINDOW);
        if times.len() >= MAX_SOURCE_EVENTS {
            return Err("busy");
        }
        times.push_back(now);
        if self
            .recent
            .iter()
            .filter(|entry| matches_source(entry))
            .count()
            >= MAX_SOURCE_RECENT
        {
            if let Some(first) = self.recent.iter().position(matches_source) {
                self.recent.remove(first);
            }
        }
        self.recent.push_back(Received {
            source: card.source.clone(),
            id: card.id.clone(),
            session: card.session.clone(),
        });
        if !self.preview {
            redact(&mut card);
        }
        self.events.push_back(Event::Completed(card));
        Ok(())
    }

    fn set_status(
        &mut self,
        mut card: Completion,
        now: Instant,
        ttl: Duration,
    ) -> Result<(), &'static str> {
        let allowed = &mut self.sources.get_mut(&card.source).unwrap().next_status;
        if allowed.is_some_and(|next| now < next) {
            return Err("busy");
        }
        *allowed = Some(now + STATUS_INTERVAL);
        if !self.preview {
            redact(&mut card);
        }
        let key = std::mem::take(&mut card.id);
        let expires = now + ttl;
        let source = card.source.clone();
        let current = self.statuses.get(&source);
        if let Some(current) =
            current.filter(|status| status.key == key && status.card.session == card.session)
        {
            card.id.clone_from(&current.card.id);
            if current.card != card || expires < current.expires {
                self.dirty_statuses.insert(source.clone());
            }
        } else {
            if let Some(current) = current {
                self.opens
                    .retain(|open| open.source != source || open.id != current.card.id);
            }
            // 已撤下的卡片再次出现时，旧按钮和回执不能影响新卡片。
            self.status_serial += 1;
            card.id = format!("\0{}", self.status_serial);
            self.dirty_statuses.insert(source.clone());
        }
        self.statuses
            .insert(source, LiveStatus { key, card, expires });
        Ok(())
    }

    fn clear_status(&mut self, source: &str) {
        if let Some(status) = self.statuses.remove(source) {
            self.opens
                .retain(|open| open.source != source || open.id != status.card.id);
            self.dirty_statuses.insert(source.to_owned());
        }
    }

    fn resolve_open(&mut self, index: usize, ok: bool) {
        if let Some(open) = self.opens.remove(index) {
            self.events.push_back(Event::Opened {
                source: open.source,
                id: open.id,
                ok,
            });
        }
    }

    fn expire(&mut self, now: Instant) {
        let expired: Vec<_> = self
            .statuses
            .iter()
            .filter(|(_, status)| status.expires <= now)
            .map(|(source, _)| source.clone())
            .collect();
        for source in expired {
            self.clear_status(&source);
        }
        for index in (0..self.opens.len()).rev() {
            if self.opens[index].expires <= now {
                self.resolve_open(index, false);
            }
        }
    }

    fn handle(&mut self, req: Request, now: Instant) -> Value {
        if req.version != VERSION {
            return failure("unsupported_version");
        }
        let Some(source) = self.sources.get(&req.source) else {
            return failure("unauthorized");
        };
        if !same_token(&source.token, &req.token) {
            return failure("unauthorized");
        }
        let preview = self.preview;
        let name = source.name.clone();
        if self.muted {
            return match req.action {
                Action::Status | Action::Complete { .. } => json!({"ok": true, "preview": false}),
                Action::PollOpen => json!({"ok": true, "preview": false, "open": null}),
                _ => failure("disabled"),
            };
        }
        self.expire(now);
        match req.action {
            Action::Status => {}
            Action::Complete {
                id,
                session,
                markdown,
            } => {
                if !valid_text(&id, 128)
                    || session
                        .as_ref()
                        .is_some_and(|value| !valid_text(value, 512))
                    || markdown
                        .as_ref()
                        .is_some_and(|value| value.len() > MAX_MARKDOWN)
                {
                    return failure("invalid_completion");
                }
                let card = Completion {
                    source: req.source,
                    name,
                    id,
                    session,
                    markdown: markdown.unwrap_or_default(),
                    title: None,
                    action_label: None,
                };
                if let Err(error) = self.queue_notice(card, now) {
                    return failure(error);
                }
            }
            Action::Notify { card } => {
                let Some(card) = card.into_card(&req.source, name) else {
                    return failure("invalid_card");
                };
                if let Err(error) = self.queue_notice(card, now) {
                    return failure(error);
                }
            }
            Action::StatusUpdate { card, ttl_seconds } => {
                let Some(card) = card.into_card(&req.source, name) else {
                    return failure("invalid_card");
                };
                if !(2..=120).contains(&ttl_seconds) {
                    return failure("invalid_ttl");
                }
                if let Err(error) = self.set_status(card, now, Duration::from_secs(ttl_seconds)) {
                    return failure(error);
                }
            }
            Action::StatusClear { id } => {
                if !valid_text(&id, 128) {
                    return failure("invalid_card");
                }
                if self
                    .statuses
                    .get(&req.source)
                    .is_some_and(|status| status.key == id)
                {
                    self.clear_status(&req.source);
                }
            }
            Action::PollOpen => {
                let open = self
                    .opens
                    .iter_mut()
                    .find(|open| open.source == req.source && !open.delivered)
                    .map(|open| {
                        open.delivered = true;
                        json!({"request_id": open.request_id, "session": open.session})
                    });
                return json!({"ok": true, "preview": preview, "open": open});
            }
            Action::AckOpen { request_id, opened } => {
                let Some(index) = self.opens.iter().position(|open| {
                    open.source == req.source && open.request_id == request_id && open.delivered
                }) else {
                    return failure("unknown_open");
                };
                self.resolve_open(index, opened);
            }
        }
        json!({"ok": true, "preview": preview})
    }
}

pub(crate) struct Bridge {
    dir: PathBuf,
    sources: Vec<Source>,
    state: Arc<Mutex<State>>,
    ctx: egui::Context,
    listener: Option<Listener>,
}

impl Bridge {
    pub(crate) fn start(dir: &Path, ctx: egui::Context) -> Result<Self, String> {
        let dir = dir.join("plugin-notices");
        let sources = match fs::read(dir.join("sources.json")) {
            Ok(bytes) => serde_json::from_slice::<Vec<Source>>(&bytes)
                .map_err(|_| "插件通知授权配置无法读取".to_owned())?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(format!("插件通知授权配置无法读取：{error}")),
        };
        if sources.len() > MAX_SOURCES
            || sources.iter().enumerate().any(|(index, source)| {
                !valid_source(&source.id, &source.name)
                    || sources[..index].iter().any(|other| other.id == source.id)
            })
        {
            return Err("插件通知授权配置无效".into());
        }
        let mut bridge = Self {
            dir,
            sources: Vec::new(),
            state: Arc::new(Mutex::new(State::default())),
            ctx,
            listener: None,
        };
        if !sources.is_empty() {
            let listener = Listener::start(bridge.state.clone(), bridge.ctx.clone())?;
            for source in sources {
                let token = random_token()?;
                bridge.write_connection(&source.id, &token, listener.addr.port())?;
                bridge.state.lock().unwrap().sources.insert(
                    source.id.clone(),
                    Authorized {
                        name: source.name.clone(),
                        token,
                        notice_times: VecDeque::new(),
                        next_status: None,
                    },
                );
                bridge.sources.push(source);
            }
            bridge.listener = Some(listener);
        }
        Ok(bridge)
    }

    pub(crate) fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub(crate) fn connection_file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub(crate) fn authorize(&mut self, id: &str, name: &str) -> Result<(), String> {
        let id = id.trim();
        let name = name.trim();
        if !valid_source(id, name) {
            return Err("请输入有效的插件来源 ID 和名称".into());
        }
        if self.sources.iter().any(|source| source.id == id) {
            return Err("此来源已经授权".into());
        }
        if self.sources.len() == MAX_SOURCES {
            return Err("插件通知来源已达上限".into());
        }
        let listener = if self.listener.is_none() {
            Some(Listener::start(self.state.clone(), self.ctx.clone())?)
        } else {
            None
        };
        let port = listener
            .as_ref()
            .or(self.listener.as_ref())
            .unwrap()
            .addr
            .port();
        let token = random_token()?;
        fs::create_dir_all(&self.dir).map_err(|error| format!("无法创建插件通知目录：{error}"))?;
        let mut sources = self.sources.clone();
        sources.push(Source {
            id: id.to_owned(),
            name: name.to_owned(),
        });
        let mut state = self.state.lock().unwrap();
        self.write_connection(id, &token, port)?;
        if let Err(error) = self.write_sources(&sources) {
            let _ = fs::remove_file(self.connection_file(id));
            return Err(error);
        }
        state.sources.insert(
            id.to_owned(),
            Authorized {
                name: name.to_owned(),
                token,
                notice_times: VecDeque::new(),
                next_status: None,
            },
        );
        drop(state);
        self.sources = sources;
        if listener.is_some() {
            self.listener = listener;
        }
        Ok(())
    }

    pub(crate) fn revoke(&mut self, id: &str) -> Result<(), String> {
        let mut sources = self.sources.clone();
        sources.retain(|source| source.id != id);
        if sources.len() == self.sources.len() {
            return Ok(());
        }
        let mut state = self.state.lock().unwrap();
        self.write_sources(&sources)?;
        state.sources.remove(id);
        state.events.retain(|event| event.source() != id);
        state.recent.retain(|item| item.source != id);
        state.opens.retain(|open| open.source != id);
        state.statuses.remove(id);
        state.dirty_statuses.remove(id);
        drop(state);
        self.sources = sources;
        let _ = fs::remove_file(self.connection_file(id));
        if self.sources.is_empty() {
            self.listener = None;
        }
        Ok(())
    }

    pub(crate) fn display(&self, enabled: bool, preview: bool) {
        let mut state = self.state.lock().unwrap();
        if !enabled && !state.muted {
            state.events.clear();
            state.recent.clear();
            state.opens.clear();
            state.statuses.clear();
            state.dirty_statuses.clear();
        }
        let preview = enabled && preview;
        if state.preview && !preview {
            for event in &mut state.events {
                if let Event::Completed(completion) = event {
                    redact(completion);
                }
            }
            for status in state.statuses.values_mut() {
                redact(&mut status.card);
            }
        }
        state.preview = preview;
        state.muted = !enabled;
    }

    pub(crate) fn drain(&self) -> Vec<Event> {
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        state.expire(now);
        let mut events: Vec<_> = state.events.drain(..).collect();
        for source in std::mem::take(&mut state.dirty_statuses) {
            let card = state
                .statuses
                .get(&source)
                .map(|status| status.card.clone());
            events.push(Event::StatusChanged { source, card });
        }
        let expires = state.statuses.values().map(|status| status.expires).min();
        drop(state);
        if let Some(expires) = expires {
            self.ctx
                .request_repaint_after(expires.saturating_duration_since(now));
        }
        events
    }

    pub(crate) fn open_session(&self, source: &str, id: &str, session: &str) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if state.muted || !state.sources.contains_key(source) {
            return Err("插件来源未授权".into());
        }
        let now = Instant::now();
        state.expire(now);
        let status = state.statuses.get(source).is_some_and(|status| {
            status.card.id == id && status.card.session.as_deref() == Some(session)
        });
        if !status
            && !state.recent.iter().any(|item| {
                item.source == source && item.id == id && item.session.as_deref() == Some(session)
            })
        {
            return Err("这条通知的会话已无法打开".into());
        }
        if state
            .opens
            .iter()
            .any(|open| open.source == source && open.id == id)
        {
            return Ok(());
        }
        if state.opens.len() == MAX_OPENS
            || state.opens.iter().any(|open| open.source == source)
            || !state.has_room(source)
        {
            return Err("会话跳转队列已满，请稍后重试".into());
        }
        state.opens.push_back(PendingOpen {
            source: source.to_owned(),
            id: id.to_owned(),
            session: session.to_owned(),
            request_id: random_token()?,
            expires: now + OPEN_TIMEOUT,
            delivered: false,
        });
        drop(state);
        self.ctx.request_repaint_after(OPEN_TIMEOUT);
        Ok(())
    }

    fn write_connection(&self, id: &str, token: &str, port: u16) -> Result<(), String> {
        let descriptor = json!({
            "version": VERSION, "host": "127.0.0.1", "port": port,
            "source": id, "token": token
        });
        write_json(&self.connection_file(id), &descriptor)
    }

    fn write_sources(&self, sources: &[Source]) -> Result<(), String> {
        write_json(&self.dir.join("sources.json"), &sources)
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.listener = None;
        let mut state = self.state.lock().unwrap();
        for id in state.sources.keys() {
            let _ = fs::remove_file(self.connection_file(id));
        }
        *state = State::default();
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    crate::model::atomic_write(path, &bytes)
        .map_err(|error| format!("无法保存插件通知授权：{error}"))
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn redact(card: &mut Completion) {
    card.markdown.clear();
    if let Some(title) = &mut card.title {
        title.clear();
    }
}

fn valid_source(id: &str, name: &str) -> bool {
    let first = id.split('.').next().unwrap_or_default();
    let reserved = matches!(first, "con" | "prn" | "aux" | "nul")
        || first
            .strip_prefix("com")
            .or_else(|| first.strip_prefix("lpt"))
            .is_some_and(|number| matches!(number.as_bytes(), [b'0'..=b'9']));
    !reserved
        && id != "sources"
        && id.len() <= 64
        && id.split('.').all(|part| {
            part.as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
        && valid_text(name, 96)
}

fn random_token() -> Result<String, String> {
    use windows::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_ALG_HANDLE, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };

    let mut bytes = [0u8; 32];
    // 系统首选随机源不需要预先打开算法句柄。
    let status = unsafe {
        BCryptGenRandom(
            BCRYPT_ALG_HANDLE::default(),
            &mut bytes,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status.0 < 0 {
        return Err("无法生成插件通知连接凭据".into());
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn same_token(expected: &str, provided: &str) -> bool {
    expected.len() == provided.len()
        && expected
            .bytes()
            .zip(provided.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

fn failure(code: &'static str) -> Value {
    json!({"ok": false, "error": code})
}

struct Listener {
    addr: SocketAddrV4,
    stop: Arc<AtomicBool>,
}

impl Listener {
    fn start(state: Arc<Mutex<State>>, ctx: egui::Context) -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("无法启动本机插件通知连接：{error}"))?;
        let addr = SocketAddrV4::new(
            Ipv4Addr::LOCALHOST,
            listener
                .local_addr()
                .map_err(|error| error.to_string())?
                .port(),
        );
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        thread::Builder::new()
            .name("daozai-plugin-notice".into())
            .spawn(move || {
                while let Ok((stream, _)) = listener.accept() {
                    if stopping.load(Ordering::Relaxed) {
                        break;
                    }
                    let _ = serve(stream, &state, &ctx);
                    if stopping.load(Ordering::Relaxed) {
                        break;
                    }
                }
            })
            .map_err(|error| format!("无法启动插件通知线程：{error}"))?;
        Ok(Self { addr, stop })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&self.addr.into(), Duration::from_millis(50));
    }
}

fn serve(mut stream: TcpStream, state: &Mutex<State>, ctx: &egui::Context) -> io::Result<()> {
    let frame = read_frame(&mut stream)?;
    let response = match serde_json::from_slice(&frame) {
        Ok(request) => {
            let mut state = state.lock().unwrap();
            let pending = state.pending();
            let response = state.handle(request, Instant::now());
            let repaint = !pending && state.pending();
            drop(state);
            if repaint {
                ctx.request_repaint_after(UPDATE_DELAY);
            }
            response
        }
        Err(_) => failure("invalid_request"),
    };
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
    let mut encoded = serde_json::to_vec(&response)?;
    encoded.push(b'\n');
    stream.write_all(&encoded)
}

fn read_frame(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let mut frame = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(io::ErrorKind::TimedOut.into());
        };
        stream.set_read_timeout(Some(remaining.max(Duration::from_millis(1))))?;
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let chunk = &buffer[..count];
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let content = &chunk[..newline.unwrap_or(chunk.len())];
        if frame.len() + content.len() > MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "插件通知请求过大",
            ));
        }
        frame.extend_from_slice(content);
        if newline.is_some() {
            return Ok(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    mod isolation {
        include!("plugin_notice_isolation_tests.rs");
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("daozai-plugin-notice-{}", random_token().unwrap())),
            )
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn descriptor(bridge: &Bridge, id: &str) -> Value {
        serde_json::from_slice(&fs::read(bridge.connection_file(id)).unwrap()).unwrap()
    }

    fn exchange(connection: &Value, mut body: Value) -> Value {
        for name in ["version", "source", "token"] {
            if body.get(name).is_none() {
                body[name] = connection[name].clone();
            }
        }
        let mut stream = TcpStream::connect((
            Ipv4Addr::LOCALHOST,
            connection["port"].as_u64().unwrap() as u16,
        ))
        .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut frame = serde_json::to_vec(&body).unwrap();
        frame.push(b'\n');
        stream.write_all(&frame).unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn request_at(state: &mut State, connection: &Value, mut body: Value, now: Instant) -> Value {
        for name in ["version", "source", "token"] {
            body[name] = connection[name].clone();
        }
        state.handle(serde_json::from_value(body).unwrap(), now)
    }

    #[test]
    fn busy_source_cannot_fill_the_shared_queue_or_evict_another_sources_jump() {
        let dir = TestDir::new();
        let mut bridge = Bridge::start(&dir.0, egui::Context::default()).unwrap();
        bridge.authorize("noisy", "高频来源").unwrap();
        bridge.authorize("other", "其他来源").unwrap();
        let noisy = descriptor(&bridge, "noisy");
        let other = descriptor(&bridge, "other");
        {
            let mut state = bridge.state.lock().unwrap();
            let now = Instant::now();
            for id in 0..MAX_SOURCE_EVENTS {
                assert_eq!(
                    request_at(
                        &mut state,
                        &noisy,
                        json!({"method":"notify","id":id.to_string(),"title":"更新"}),
                        now
                    )["ok"],
                    true
                );
            }
            assert_eq!(
                request_at(
                    &mut state,
                    &noisy,
                    json!({"method":"notify","id":"0","title":"更新"}),
                    now
                )["ok"],
                true
            );
            assert_eq!(
                request_at(
                    &mut state,
                    &noisy,
                    json!({"method":"notify","id":"retry","title":"更新"}),
                    now
                )["error"],
                "busy"
            );
            assert_eq!(
                request_at(
                    &mut state,
                    &other,
                    json!({"method":"complete","id":"turn","session":"session"}),
                    now
                )["ok"],
                true
            );
            state.events.clear();
            assert_eq!(
                request_at(
                    &mut state,
                    &noisy,
                    json!({"method":"notify","id":"retry","title":"更新"}),
                    now
                )["error"],
                "busy"
            );
            assert_eq!(
                request_at(
                    &mut state,
                    &noisy,
                    json!({"method":"notify","id":"retry","title":"更新"}),
                    now + NOTICE_WINDOW
                )["ok"],
                true
            );
            for id in 0..(MAX_SOURCE_RECENT * 2) {
                state.events.clear();
                assert_eq!(
                    request_at(
                        &mut state,
                        &noisy,
                        json!({"method":"notify","id":format!("later-{id}"),"title":"更新"}),
                        now + Duration::from_secs(2 + id as u64)
                    )["ok"],
                    true
                );
            }
        }
        bridge.open_session("other", "turn", "session").unwrap();
        assert_eq!(
            exchange(&other, json!({"method":"poll_open"}))["open"]["session"],
            "session"
        );
    }

    #[test]
    fn status_updates_coalesce_expire_and_invalidate_old_actions() {
        let dir = TestDir::new();
        let mut bridge = Bridge::start(&dir.0, egui::Context::default()).unwrap();
        bridge.authorize("printer", "打印机").unwrap();
        let printer = descriptor(&bridge, "printer");
        bridge.display(true, true);
        let status = |id, title, ttl_seconds| {
            json!({"method":"status_update","id":id,"title":title,
            "session":format!("job-{id}"),"action_label":"查看进度","ttl_seconds":ttl_seconds})
        };
        let now = Instant::now();
        {
            let mut state = bridge.state.lock().unwrap();
            assert_eq!(
                request_at(&mut state, &printer, status("one", "打印 42%", 1), now)["error"],
                "invalid_ttl"
            );
            assert_eq!(
                request_at(&mut state, &printer, status("one", "打印 42%", 20), now)["ok"],
                true
            );
            assert_eq!(
                request_at(&mut state, &printer, status("one", "打印 43%", 20), now)["error"],
                "busy"
            );
            assert_eq!(
                request_at(
                    &mut state,
                    &printer,
                    status("one", "打印 44%", 20),
                    now + STATUS_INTERVAL
                )["ok"],
                true
            );
        }
        let events = bridge.drain();
        let [Event::StatusChanged {
            card: Some(first), ..
        }] = events.as_slice()
        else {
            panic!("{events:?}");
        };
        assert_eq!(first.title.as_deref(), Some("打印 44%"));
        bridge
            .open_session("printer", &first.id, "job-one")
            .unwrap();
        let pending = exchange(&printer, json!({"method":"poll_open"}));
        {
            let mut state = bridge.state.lock().unwrap();
            assert_eq!(
                request_at(
                    &mut state,
                    &printer,
                    status("two", "新任务", 20),
                    now + STATUS_INTERVAL * 2
                )["ok"],
                true
            );
        }
        assert_eq!(
            exchange(&printer, json!({"method":"status_clear","id":"one"}))["ok"],
            true
        );
        assert_eq!(
            exchange(
                &printer,
                json!({"method":"ack_open",
            "request_id":pending["open"]["request_id"],"opened":true})
            )["error"],
            "unknown_open"
        );
        assert!(bridge
            .open_session("printer", &first.id, "job-one")
            .is_err());
        let events = bridge.drain();
        assert!(
            matches!(&events[..], [Event::StatusChanged { card: Some(card), .. }]
            if card.title.as_deref() == Some("新任务"))
        );
        bridge
            .state
            .lock()
            .unwrap()
            .statuses
            .get_mut("printer")
            .unwrap()
            .expires = Instant::now();
        assert!(
            matches!(&bridge.drain()[..], [Event::StatusChanged { source, card: None }] if source == "printer")
        );
        assert!(bridge.drain().is_empty());
    }

    #[test]
    fn disabled_display_drops_work_until_enabled_and_redacts_queued_content() {
        let dir = TestDir::new();
        let mut bridge = Bridge::start(&dir.0, egui::Context::default()).unwrap();
        bridge.authorize("source", "来源").unwrap();
        let source = descriptor(&bridge, "source");
        bridge.display(true, true);
        let notice = json!({"method":"notify","id":"new","title":"私密标题","markdown":"正文","session":"thread"});
        assert_eq!(exchange(&source, notice.clone())["ok"], true);
        bridge.display(true, false);
        let events = bridge.drain();
        assert!(matches!(&events[..], [Event::Completed(card)]
            if card.title.as_deref() == Some("") && card.markdown.is_empty()));
        bridge.open_session("source", "new", "thread").unwrap();
        bridge.display(false, false);
        assert_eq!(exchange(&source, notice.clone())["error"], "disabled");
        assert_eq!(
            exchange(&source, json!({"method":"poll_open"}))["open"],
            Value::Null
        );
        assert!(bridge.drain().is_empty());
        bridge.display(true, true);
        assert_eq!(exchange(&source, notice)["ok"], true);
        assert!(
            matches!(&bridge.drain()[..], [Event::Completed(card)] if card.title.as_deref() == Some("私密标题"))
        );
    }

    #[test]
    fn notice_transport_authorizes_redacts_and_deduplicates() {
        let dir = TestDir::new();
        let mut bridge = Bridge::start(&dir.0, egui::Context::default()).unwrap();
        assert!(bridge.listener.is_none());
        assert!(bridge.authorize("con.json", "错误 ID").is_err());
        bridge.authorize("codex", "Codex").unwrap();
        bridge.authorize("dsh", "DeepSeek Harness").unwrap();
        let codex = descriptor(&bridge, "codex");
        let dsh = descriptor(&bridge, "dsh");
        assert_ne!(codex["token"], dsh["token"]);
        assert_eq!(
            exchange(&codex, json!({"method": "status", "source": "dsh"}))["error"],
            "unauthorized"
        );
        assert_eq!(
            exchange(&codex, json!({"method": "status", "version": 2}))["error"],
            "unsupported_version"
        );

        let completion = json!({"method": "complete", "id": "turn-1", "session": "thread-2", "markdown": "正文"});
        assert_eq!(
            exchange(&codex, completion.clone()),
            json!({"ok": true, "preview": false})
        );
        assert_eq!(exchange(&codex, completion)["ok"], true);
        let events = bridge.drain();
        let [Event::Completed(hidden)] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert_eq!(hidden.session.as_deref(), Some("thread-2"));
        assert!(hidden.markdown.is_empty());

        bridge.display(true, true);
        let completion = json!({"method": "complete", "id": "turn-2", "markdown": "# 正文"});
        assert_eq!(exchange(&codex, completion)["preview"], true);
        let events = bridge.drain();
        let [Event::Completed(shown)] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert_eq!(shown.markdown, "# 正文");
        exchange(
            &codex,
            json!({"method": "complete", "id": "turn-3", "markdown": "忽略"}),
        );
        bridge.display(true, false);
        let events = bridge.drain();
        let [Event::Completed(scrubbed)] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert!(scrubbed.markdown.is_empty());
    }

    #[test]
    fn notice_transport_jump_uses_live_ack_and_allows_retry_after_timeout() {
        let dir = TestDir::new();
        let mut bridge = Bridge::start(&dir.0, egui::Context::default()).unwrap();
        bridge.authorize("dsh", "DeepSeek Harness").unwrap();
        let dsh = descriptor(&bridge, "dsh");
        exchange(
            &dsh,
            json!({"method": "complete", "id": "turn", "session": "session"}),
        );
        bridge.drain();
        assert!(bridge.open_session("dsh", "turn", "other-session").is_err());
        bridge.open_session("dsh", "turn", "session").unwrap();
        let first = exchange(&dsh, json!({"method": "poll_open"}));
        assert_eq!(first["open"]["session"], "session");
        assert_eq!(
            exchange(&dsh, json!({"method": "poll_open"}))["open"],
            Value::Null
        );
        bridge.state.lock().unwrap().opens[0].expires = Instant::now();
        let late_ack = json!({"method": "ack_open", "request_id": first["open"]["request_id"], "opened": true});
        assert_eq!(exchange(&dsh, late_ack)["error"], "unknown_open");
        let events = bridge.drain();
        assert!(matches!(&events[..], [Event::Opened { ok: false, .. }]));

        bridge.open_session("dsh", "turn", "session").unwrap();
        let next = exchange(&dsh, json!({"method": "poll_open"}));
        assert_ne!(next["open"]["request_id"], first["open"]["request_id"]);
        let ack =
            json!({"method": "ack_open", "request_id": next["open"]["request_id"], "opened": true});
        assert_eq!(exchange(&dsh, ack)["ok"], true);
        let events = bridge.drain();
        assert!(
            matches!(&events[..], [Event::Opened { source, id, ok: true }] if source == "dsh" && id == "turn")
        );
    }

    #[test]
    fn notice_transport_restores_sources_rotates_and_revokes_credentials() {
        let dir = TestDir::new();
        let mut bridge = Bridge::start(&dir.0, egui::Context::default()).unwrap();
        bridge.authorize("codex", "Codex").unwrap();
        bridge.authorize("dsh", "DeepSeek Harness").unwrap();
        let original = descriptor(&bridge, "codex");
        drop(bridge);

        let mut bridge = Bridge::start(&dir.0, egui::Context::default()).unwrap();
        let restored = descriptor(&bridge, "codex");
        assert_eq!(bridge.sources()[0].name, "Codex");
        assert_ne!(original["token"], restored["token"]);
        assert_eq!(
            exchange(
                &restored,
                json!({"method": "status", "token": original["token"]})
            )["error"],
            "unauthorized"
        );
        exchange(&restored, json!({"method": "complete", "id": "queued"}));
        bridge.revoke("codex").unwrap();
        assert!(!bridge.connection_file("codex").exists());
        assert_eq!(
            exchange(&restored, json!({"method": "status"}))["error"],
            "unauthorized"
        );
        assert!(bridge.drain().is_empty());
        bridge.revoke("dsh").unwrap();
        assert!(bridge.listener.is_none());

        drop(bridge);
        let bridge = Bridge::start(&dir.0, egui::Context::default()).unwrap();
        assert!(bridge.sources().is_empty());
        assert!(bridge.listener.is_none());
    }
}
