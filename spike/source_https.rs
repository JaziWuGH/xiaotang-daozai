//! HTTPS 数据源的授权校验和受控读取。脚本只接收响应正文。
use super::{
    source_read::{check_cancelled, read_text, SourceGrant},
    SourceInput, Values,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
    net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use url::{Host, Url};

const LIMIT: u64 = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
const ADDRESS_ERROR: &str = "请输入白名单域名下的 HTTPS 地址，端口使用 443";
const SERVICE: &str = "daozai.plugin-https-source";
pub(super) const MIN_REFRESH_SECONDS: u32 = 60;
pub(super) const MAX_REFRESH_SECONDS: u32 = 86400;
pub(super) const REFRESH_FIELD: &str = "";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Network {
    pub(super) hosts: Vec<String>,
    #[serde(
        default,
        deserialize_with = "super::present_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) url: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::present_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) sources: Option<Vec<NetworkSource>>,
    #[serde(
        default,
        deserialize_with = "super::present_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) credential: Option<Credential>,
    #[serde(
        default,
        deserialize_with = "super::present_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) min_refresh_seconds: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NetworkSource {
    pub(super) key: String,
    pub(super) url: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Attribution {
    pub(super) label: String,
    #[serde(
        default,
        deserialize_with = "super::present_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) url: Option<String>,
}

impl Attribution {
    pub(super) fn valid(&self, network: &Network) -> bool {
        !self.label.trim().is_empty()
            && self.label.chars().count() <= 80
            && !self.label.chars().any(char::is_control)
            && self.url.as_ref().is_none_or(|url| network.address(url).is_ok())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Credential {
    pub(super) label: String,
    header: String,
    #[serde(
        default,
        deserialize_with = "super::present_value",
        skip_serializing_if = "Option::is_none"
    )]
    prefix: Option<String>,
}

impl Network {
    pub(super) fn valid(&self, settings: &Values) -> bool {
        (1..=4).contains(&self.hosts.len())
            && self.hosts.iter().enumerate().all(|(i, host)| {
                valid_host(host)
                    && !self.hosts[..i]
                        .iter()
                        .any(|other| other.eq_ignore_ascii_case(host))
            })
            && self
                .url
                .as_deref()
                .is_none_or(|url| self.address(url).is_ok())
            && self.credential.as_ref().is_none_or(Credential::valid)
            && self.min_refresh_seconds.is_none_or(|seconds| {
                (MIN_REFRESH_SECONDS..=MAX_REFRESH_SECONDS).contains(&seconds)
            })
            && self.sources.as_ref().is_none_or(|sources| {
                let mut keys = BTreeSet::new();
                self.url.is_none()
                    && (1..=3).contains(&sources.len())
                    && sources.iter().all(|source| {
                        valid_key(&source.key)
                            && keys.insert(&source.key)
                            && self.expand(&source.url, settings).is_ok()
                    })
            })
    }

    pub(super) fn only_hosts(&self) -> bool {
        self.url.is_none()
            && self.sources.is_none()
            && self.credential.is_none()
            && self.min_refresh_seconds.is_none()
    }

    pub(super) fn same_authorization(&self, other: &Self) -> bool {
        self.hosts == other.hosts
            && self.sources == other.sources
            && self.url == other.url
            && self.credential == other.credential
    }

    pub(super) fn refresh_seconds(&self, default: u32, preference: Option<u32>) -> u32 {
        let Some(min) = self.min_refresh_seconds else {
            return default;
        };
        preference.unwrap_or(default).clamp(min, MAX_REFRESH_SECONDS)
    }

    pub(super) fn address(&self, raw: &str) -> io::Result<Url> {
        allowed_url(raw, &self.hosts).ok_or_else(|| io::Error::other(ADDRESS_ERROR))
    }

    pub(super) fn grant(&self, raw: &str) -> io::Result<SourceGrant> {
        if let Some(sources) = &self.sources {
            return Ok(SourceGrant::HttpsSources {
                hosts: self.hosts.clone(),
                sources: sources.clone(),
            });
        }
        Ok(SourceGrant::Https {
            url: self.address(raw.trim())?.into(),
        })
    }

    pub(super) fn accepts(&self, grant: &SourceGrant) -> bool {
        match grant {
            SourceGrant::Https { url } => self.sources.is_none() && self.address(url).is_ok(),
            SourceGrant::HttpsSources { hosts, sources } => {
                self.hosts == *hosts && self.sources.as_ref() == Some(sources)
            }
            _ => false,
        }
    }

    fn expand(&self, raw: &str, settings: &Values) -> io::Result<Url> {
        self.address(&expand_url(raw, settings).ok_or_else(|| io::Error::other(ADDRESS_ERROR))?)
    }

    pub(super) fn read(&self, id: &str, raw: &str, cancelled: &AtomicBool) -> io::Result<String> {
        let url = self.address(raw)?;
        self.read_address(id, url, cancelled)
    }

    pub(super) fn read_sources(
        &self,
        id: &str,
        settings: &Values,
        cancelled: &AtomicBool,
        fetch: &mut FetchState,
        interval: Duration,
    ) -> SourceInput {
        fetch.read_sources(self.sources.as_deref().unwrap_or_default(), Instant::now, interval, |source| {
            self.expand(&source.url, settings)
                .and_then(|url| self.read_address(id, url, cancelled))
        })
    }

    fn read_address(&self, id: &str, url: Url, cancelled: &AtomicBool) -> io::Result<String> {
        check_cancelled(cancelled)?;
        let secret = match &self.credential {
            Some(config) => {
                let secret =
                    load_secret(id)?.ok_or_else(|| io::Error::other("请重新授权数据源密钥"))?;
                let value = config.header_value(&secret)?;
                Some((config, secret, value))
            }
            None => None,
        };
        let deadline = Instant::now() + TIMEOUT;
        let gold = id == "daozai.gold" && url.host_str() == Some("xaus.com")
            && url.path() == "/api/v1/spot" && self.credential.is_none();
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .timeout_connect(TIMEOUT)
            .resolver(move |address: &str| resolve_public(address, deadline, gold))
            .build();
        let mut request = agent
            .get(url.as_str())
            .timeout(deadline.saturating_duration_since(Instant::now()))
            .set("Accept", "application/json, text/plain")
            .set("Accept-Encoding", "identity");
        if let Some((config, _, value)) = &secret {
            request = request.set_sensitive(&config.header, value);
        }
        check_cancelled(cancelled)?;
        let response = request
            .call()
            .map_err(|_| io::Error::other("HTTPS 数据源无法读取"))?;
        let text = response_text(response, cancelled)?;
        if secret
            .as_ref()
            .is_some_and(|(_, secret, _)| text.contains(secret))
        {
            return Err(io::Error::other("HTTPS 数据源返回了凭据"));
        }
        Ok(text)
    }
}

fn valid_key(key: &str) -> bool {
    key.len() <= 40
        && key.starts_with(|c: char| c.is_ascii_lowercase())
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}

fn expand_url(raw: &str, settings: &Values) -> Option<String> {
    if raw.len() > 2048 {
        return None;
    }
    let split = raw.get(8..)?.find(['/', '?']).map_or(raw.len(), |i| i + 8);
    let (authority, tail) = raw.split_at(split);
    if authority.contains(['{', '}']) || tail.contains('#') {
        return None;
    }
    let mut url = String::from(authority);
    let mut remaining = tail;
    while let Some(i) = remaining.find(['{', '}']) {
        let (literal, placeholder) = remaining.split_at(i);
        url.push_str(literal);
        let (key, rest) = placeholder.strip_prefix("{settings.")?.split_once('}')?;
        let value = settings.get(key)?;
        // 对路径和查询串中的设置值逐字节编码。
        for byte in value.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'~') {
                url.push(char::from(byte));
            } else {
                use std::fmt::Write;
                write!(url, "%{byte:02X}").ok()?;
            }
        }
        remaining = rest;
    }
    url.push_str(remaining);
    if url[split..].split('?').next()?.split('/').any(|segment| {
        ["%2E", ".%2E", "%2E.", "%2E%2E"].iter().any(|dot| segment.eq_ignore_ascii_case(dot))
    }) {
        return None;
    }
    Some(url)
}

