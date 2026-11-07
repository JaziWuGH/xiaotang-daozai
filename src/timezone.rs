//! 时间设置：主时钟时区换算、世界时钟，以及互联网标准时间校准。
//!
//! 这里只做「算时间」：界面读换算后的墙上时间字符串，设置页负责开关与城市清单。
//! 系统时区不一定等于用户想看的时区（例如系统是美东、想看北京时间），所以顶栏
//! 与日历等业务模块使用两套时间：`app::Env` 的 `clock/today` 仍是系统本地时间，
//! `display_*` 才是设置里选中的时区。

use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDateTime, Offset, Timelike, Utc};
use chrono_tz::Tz;
use std::net::UdpSocket;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 下拉里的常用时区。空串表示跟随系统时区。
pub const PRESETS: &[(&str, &str)] = &[
    ("", "跟随系统"),
    ("Asia/Shanghai", "北京 / 上海"),
    ("Asia/Hong_Kong", "中国香港"),
    ("Asia/Taipei", "中国台北"),
    ("Asia/Tokyo", "东京"),
    ("Asia/Seoul", "首尔"),
    ("Asia/Singapore", "新加坡"),
    ("Asia/Dubai", "迪拜"),
    ("Europe/London", "伦敦"),
    ("Europe/Paris", "巴黎"),
    ("America/New_York", "纽约"),
    ("America/Los_Angeles", "洛杉矶"),
    ("Australia/Sydney", "悉尼"),
    ("Etc/UTC", "协调世界时 UTC"),
];

/// 默认世界时钟清单：主时钟之外再给几个常用参照。
pub fn default_cities() -> Vec<(String, String)> {
    vec![
        ("纽约".into(), "America/New_York".into()),
        ("伦敦".into(), "Europe/London".into()),
        ("东京".into(), "Asia/Tokyo".into()),
    ]
}

pub fn label_of(tz: &str) -> String {
    PRESETS.iter().find(|(v, _)| *v == tz).map(|(_, l)| l.to_string())
        .unwrap_or_else(|| tz.rsplit('/').next().unwrap_or(tz).to_string())
}

/// 把墙上时间格式化成顶栏需要的三串：`HH:MM:SS` / `YYYY-MM-DD` / `M月D日 周X`。
pub fn render(naive: NaiveDateTime) -> (String, String, String) {
    const WEEK: [&str; 7] = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
    (
        format!("{:02}:{:02}:{:02}", naive.hour(), naive.minute(), naive.second()),
        naive.format("%Y-%m-%d").to_string(),
        format!("{}月{}日 {}", naive.month(), naive.day(), WEEK[naive.weekday().num_days_from_sunday() as usize]),
    )
}

/// 应用网络对时偏差后的 UTC 时刻。
pub fn corrected_utc(offset_ms: i64) -> DateTime<Utc> {
    Utc::now() + ChronoDuration::milliseconds(offset_ms)
}

/// 跟随系统时区（同样应用校准偏差）。
pub fn system(offset_ms: i64) -> (String, String, String) {
    render((chrono::Local::now() + ChronoDuration::milliseconds(offset_ms)).naive_local())
}

/// 纯函数版：给定 UTC 时刻和目标时区，算出三串。便于测试与多城市复用。
pub fn zoned_at(utc: DateTime<Utc>, tz: &Tz) -> (String, String, String) {
    render(utc.with_timezone(tz).naive_local())
}

/// 目标时区的三串；时区名非法时返回 None，交给调用方回退到系统时间。
pub fn zoned(tz: &str, offset_ms: i64) -> Option<(String, String, String)> {
    let zone: Tz = tz.parse().ok()?;
    Some(zoned_at(corrected_utc(offset_ms), &zone))
}

/// 一座城市在世界时钟卡片上的一行。
#[derive(Clone, Debug, PartialEq)]
pub struct CityTime {
    pub label: String,
    pub clock: String,
    pub date: String,
    /// 与 UTC 的偏移分钟数（已含夏令时）。
    pub utc_minutes: i32,
}

pub fn city_time_at(utc: DateTime<Utc>, label: &str, tz: &str) -> Option<CityTime> {
    let zone: Tz = tz.parse().ok()?;
    let at = utc.with_timezone(&zone);
    let (clock, today, _) = render(at.naive_local());
    Some(CityTime {
        label: label.to_string(),
        clock: clock.get(..5).unwrap_or("--:--").to_string(),
        date: today,
        utc_minutes: at.offset().fix().local_minus_utc() / 60,
    })
}

pub fn city_time(label: &str, tz: &str, offset_ms: i64) -> Option<CityTime> {
    city_time_at(corrected_utc(offset_ms), label, tz)
}

/// 目标时区相对主时钟的时差，单位分钟。主时钟是「跟随系统」时按本机 UTC 偏移算。
pub fn diff_minutes(city_utc_minutes: i32, main_utc_minutes: i32) -> i32 {
    city_utc_minutes - main_utc_minutes
}

