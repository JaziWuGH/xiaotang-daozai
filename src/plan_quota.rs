//! Read-only subscription meters. Protocol references (no real-account verification):
//! OpenCode: anomalyco/opencode d21b0790a6c146815f28fea604e41e922f009590,
//! packages/console/app/src/routes/zen/go/v1/usage.ts.
//! MiniMax: MiniMax-AI/cli 38aba5f26173dd1065b115b221d79c9ddd87c6bb,
//! src/{client/endpoints,config/schema,types/api,utils/quota,output/quota-table}.ts.
//! GLM: zai-org/zai-coding-plugins, plugins/glm-plan-usage/skills/
//! usage-query-skill/scripts/query-usage.mjs (official usage plugin).
use super::{Snapshot, Window};
use serde_json::Value;
use std::io::Read;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanProvider { OpenCodeGo, MiniMaxChina, MiniMaxGlobal, GlmChina, GlmGlobal }
impl PlanProvider {
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::OpenCodeGo => "https://opencode.ai/zen/go/v1/usage",
            Self::MiniMaxChina => "https://api.minimax.cn/v1/token_plan/remains",
            Self::MiniMaxGlobal => "https://api.minimax.io/v1/token_plan/remains",
            Self::GlmChina => "https://open.bigmodel.cn/api/monitor/usage/quota/limit",
            Self::GlmGlobal => "https://api.z.ai/api/monitor/usage/quota/limit",
        }
    }
    fn authorization(self, key: &str) -> String {
        match self { Self::GlmChina | Self::GlmGlobal => key.into(), _ => format!("Bearer {key}") }
    }
}