#[derive(Clone, Default)]
pub(super) struct FetchState {
    attempts: BTreeMap<String, Attempt>,
    pub(super) last_success: Option<chrono::DateTime<chrono::Local>>,
}

#[derive(Clone)]
struct Attempt {
    failures: u32,
    due: Instant,
    state: SourceInput,
}

impl FetchState {
    fn read_sources(
        &mut self,
        sources: &[NetworkSource],
        mut now: impl FnMut() -> Instant,
        interval: Duration,
        mut read: impl FnMut(&NetworkSource) -> io::Result<String>,
    ) -> SourceInput {
        SourceInput::Multiple(sources.iter().map(|source| {
            let value = self.read(&source.key, now(), interval, || read(source));
            (source.key.clone(), value)
        }).collect())
    }

    pub(super) fn read(
        &mut self,
        key: &str,
        now: Instant,
        interval: Duration,
        read: impl FnOnce() -> io::Result<String>,
    ) -> SourceInput {
        if let Some(previous) = self.attempts.get(key).filter(|attempt| attempt.due > now) {
            return previous.state.clone();
        }
        let value = match read() {
            Ok(text) => SourceInput::Ready { text },
            Err(_) => SourceInput::Offline,
        };
        self.record(key, now, interval, value.clone());
        value
    }