/// `+8 小时` / `-4 小时` / `+5 小时 30 分` 这样给人看的时差。
pub fn diff_text(minutes: i32) -> String {
    if minutes == 0 { return "同时".into(); }
    let sign = if minutes > 0 { "+" } else { "-" };
    let m = minutes.abs();
    let h = m / 60;
    let rest = m % 60;
    if h == 0 { format!("{sign}{rest} 分") }
    else if rest == 0 { format!("{sign}{h} 小时") }
    else { format!("{sign}{h} 小时 {rest} 分") }
}

/// 当前本机相对 UTC 的偏移分钟数（跟随系统时的主时钟基准）。
pub fn system_utc_minutes() -> i32 {
    chrono::Local::now().offset().fix().local_minus_utc() / 60
}

/// 主时钟当前相对 UTC 的偏移分钟数，用来算各城市与主时钟的时差。
pub fn main_utc_minutes(tz: &str, offset_ms: i64) -> i32 {
    if tz.is_empty() { system_utc_minutes() }
    else { city_time("", tz, offset_ms).map(|c|c.utc_minutes).unwrap_or_else(system_utc_minutes) }
}

// ================= 互联网标准时间 =================

#[derive(Clone, Debug, PartialEq)]
pub struct SyncResult {
    /// 本机时钟需要补偿的毫秒数（正数表示本机慢了）。
    pub offset_ms: i64,
    /// 请求往返耗时，用来提示这次对时是否可信。
    pub rtt_ms: i64,
    /// 成功的来源（NTP 服务器或 HTTP 站点）。
    pub source: String,
    /// 同步完成的本地时间字符串。
    pub at: String,
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// 1900-01-01 到 1970-01-01 的秒数。
const NTP_DELTA: f64 = 2_208_988_800.0;

fn ntp_bytes(bytes: &[u8]) -> Option<f64> {
    if bytes.len() < 8 { return None; }
    let secs = u32::from_be_bytes(bytes[..4].try_into().ok()?) as f64;
    let frac = u32::from_be_bytes(bytes[4..8].try_into().ok()?) as f64 / 4_294_967_296.0;
    // 转回 UNIX 秒；NTP 时间戳的小数在 2036 年回绕，靠高位判断更稳，这里只用于校时。
    Some(secs - NTP_DELTA + frac)
}

fn ntp_stamp(unix_secs: f64) -> [u8; 8] {
    let ntp = (unix_secs + NTP_DELTA).max(0.0);
    let secs = ntp as u32;
    let frac = ((ntp - ntp.floor()) * 4_294_967_296.0) as u32;
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&secs.to_be_bytes());
    out[4..].copy_from_slice(&frac.to_be_bytes());
    out
}