const MAX_BODY: u64 = 1024 * 1024;
/// Caller must supply quota::http_agent(): bounded timeout, redirects disabled,
/// user's configured proxy preserved. Never follow a region redirect with a key.
pub fn request(agent: &ureq::Agent, provider: PlanProvider, key: &str) -> Result<Snapshot, (String, u64)> {
    let key = key.trim();
    if key.is_empty() || key.len() > 8192 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(("请输入有效的套餐 API Key".into(), 0));
    }
    let response = agent.get(provider.endpoint())
        .set("Authorization", &provider.authorization(key)).set("Accept", "application/json")
        .call().map_err(|e| match e {
            ureq::Error::Status(s, _) => status_error(s),
            _ => ("套餐额度网络请求失败，请稍后重试".into(), 30),
        })?;
    if !(200..300).contains(&response.status()) { return Err(status_error(response.status())); }
    let mut body = Vec::new();
    response.into_reader().take(MAX_BODY + 1).read_to_end(&mut body)
        .map_err(|_| ("无法读取套餐额度响应".into(), 30))?;
    if body.len() as u64 > MAX_BODY { return Err(("套餐额度响应过大".into(), 30)); }
    let value = serde_json::from_slice(&body).map_err(|_| ("套餐额度响应不是有效 JSON".into(), 30))?;
    parse(provider, &value).map_err(|e| (e, 30))
}
fn status_error(status: u16) -> (String, u64) {
    match status {
        401 => ("套餐密钥认证失败（401），请检查密钥和地区".into(), 0),
        403 => ("套餐额度查询被拒绝（403），请检查套餐权限和地区".into(), 60),
        429 => ("套餐额度查询过于频繁，请稍后重试".into(), 180),
        300..=399 => ("套餐服务发生重定向，已停止以保护密钥；请检查地区".into(), 0),
        _ => (format!("套餐额度服务暂不可用（{status}）"), 30),
    }
}
fn number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok()).filter(|n| n.is_finite() && *n >= 0.0)
}
fn percent(v: &Value) -> Option<f32> { number(v).filter(|n| *n <= 100.0).map(|n| n as f32) }
fn millis(v: &Value) -> Option<i64> {
    number(v).filter(|n| *n >= 1_000_000_000_000.0 && *n <= 253_402_300_799_000.0).map(|n| (n / 1000.0) as i64)
}
fn iso(v: &Value) -> Option<i64> { chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|d| d.timestamp()) }
fn window(label: &str, seconds: Option<i64>, remaining: f32, reset: Option<i64>) -> Window {
    Window { label: label.into(), seconds, remaining, reset }
}
pub fn parse(provider: PlanProvider, v: &Value) -> Result<Snapshot, String> {
    let mut snapshot = Snapshot { at: chrono::Utc::now().timestamp(), ..Default::default() };
    match provider {
        PlanProvider::OpenCodeGo => parse_opencode(v, &mut snapshot)?,
        PlanProvider::MiniMaxChina | PlanProvider::MiniMaxGlobal => parse_minimax(v, &mut snapshot)?,
        PlanProvider::GlmChina | PlanProvider::GlmGlobal => parse_glm(v, &mut snapshot)?,
    }
    if snapshot.windows.is_empty() && snapshot.extra.is_empty() {
        return Err("服务未返回可识别的有限套餐额度；未知或无限额度不显示为零".into());
    }
    Ok(snapshot)
}
fn parse_opencode(v: &Value, s: &mut Snapshot) -> Result<(), String> {
    if !v["error"].is_null() { return Err("OpenCode Go 未返回可用套餐额度".into()); }
    s.plan = "订阅额度".into();
    // The API does not publish rolling duration. Keep provider ordering stable
    // in Snapshot::rings rather than sorting a known week ahead of rolling.
    for (key, label, duration) in [("rolling", "滚动", None), ("weekly", "本周", None), ("monthly", "本月", None)] {
        let row = &v["usage"][key];
        if let Some(p) = percent(&row["percent"]) {
            s.windows.push(window(label, duration, 100.0 - p, iso(&row["resetsAt"])));
        }
    }
    Ok(())
}
fn minimax_percent(row: &Value, prefix: &str) -> Option<f32> {
    let status = &row[format!("{prefix}_status")];
    // Explicit unlimited/unknown states cannot be represented by a finite ring.
    if !status.is_null() && !matches!(status.as_u64(), Some(1 | 2)) { return None; }
    let explicit = &row[format!("{prefix}_remaining_percent")];
    if !explicit.is_null() { return percent(explicit); }
    // Legacy protocol: usage_count is remaining, NOT used. Zero total is unknown.
    let total = number(&row[format!("{prefix}_total_count")]).filter(|n| *n > 0.0)?;
    let remaining = number(&row[format!("{prefix}_usage_count")]).filter(|n| *n <= total)?;
    Some((remaining / total * 100.0) as f32)
}
fn parse_minimax(v: &Value, s: &mut Snapshot) -> Result<(), String> {
    if let Some(code) = v.pointer("/base_resp/status_code") {
        if number(code) != Some(0.0) { return Err("MiniMax 未返回可用套餐额度，请检查套餐密钥和地区".into()); }
    }
    s.plan = "编程套餐".into();
    let rows = v["model_remains"].as_array().ok_or("MiniMax 额度响应缺少模型列表")?;
    // General is the authoritative shared pool in current plans. Do not sum
    // overlapping model pools or substitute unrelated video/image quotas.
    let row = rows.iter().find(|r| r["model_name"].as_str() == Some("general"))
        .or_else(|| rows.iter().find(|r| r["model_name"].as_str() == Some("MiniMax-M*")))
        .ok_or("MiniMax 未返回可识别的编程套餐额度")?;
    for (prefix, start, end, label) in [
        ("current_interval", "start_time", "end_time", "当前周期"),
        ("current_weekly", "weekly_start_time", "weekly_end_time", "本周"),
    ] {
        if let Some(mut p) = minimax_percent(row, prefix) {
            if prefix == "current_weekly" && !row["weekly_boost_permille"].is_null() {
                let boost = number(&row["weekly_boost_permille"]).filter(|n| *n > 0.0 && *n <= 10000.0)
                    .ok_or("MiniMax 周额度倍率无效")?;
                p *= (boost / 1000.0) as f32;
            }
            let reset = millis(&row[end]);
            let duration = millis(&row[start]).zip(reset).and_then(|(a,b)| b.checked_sub(a)).filter(|d| *d > 0);
            s.windows.push(window(label, duration, p, reset));
        }
    }
    Ok(())
}
fn parse_glm(v: &Value, s: &mut Snapshot) -> Result<(), String> {
    if v["success"].as_bool() == Some(false) || (!v["code"].is_null() && number(&v["code"]) != Some(200.0) && number(&v["code"]) != Some(0.0)) {
        return Err("GLM 未返回可用套餐额度，请检查套餐密钥和地区".into());
    }
    s.plan = "Coding Plan".into();
    let data = if v["data"].is_object() { &v["data"] } else { v };
    let rows = data["limits"].as_array().ok_or("GLM 额度响应缺少限制列表")?;
    if rows.len()>16{return Err("GLM 返回过多额度窗口，请检查响应版本".into());}
    for row in rows {
        let kind = row["type"].as_str().unwrap_or_default();
        if !matches!(kind, "TOKENS_LIMIT" | "CREDIT_LIMIT" | "TIME_LIMIT") { continue; }
        // percentage is consumed percent, not a raw token count. Never infer
        // a token balance or plan tier from it.
        let Some(p) = percent(&row["percentage"]) else { continue; };
        // The official plugin confirms percent semantics, but does not define
        // numeric unit codes or nextResetTime units. Do not guess a cadence or
        // timestamp from secondary implementations. Preserve distinct source types.
        let label = match kind {
            "TIME_LIMIT" => "工具额度",
            "CREDIT_LIMIT" => "积分额度",
            _ => "编程额度",
        };
        let w = window(label, None, 100.0 - p, None);
        if kind == "TIME_LIMIT" { s.extra.push(w); } else { s.windows.push(w); }
    }
    // Existing widgets render main windows only. A tools-only response does not
    // confirm a coding quota and must not yield a blank successful connection.
    if s.windows.is_empty() { return Err("GLM 未返回可识别的编程额度（工具额度不能代替编程额度）".into()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test] fn opencode_percentage_and_iso_reset() {
        let s = parse(PlanProvider::OpenCodeGo, &json!({"usage":{"rolling":{"percent":25,"resetsAt":"2030-01-01T00:00:00Z"},"weekly":{"percent":100},"monthly":{"percent":0}}})).unwrap();
        assert_eq!(s.windows.len(),3); assert_eq!(s.windows[0].remaining,75.0); assert_eq!(s.windows[0].reset,Some(1893456000)); assert_eq!(s.windows[1].remaining,0.0);
    }
    #[test] fn absent_invalid_or_error_data_never_becomes_zero() {
        for provider in [PlanProvider::OpenCodeGo, PlanProvider::MiniMaxChina, PlanProvider::GlmGlobal] {
            for v in [json!({}),json!({"error":{"message":"SECRET"}}),json!({"usage":{"rolling":{"percent":"NaN"}}})] { assert!(parse(provider,&v).is_err()); }
        }
        assert!(parse(PlanProvider::OpenCodeGo,&json!({"usage":{"rolling":{"percent":101}}})).is_err());
    }
    #[test] fn minimax_legacy_remaining_and_epoch_milliseconds() {
        let s = parse(PlanProvider::MiniMaxChina,&json!({"base_resp":{"status_code":0},"model_remains":[{"model_name":"MiniMax-M*","current_interval_total_count":100,"current_interval_usage_count":75,"start_time":1900000000000i64,"end_time":1900018000000i64}]})).unwrap();
        assert_eq!(s.windows[0].remaining,75.0); assert_eq!(s.windows[0].seconds,Some(18000)); assert_eq!(s.windows[0].reset,Some(1900018000));
    }
    #[test] fn minimax_prefers_explicit_percent_and_general_pool() {
        let s = parse(PlanProvider::MiniMaxGlobal,&json!({"model_remains":[{"model_name":"MiniMax-M*","current_interval_total_count":100,"current_interval_usage_count":90},{"model_name":"general","current_interval_total_count":0,"current_interval_usage_count":0,"current_interval_remaining_percent":40,"current_weekly_remaining_percent":80,"weekly_boost_permille":1500}]})).unwrap();
        assert_eq!(s.windows[0].remaining,40.0); assert_eq!(s.windows[1].remaining,120.0);
    }
    #[test] fn minimax_unknown_unlimited_and_zero_total_not_finite_quota() {
        for row in [json!({"model_name":"general","current_interval_total_count":0,"current_interval_usage_count":0}),json!({"model_name":"general","current_interval_status":3,"current_interval_remaining_percent":100}),json!({"model_name":"video","current_interval_remaining_percent":100})] {
            assert!(parse(PlanProvider::MiniMaxChina,&json!({"model_remains":[row]})).is_err());
        }
    }
    #[test] fn glm_used_percentage_windows_and_tools_are_separate() {
        let s = parse(PlanProvider::GlmChina,&json!({"success":true,"code":200,"data":{"limits":[{"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":20,"nextResetTime":1900000000000i64},{"type":"CREDIT_LIMIT","unit":6,"number":1,"percentage":75},{"type":"TIME_LIMIT","unit":5,"number":1,"percentage":50},{"type":"FUTURE_LIMIT","percentage":0}]}})).unwrap();
        assert_eq!(s.windows.len(),2); assert_eq!(s.extra.len(),1); assert_eq!(s.windows[0].remaining,80.0); assert_eq!(s.windows[0].reset,None); assert_eq!(s.windows[0].seconds,None); assert_eq!(s.windows[1].remaining,25.0);
        assert!(parse(PlanProvider::GlmChina,&json!({"data":{"limits":[{"type":"TOKENS_LIMIT"}]}})).is_err());
    }
    #[test] fn opencode_unknown_rolling_cadence_preserves_primary_order() {
        let s = parse(PlanProvider::OpenCodeGo, &json!({"usage":{"rolling":{"percent":10},"weekly":{"percent":20},"monthly":{"percent":30}}})).unwrap();
        assert_eq!(s.rings().iter().map(|w| w.label.as_str()).collect::<Vec<_>>(), vec!["滚动", "本周"]);
        assert!(s.windows.iter().all(|w| w.seconds.is_none()));
    }
    #[test] fn glm_tools_only_cannot_connect_as_blank_coding_account() {
        assert!(parse(PlanProvider::GlmGlobal, &json!({"data":{"limits":[{"type":"TIME_LIMIT","percentage":10}]}})).is_err());
    }
    #[test] fn authorization_regions_and_errors_are_fixed() {
        assert_eq!(PlanProvider::GlmChina.authorization("synthetic"),"synthetic");
        assert_eq!(PlanProvider::OpenCodeGo.authorization("synthetic"),"Bearer synthetic");
        assert!(PlanProvider::MiniMaxChina.endpoint().starts_with("https://api.minimax.cn/"));
        assert!(PlanProvider::MiniMaxGlobal.endpoint().starts_with("https://api.minimax.io/"));
        assert_eq!(status_error(302).1,0); assert_eq!(status_error(429).1,180);
        assert!(!parse(PlanProvider::MiniMaxChina,&json!({"base_resp":{"status_code":1004,"status_msg":"SECRET"}})).unwrap_err().contains("SECRET"));
    }
}