    fn record(&mut self, key: &str, now: Instant, interval: Duration, state: SourceInput) {
        let failures = if state.ready() {
            self.last_success = Some(chrono::Local::now());
            0
        } else {
            self.attempts.get(key).map_or(1, |old| (old.failures + 1).min(4))
        };
        let cap = interval.max(Duration::from_secs(1800));
        let delay = (interval * (1 << failures.saturating_sub(1))).min(cap);
        self.attempts.insert(key.into(), Attempt { failures, due: now + delay, state });
    }

    pub(super) fn next_due(&self) -> Option<Instant> {
        self.attempts.values().map(|attempt| attempt.due).min()
    }
}

impl Credential {
    fn valid(&self) -> bool {
        !self.label.trim().is_empty()
            && self.label.chars().count() <= 80
            && !self.label.chars().any(char::is_control)
            && (1..=64).contains(&self.header.len())
            && self
                .header
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_\x60|~".contains(&byte))
            && ![
                "Host",
                "Connection",
                "Content-Length",
                "Transfer-Encoding",
                "TE",
                "Upgrade",
                "Trailer",
                "Expect",
                "Accept-Encoding",
                "Proxy-Authorization",
                "Proxy-Connection",
            ]
            .iter()
            .any(|name| self.header.eq_ignore_ascii_case(name))
            && self.prefix.as_ref().is_none_or(|prefix| {
                prefix.len() <= 80 && prefix.bytes().all(|b| matches!(b, b' '..=b'~'))
            })
    }

    pub(super) fn header_value(&self, secret: &str) -> io::Result<String> {
        if !(1..=2048).contains(&secret.len())
            || secret.trim() != secret
            || !secret.bytes().all(|b| matches!(b, b' '..=b'~'))
        {
            return Err(io::Error::other("密钥须为 1–2048 个可打印 ASCII 字符"));
        }
        Ok(format!(
            "{}{secret}",
            self.prefix.as_deref().unwrap_or_default()
        ))
    }
}

fn valid_host(host: &str) -> bool {
    host.len() <= 253
        && host.contains('.')
        && !host.to_ascii_lowercase().ends_with(".localhost")
        && host.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && matches!(Host::parse(host), Ok(Host::Domain(domain)) if domain.eq_ignore_ascii_case(host))
}

fn allowed_url(raw: &str, hosts: &[String]) -> Option<Url> {
    if raw.len() > 2048
        || raw.chars().any(|c| c.is_control() || c == '\\')
        || !raw.get(..8)?.eq_ignore_ascii_case("https://")
        || raw[8..]
            .split(['/', '?', '#'])
            .next()
            .is_none_or(|authority| authority.is_empty() || authority.contains('@'))
    {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    let Host::Domain(host) = url.host()? else {
        return None;
    };
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.fragment().is_none()
        && hosts
            .iter()
            .any(|allowed| host.eq_ignore_ascii_case(allowed)))
    .then_some(url)
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !matches!(
        (a, b, c),
        (0 | 10 | 127 | 224..=255, _, _)
            | (100, 64..=127, _)
            | (169, 254, _)
            | (172, 16..=31, _)
            | (192, 0, 0 | 2)
            | (192, 168, _)
            | (198, 18 | 19, _)
            | (198, 51, 100)
            | (203, 0, 113)
    )
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or_else(
            || {
                let [a, b, ..] = ip.segments();
                // 仅接受全球单播地址，排除 Teredo、6to4 及文档地址。
                (0x2000..=0x3fff).contains(&a)
                    && !matches!((a, b), (0x2001, 0 | 0xdb8) | (0x2002 | 0x3fff, _))
            },
            public_v4,
        ),
    }
}