fn sntp_server(server: &str) -> Result<SyncResult, String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("UDP: {e}"))?;
    socket.set_read_timeout(Some(Duration::from_millis(1200))).map_err(|e| format!("UDP: {e}"))?;
    let mut req = [0u8; 48];
    req[0] = 0x1b; // LI=0, VN=3, Mode=3 (client)
    let t1 = unix_now();
    req[40..48].copy_from_slice(&ntp_stamp(t1));
    socket.send_to(&req, (server, 123)).map_err(|e| format!("{server}: {e}"))?;
    let mut buf = [0u8; 48];
    let (n, _) = socket.recv_from(&mut buf).map_err(|e| format!("{server}: {e}"))?;
    let t4 = unix_now();
    if n < 48 { return Err(format!("{server}: 响应过短")); }
    if buf[0] & 0x38 != 0x18 { return Err(format!("{server}: 非服务器响应")); }
    let t2 = ntp_bytes(&buf[32..40]).ok_or_else(|| format!("{server}: 时间戳无效"))?;
    let t3 = ntp_bytes(&buf[40..48]).ok_or_else(|| format!("{server}: 时间戳无效"))?;
    if t2 <= 0.0 || t3 <= 0.0 { return Err(format!("{server}: 服务器未同步")); }
    let offset = ((t2 - t1) + (t3 - t4)) / 2.0;
    let rtt = ((t4 - t1) - (t3 - t2)).max(0.0);
    // 偏差超过 24 小时基本是解析或中间设备出错，不拿它改时钟。
    if offset.abs() > 86_400.0 { return Err(format!("{server}: 偏差异常")); }
    Ok(SyncResult {
        offset_ms: (offset * 1000.0).round() as i64,
        rtt_ms: (rtt * 1000.0).round() as i64,
        source: server.to_string(),
        at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

fn sntp() -> Result<SyncResult, String> {
    let mut last = String::new();
    for server in ["ntp.aliyun.com", "time.windows.com", "cn.pool.ntp.org", "pool.ntp.org"] {
        match sntp_server(server) {
            Ok(result) => return Ok(result),
            Err(error) => last = error,
        }
    }
    Err(format!("NTP 全部失败（{last}）"))
}

/// HTTP `Date` 响应头只有秒精度，用 RTT/2 与半秒中值做补偿。
fn http_date_site(url: &str) -> Result<SyncResult, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(3)).redirects(0).build();
    let start = Instant::now();
    let response = agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let rtt = start.elapsed().as_secs_f64();
    let date = response.header("Date").ok_or_else(|| format!("{url}: 无 Date 头"))?;
    let server = chrono::DateTime::parse_from_rfc2822(date).map_err(|e| format!("{url}: {e}"))?.timestamp() as f64;
    let mid = unix_now() - rtt / 2.0;
    let offset = (server + 0.5 - mid) * 1000.0;
    if offset.abs() > 86_400_000.0 { return Err(format!("{url}: 偏差异常")); }
    Ok(SyncResult {
        offset_ms: offset.round() as i64,
        rtt_ms: (rtt * 1000.0).round() as i64,
        source: url.trim_start_matches("https://").to_string(),
        at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

fn http_date() -> Result<SyncResult, String> {
    let mut last = String::new();
    for url in ["https://www.baidu.com", "https://www.cloudflare.com", "https://www.microsoft.com"] {
        match http_date_site(url) {
            Ok(result) => return Ok(result),
            Err(error) => last = error,
        }
    }
    Err(format!("HTTP 对时全部失败（{last}）"))
}

/// 先试 SNTP（毫秒级），UDP 123 不通时回退到任意站点的 HTTP Date 头。
pub fn sync_once() -> Result<SyncResult, String> {
    sntp().or_else(|ntp_error| http_date().map_err(|http_error| format!("{ntp_error}；{http_error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.from_utc_datetime(&NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, mi, s).unwrap())
    }

    #[test]
    fn render_formats_clock_date_and_weekday() {
        let (clock, today, date) = render(NaiveDate::from_ymd_opt(2026, 11, 6).unwrap().and_hms_opt(9, 5, 3).unwrap());
        assert_eq!(clock, "09:05:03");
        assert_eq!(today, "2026-11-06");
        assert_eq!(date, "11月6日 周五");
    }

    #[test]
    fn beijing_is_eight_hours_ahead_of_utc() {
        let shanghai: Tz = "Asia/Shanghai".parse().unwrap();
        assert_eq!(zoned_at(utc(2026, 1, 1, 16, 0, 0), &shanghai).0, "00:00:00");
        assert_eq!(zoned_at(utc(2026, 1, 1, 16, 0, 0), &shanghai).1, "2026-01-02");
        assert_eq!(city_time_at(utc(2026, 1, 1, 16, 0, 0), "北京", "Asia/Shanghai").unwrap().utc_minutes, 480);
    }

    #[test]
    fn new_york_observes_daylight_saving() {
        let winter = city_time_at(utc(2026, 1, 15, 12, 0, 0), "纽约", "America/New_York").unwrap();
        let summer = city_time_at(utc(2026, 7, 15, 12, 0, 0), "纽约", "America/New_York").unwrap();
        assert_eq!(winter.utc_minutes, -300);
        assert_eq!(summer.utc_minutes, -240);
        assert_eq!(winter.clock, "07:00");
        assert_eq!(summer.clock, "08:00");
    }

    #[test]
    fn unknown_zone_falls_back_to_none() {
        assert!(zoned("Mars/Olympus", 0).is_none());
        assert!(city_time("火星", "Mars/Olympus", 0).is_none());
        assert_eq!(label_of("Asia/Shanghai"), "北京 / 上海");
        assert_eq!(label_of(""), "跟随系统");
    }

    #[test]
    fn every_preset_is_a_real_timezone() {
        for (value, label) in PRESETS {
            assert!(!label.is_empty());
            if !value.is_empty() { assert!(value.parse::<Tz>().is_ok(), "{value} 不是合法时区"); }
        }
        for (_, value) in default_cities() { assert!(value.parse::<Tz>().is_ok()); }
    }

    #[test]
    fn diff_text_handles_half_hours_and_signs() {
        assert_eq!(diff_text(0), "同时");
        assert_eq!(diff_text(480), "+8 小时");
        assert_eq!(diff_text(-300), "-5 小时");
        assert_eq!(diff_text(330), "+5 小时 30 分");
        assert_eq!(diff_minutes(480, -300), 780);
    }

    #[test]
    fn ntp_timestamp_round_trip() {
        let unix = 1_762_387_200.0_f64;
        let bytes = ntp_stamp(unix);
        assert!((ntp_bytes(&bytes).unwrap() - unix).abs() < 0.001);
    }

    #[test]
    fn corrected_utc_applies_offset() {
        let base = corrected_utc(1500);
        assert!(base > Utc::now());
    }
}