fn public_addresses(addresses: impl Iterator<Item = SocketAddr>) -> io::Result<Vec<SocketAddr>> {
    let addresses: Vec<_> = addresses.filter(|addr| public_ip(addr.ip())).collect();
    if addresses.is_empty() {
        return Err(io::Error::other("HTTPS 域名未解析到公网地址"));
    }
    Ok(addresses)
}

fn resolve_public(address: &str, deadline: Instant, gold: bool) -> io::Result<Vec<SocketAddr>> {
    // 系统 DNS 是同步调用。最多保留一个查询线程，请求超时后立即结束等待。
    static RESOLVING: AtomicBool = AtomicBool::new(false);
    let (host, port) = address
        .rsplit_once(':')
        .ok_or_else(|| io::Error::other(ADDRESS_ERROR))?;
    if port != "443" || !valid_host(host) {
        return Err(io::Error::other(ADDRESS_ERROR));
    }
    let target = format!("{host}.:443");
    if RESOLVING.swap(true, Ordering::AcqRel) {
        return Err(io::Error::other("系统 DNS 正在查询"));
    }
    let (send, receive) = mpsc::sync_channel(1);
    if let Err(error) = std::thread::Builder::new()
        .name("daozai-plugin-dns".into())
        .spawn(move || {
            let addresses = target.to_socket_addrs().map(|items| items.collect::<Vec<_>>());
            RESOLVING.store(false, Ordering::Release);
            let _ = send.send(addresses);
        })
    {
        RESOLVING.store(false, Ordering::Release);
        return Err(error);
    }
    let addresses = receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "HTTPS 域名解析超时"))??;
    if gold && host == "xaus.com" && !addresses.iter().any(|addr| public_ip(addr.ip()))
        && addresses.iter().any(|addr| matches!(addr.ip(), IpAddr::V4(ip)
            if matches!(ip.octets(), [198, 18 | 19, _, _])))
    {
        // 代理的虚拟地址无法通过公网校验。官方固定报价源可改查真实公网地址。
        let remaining = deadline.saturating_duration_since(Instant::now());
        let response = ureq::AgentBuilder::new().try_proxy_from_env(false).redirects(0).build()
            .get("https://1.1.1.1/dns-query?name=xaus.com&type=A")
            .timeout(remaining).set("Accept", "application/dns-json")
            .call().map_err(|_| io::Error::other("无法查询金价源公网地址"))?;
        if response.status() != 200 {
            return Err(io::Error::other("无法查询金价源公网地址"));
        }
        return gold_dns_answers(response.into_reader());
    }
    public_addresses(addresses.into_iter())
}

fn gold_dns_answers(reader: impl Read) -> io::Result<Vec<SocketAddr>> {
    let reply: serde_json::Value = serde_json::from_reader(reader.take(4096))?;
    let answers = reply.get("Answer").and_then(serde_json::Value::as_array)
        .filter(|_| reply["Status"].as_u64() == Some(0))
        .ok_or_else(|| io::Error::other("无法查询金价源公网地址"))?;
    public_addresses(answers.iter().filter_map(|answer| {
        if answer["type"].as_u64() != Some(1) { return None; }
        let ip: Ipv4Addr = answer["data"].as_str()?.parse().ok()?;
        Some(SocketAddr::new(ip.into(), 443))
    }))
}

fn response_text(response: ureq::Response, cancelled: &AtomicBool) -> io::Result<String> {
    if !(200..300).contains(&response.status()) {
        return Err(io::Error::other("HTTPS 数据源没有直接返回数据"));
    }
    read_text(response.into_reader(), cancelled, LIMIT)
}

fn keyring_entry(id: &str) -> io::Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, id).map_err(|_| io::Error::other("无法访问 Windows 凭据管理器"))
}

pub(super) fn load_secret(id: &str) -> io::Result<Option<String>> {
    match keyring_entry(id)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(io::Error::other("无法读取数据源密钥")),
    }
}

pub(super) fn save_secret(id: &str, secret: &str) -> io::Result<()> {
    keyring_entry(id)?
        .set_password(secret)
        .map_err(|_| io::Error::other("无法保存数据源密钥"))
}

pub(super) fn delete_secret(id: &str) -> io::Result<()> {
    match keyring_entry(id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(io::Error::other("无法删除数据源密钥")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gold_dns_answers_accept_only_public_ipv4_records() {
        let json = br#"{"Status":0,"Answer":[{"type":1,"data":"198.18.2.124"},
            {"type":1,"data":"127.0.0.1"},{"type":1,"data":"98.84.224.111"},
            {"type":5,"data":"18.208.88.157"}]}"#;
        assert_eq!(gold_dns_answers(&json[..]).unwrap(),
            ["98.84.224.111:443".parse::<SocketAddr>().unwrap()]);
        assert!(gold_dns_answers(&br#"{"Status":3,"Answer":[]}"#[..]).is_err());
    }

    #[test]
    fn domains_and_addresses_must_match_the_https_grant() {
        let hosts = vec!["example.com".to_owned()];
        for host in [
            "*",
            "*.example.com",
            "127.0.0.1",
            "[::1]",
            "127.1",
            "localhost",
            "api.localhost",
            "example.com:443",
            "example.com.",
        ] {
            assert!(!valid_host(host), "{host}");
        }
        assert!(valid_host("API.Example.com"));
        for raw in [
            "http://example.com",
            "https://example.net",
            "https://sub.example.com",
            "https://127.0.0.1",
            "https://user:secret@example.com",
            "https://@example.com",
            "https://example.com:444",
            "https://example.com/#part",
            "https:///example.com",
            "https://example.com.evil.net",
        ] {
            assert!(allowed_url(raw, &hosts).is_none(), "{raw}");
        }
        assert_eq!(
            allowed_url("https://EXAMPLE.COM:443/path?arg=1", &hosts)
                .unwrap()
                .as_str(),
            "https://example.com/path?arg=1"
        );
    }

    #[test]
    fn templates_encode_settings_and_keep_authority_fixed() {
        let network: Network = serde_json::from_value(serde_json::json!({
            "hosts": ["example.com"],
            "sources": [{"key": "sample", "url": "https://example.com/path/{settings.value}?arg={settings.value}"}]
        })).unwrap();
        let settings = Values::from([("value".into(), "中文 /?#+@&=x".into())]);
        assert!(network.valid(&settings));
        let encoded = "%E4%B8%AD%E6%96%87%20%2F%3F%23%2B%40%26%3Dx";
        let template = &network.sources.as_ref().unwrap()[0].url;
        assert_eq!(
            network.expand(template, &settings).unwrap().as_str(),
            format!("https://example.com/path/{encoded}?arg={encoded}"),
        );
        for raw in [
            "http://example.com/{settings.value}",
            "https://127.0.0.1/{settings.value}",
            "https://example.net/{settings.value}",
            "https://user:secret@example.com/{settings.value}",
            "https://example.com:444/{settings.value}",
            "https://{settings.value}.example.com/path",
            "https://example.com/?arg={settings.missing}",
        ] {
            assert!(network.expand(raw, &settings).is_err(), "{raw}");
        }
        let grant = network.grant("").unwrap();
        assert!(network.accepts(&grant));
        let mut updated = network.clone();
        updated.sources.as_mut().unwrap()[0].url.push_str("&after=1");
        assert!(!updated.accepts(&grant));
        assert!(!updated.same_authorization(&network));
        let mut malformed = network.clone();
        let duplicate = malformed.sources.as_ref().unwrap()[0].clone();
        malformed.sources.as_mut().unwrap().push(duplicate);
        assert!(!malformed.valid(&settings));
        malformed = network.clone();
        malformed.sources = Some(Vec::new());
        assert!(!malformed.valid(&settings));
        let dot = Values::from([("value".into(), "..".into())]);
        assert!(network.expand(template, &dot).is_err());
    }

    #[test]
    fn network_sources_keep_independent_retry_budgets() {
        let sources = [
            NetworkSource { key: "gold".into(), url: String::new() },
            NetworkSource { key: "fx".into(), url: String::new() },
        ];
        let start = Instant::now();
        let interval = Duration::from_secs(60);
        let mut state = FetchState::default();
        let mut fx_reads = 0;
        for seconds in (0..=180).step_by(60) {
            let now = start + Duration::from_secs(seconds);
            let input = state.read_sources(&sources, || now, interval, |source| {
                if source.key == "fx" {
                    fx_reads += 1;
                    if seconds < 180 {
                        return Err(io::Error::other("fixture"));
                    }
                }
                Ok("ready".into())
            });
            let SourceInput::Multiple(results) = input else { panic!() };
            assert!(results["gold"].ready());
            assert_eq!(results["fx"].ready(), seconds == 180);
        }
        assert_eq!(fx_reads, 3);
        assert_eq!(state.next_due(), Some(start + Duration::from_secs(240)));
        assert!(state.last_success.is_some());
    }

    #[test]
    fn later_sources_keep_their_own_minimum_interval() {
        let sources = [
            NetworkSource { key: "gold".into(), url: String::new() },
            NetworkSource { key: "fx".into(), url: String::new() },
        ];
        let start = Instant::now();
        let interval = Duration::from_secs(60);
        let clock = std::cell::Cell::new(start);
        let mut state = FetchState::default();

        state.read_sources(&sources, || clock.get(), interval, |source| {
            if source.key == "gold" {
                clock.set(start + Duration::from_secs(10));
            }
            Ok("ready".into())
        });
        clock.set(start + interval);
        let mut queried = Vec::new();
        state.read_sources(&sources, || clock.get(), interval, |source| {
            queried.push(source.key.clone());
            Ok("ready".into())
        });
        assert_eq!(queried, ["gold"]);
        assert_eq!(state.next_due(), Some(start + Duration::from_secs(70)));
    }

    #[test]
    fn retry_progression_resets_and_caps() {
        let start = Instant::now();
        let interval = Duration::from_secs(300);
        let mut state = FetchState::default();
        let mut time = start;
        for expected in [300, 600, 1200, 1800, 1800] {
            state.record("gold", time, interval, SourceInput::Offline);
            time += Duration::from_secs(expected);
            assert_eq!(state.next_due(), Some(time));
        }
        state.record("gold", time, interval, SourceInput::Ready { text: "ready".into() });
        time += interval;
        assert_eq!(state.next_due(), Some(time));
        state.record("gold", time, interval, SourceInput::Offline);
        assert_eq!(state.next_due(), Some(time + interval));
    }

    #[test]
    fn resolver_uses_only_public_addresses() {
        let blocked = [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.2",
            "172.31.0.1",
            "192.168.1.1",
            "224.0.0.1",
            "::",
            "::1",
            "fe80::1",
            "fc00::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
        ];
        let make = |s: &str| SocketAddr::new(s.parse().unwrap(), 443);
        assert!(public_addresses(blocked.map(make).into_iter()).is_err());
        let public = ["8.8.8.8", "2001:4860:4860::8888"].map(make);
        let all = blocked.map(make).into_iter().chain(public);
        assert_eq!(public_addresses(all).unwrap(), public);
    }

    #[test]
    fn redirects_oversize_and_invalid_text_are_rejected() {
        let cancelled = AtomicBool::new(false);
        let redirect = ureq::Response::new(302, "Found", "redirect").unwrap();
        assert!(response_text(redirect, &cancelled).is_err());
        let body = "a".repeat(LIMIT as usize);
        let response = ureq::Response::new(200, "OK", &body).unwrap();
        assert_eq!(response_text(response, &cancelled).unwrap(), body);
        let body = "a".repeat(LIMIT as usize + 1);
        let response = ureq::Response::new(200, "OK", &body).unwrap();
        assert!(response_text(response, &cancelled).is_err());
        assert!(read_text(&[0xff][..], &cancelled, LIMIT).is_err());
    }

    #[test]
    fn credential_headers_are_validated_and_redacted() {
        let mut config = Credential {
            label: "密钥".into(),
            header: "X-Service-Key".into(),
            prefix: Some("Bearer ".into()),
        };
        assert!(config.valid());
        assert!(config.header_value("abc\r\nHost: other.example").is_err());
        let header = config.header_value("example-private-value").unwrap();
        let request = ureq::get("https://example.com").set_sensitive(&config.header, &header);
        assert_eq!(request.header(&config.header), Some(header.as_str()));
        let debug = format!("{request:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("example-private-value"));
        config.header = "hOsT".into();
        assert!(!config.valid());
    }
}
