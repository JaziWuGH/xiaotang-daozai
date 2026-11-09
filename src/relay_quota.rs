//! Read-only adapters for distinct relay protocols. Do not convert site quota to money.
//! Protocol references (reviewed 2026-11-08):
//! https://github.com/QuantumNous/new-api/blob/main/controller/token.go
//! https://github.com/songquanpeng/one-api/blob/main/controller/billing.go
//! https://github.com/Wei-Shaw/sub2api/blob/main/backend/internal/handler/gateway_handler.go
use super::{Snapshot, Window};
use serde_json::Value;
use std::io::Read;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { NewApi, OneApi, Sub2Api }
impl Kind {
    fn name(self) -> &'static str { match self { Self::NewApi => "New API", Self::OneApi => "One API", Self::Sub2Api => "Sub2API" } }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Amount {
    pub label: String,
    pub unit: String,
    pub remaining: Option<f64>,
    pub limit: Option<f64>,
    pub used: Option<f64>,
    pub unlimited: bool,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Quota {
    pub source: String,
    pub amounts: Vec<Amount>,
    /// Expiry is not a reset. Keep it out of Window.reset.
    pub expires_at: Option<i64>,
}
impl Quota {
    pub fn compact(&self) -> String {
        let limited=self.amounts.iter().filter(|a|!a.unlimited&&a.remaining.is_some()).min_by(|a,b|a.remaining.unwrap().total_cmp(&b.remaining.unwrap()));
        let Some(a) = limited.or_else(||self.amounts.first()) else { return "额度未知".into() };
        if a.unlimited { return format!("{}不限额", a.label.trim_end_matches("额度")) }
        match a.remaining { Some(n) => if self.amounts.len()>1{format!("{} {} · {}",number(n),a.unit,a.label)}else{format!("{} {}", number(n), a.unit)}, None => "额度未知".into() }
    }
    pub fn full(&self) -> String {
        let mut rows = vec![self.source.clone()];
        for a in &self.amounts {
            let value = if a.unlimited { "不限额".into() } else { a.remaining.map(|n|format!("剩余 {} {}",number(n),a.unit)).unwrap_or_else(||"剩余额度未知".into()) };
            let used = a.used.map(|n|format!("，已用 {} {}",number(n),a.unit)).unwrap_or_default();
            rows.push(format!("{}：{value}{used}",a.label));
        }
        if let Some(at) = self.expires_at.and_then(|t|chrono::DateTime::from_timestamp(t,0)) { rows.push(format!("到期 {} UTC",at.format("%Y-%m-%d %H:%M"))); }
        rows.join("\n")
    }
}
fn number(n:f64)->String {
    let raw = if n.abs() >= 1e12 || (n != 0.0 && n.abs() < 0.0001) { format!("{n:.3e}") } else { format!("{n:.4}") };
    if raw.contains('e') { raw } else { raw.trim_end_matches('0').trim_end_matches('.').to_owned() }
}

/// Accept the site root, preserving its reverse-proxy path prefix.
/// HTTPS only: localhost HTTP is deliberately not an implicit credential exception.
pub fn validate_base_url(raw:&str)->Result<String,String> {
    let raw=raw.trim();
    if raw.is_empty() || raw.chars().any(char::is_whitespace) || raw.contains('\\') { return Err("请输入有效的 HTTPS 站点地址".into()); }
    let url=url::Url::parse(raw).map_err(|_|"请输入完整的 HTTPS 站点地址")?;
    if url.scheme()!="https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("站点地址必须使用 HTTPS，且不能包含账号、查询参数或片段".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
/// Preserve the user's exact deployment path; never guess whether /v1 is a
/// tenant prefix or an API suffix. The UI asks for the site root, not API base.
pub fn normalize_base_url(raw:&str)->Result<String,String>{validate_base_url(raw)}

fn numeric(v:&Value)->Option<f64> { v.as_f64().filter(|n|n.is_finite()) }
fn required(v:&Value,key:&str)->Result<f64,String> { numeric(&v[key]).ok_or_else(||"站点返回的额度字段缺失或无效".into()) }
fn nonnegative(v:&Value,key:&str)->Result<f64,String> { required(v,key).and_then(|n|if n>=0.0{Ok(n)}else{Err("站点返回的额度字段无效".into())}) }
fn expiry(v:&Value)->Option<i64> {
    v.as_i64().filter(|n|*n>0).or_else(||v.as_str().and_then(|s|chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d|d.timestamp()).filter(|n|*n>0))
}
fn snapshot(kind:Kind,amounts:Vec<Amount>,expires_at:Option<i64>,windows:Vec<Window>)->Snapshot {
    Snapshot { plan:kind.name().into(), relay:Some(Quota{source:kind.name().into(),amounts,expires_at}), windows, at:chrono::Utc::now().timestamp(), ..Default::default() }
}
fn window(label:&str,limit:f64,remaining:f64,seconds:Option<i64>,reset:Option<i64>)->Option<Window> {
    (limit>0.0).then(||Window{label:label.into(),seconds,remaining:((remaining/limit).clamp(0.0,1.0)*100.0) as f32,reset})
}
fn amount(label:&str,unit:&str,remaining:Option<f64>,limit:Option<f64>,used:Option<f64>,unlimited:bool)->Amount {
    Amount{label:label.into(),unit:unit.into(),remaining,limit,used,unlimited}
}

pub fn parse_new_api(v:&Value)->Result<Snapshot,String> {
    if v["code"]!=true || !v["error"].is_null() { return Err("站点未返回有效的令牌额度".into()); }
    let d=&v["data"];
    if d["object"]!="token_usage" { return Err("该站点版本不支持 New API 令牌额度接口".into()); }
    let unlimited=d["unlimited_quota"].as_bool().ok_or("站点未返回令牌限额状态")?;
    let used=nonnegative(d,"total_used")?;
    let (remaining,limit)=if unlimited {(None,None)}else{(Some(required(d,"total_available")?),Some(nonnegative(d,"total_granted")?))};
    let windows=match (remaining,limit) { (Some(r),Some(l))=>window("令牌额度",l,r,None,None).into_iter().collect(),_=>vec![] };
    let mut amounts=vec![amount("令牌额度","站点配额",remaining,limit,Some(used),unlimited)];
    if d["user_quota"].is_object() {
        let u=&d["user_quota"];
        amounts.push(amount("账户额度","站点配额",Some(required(u,"total_available")?),None,Some(nonnegative(u,"total_used")?),false));
    }
    Ok(snapshot(Kind::NewApi,amounts,expiry(&d["expires_at"]),windows))
}

pub fn parse_one_api(subscription:&Value,usage:&Value)->Result<Snapshot,String> {
    if !subscription["error"].is_null() || !usage["error"].is_null() || subscription["object"]!="billing_subscription" || usage["object"]!="list" {
        return Err("该站点未返回有效的 One API 计费额度".into());
    }
    let limit=nonnegative(subscription,"hard_limit_usd")?;
    // This endpoint multiplies usage by 100 even when DisplayInCurrencyEnabled is false.
    let used=nonnegative(usage,"total_usage")?/100.0;
    // Official One API uses this sentinel for unlimited, but cannot distinguish it
    // from a real numeric limit. Never present a fabricated balance or percentage.
    let ambiguous=limit==100_000_000.0;
    let remaining=if ambiguous {None}else{Some(limit-used)};
    let windows=remaining.and_then(|r|window("计费额度",limit,r,None,None)).into_iter().collect();
    Ok(snapshot(Kind::OneApi,vec![amount("计费额度（站点配置单位）","站点单位",remaining,if ambiguous{None}else{Some(limit)},Some(used),false)],expiry(&subscription["access_until"]),windows))
}

pub fn parse_sub2api(v:&Value)->Result<Snapshot,String> {
    if v["isValid"]!=true || !v["error"].is_null() { return Err("站点未返回有效的 Sub2API 额度".into()); }
    let mut amounts=vec![];let mut windows=vec![];
    match v["mode"].as_str() {
        Some("quota_limited")=>{
            if !v["quota"].is_null() {
                let q=&v["quota"];
                if q["unit"]!="USD" {return Err("站点返回了未知的额度单位".into());}
                let limit=nonnegative(q,"limit")?;let used=nonnegative(q,"used")?;let remaining=required(q,"remaining")?;
                amounts.push(amount("API Key 总额度","USD",Some(remaining),Some(limit),Some(used),false));
                if let Some(w)=window("总额度",limit,remaining,None,None){windows.push(w);}
            }
            if let Some(rows)=v["rate_limits"].as_array() {
                if rows.len()>3 {return Err("站点返回了重复或过多的限额周期".into());}
                let mut seen=std::collections::HashSet::new();
                for row in rows {
                    let (label,seconds)=match row["window"].as_str(){Some("5h")=>("5 小时",18000),Some("1d")=>("每日",86400),Some("7d")=>("每周",604800),_=>return Err("站点返回了未知的限额周期".into())};
                    if !seen.insert(seconds){return Err("站点返回了重复的限额周期".into());}
                    let limit=nonnegative(row,"limit")?;let used=nonnegative(row,"used")?;let remaining=required(row,"remaining")?;
                    // Official rate_limits fields are USD spending limits, not request counts.
                    amounts.push(amount(label,"USD",Some(remaining),Some(limit),Some(used),false));
                    if let Some(w)=window(label,limit,remaining,Some(seconds),expiry(&row["reset_at"])){windows.push(w);}
                }
            }
        }
        Some("unrestricted")=>{
            if v["unit"]!="USD" {return Err("站点返回了未知的额度单位".into());}
            if v["subscription"].is_object() {
                let sub=&v["subscription"];
                for (prefix,label,seconds) in [("daily","每日",86400),("weekly","每周",604800),("monthly","每月",2592000)] {
                    let field=format!("{prefix}_limit_usd");
                    // Null / absent is not an exhausted window, and does not prove unlimited.
                    if sub[&field].is_null(){continue;}
                    let limit=nonnegative(sub,&field)?;
                    if limit==0.0 {continue;}
                    let used=nonnegative(sub,&format!("{prefix}_usage_usd"))?;let remaining=(limit-used).max(0.0);
                    amounts.push(amount(label,"USD",Some(remaining),Some(limit),Some(used),false));
                    if let Some(w)=window(label,limit,remaining,Some(seconds),None){windows.push(w);}
                }
                if amounts.is_empty() {
                    // -1 is an explicit unlimited signal only for this subscription mode.
                    if numeric(&v["remaining"])==Some(-1.0) {
                        amounts.push(amount("订阅额度","USD",None,None,None,true));
                    } else { return Err("站点未返回可用的订阅限额明细".into()); }
                }
            } else if !v["balance"].is_null() {
                amounts.push(amount("账户钱包","USD",Some(required(v,"balance")?),None,None,false));
            } else {
                return Err("站点未返回订阅限额或钱包余额，请检查站点版本与权限".into());
            }
        }
        _=>return Err("该站点版本不支持当前 Sub2API 额度协议".into()),
    }
    if amounts.is_empty(){return Err("站点未返回可用的额度明细".into());}
    let expires=expiry(&v["expires_at"]).or_else(||expiry(&v["subscription"]["expires_at"]));
    Ok(snapshot(Kind::Sub2Api,amounts,expires,windows))
}

fn response_error(status:u16)->(String,u64) {
    match status {
        401=>("站点认证失败（401），请核对 API Key".into(),0),
        403=>("站点拒绝额度查询（403），请检查权限".into(),60),
        404=>("站点没有该额度接口（404），请核对类型、地址和版本".into(),60),
        429=>("站点请求过于频繁，请稍后重试".into(),180),
        300..=399=>("站点要求重定向；为保护 API Key 已停止，请填写最终 HTTPS 地址".into(),0),
        _=>(format!("站点额度服务暂不可用（{status}）"),30),
    }
}
fn get(agent:&ureq::Agent,url:&str,key:&str)->Result<Value,(String,u64)> {
    // Callers must supply the application's timeout-bounded, redirects(0) agent.
    let response=agent.get(url).set("Authorization",&format!("Bearer {key}")).set("Accept","application/json").call().map_err(|e|match e{ureq::Error::Status(s,_)=>response_error(s),_=>("无法连接额度站点，请检查网络与地址".into(),30)})?;
    if !(200..300).contains(&response.status()){return Err(response_error(response.status()));}
    const MAX:u64=1024*1024;
    let mut raw=Vec::new();
    response.into_reader().take(MAX+1).read_to_end(&mut raw).map_err(|_|("读取站点额度失败".into(),30))?;
    if raw.len() as u64>MAX{return Err(("站点额度响应过大".into(),60));}
    serde_json::from_slice(&raw).map_err(|_|("站点未返回有效的 JSON 额度响应".into(),30))
}
/// No fallback requests to other protocols or hosts: type selection is explicit.
pub fn request_cancellable(agent:&ureq::Agent,kind:Kind,base_url:&str,key:&str,cancelled:impl Fn()->bool)->Result<Snapshot,(String,u64)> {
    if cancelled(){return Err(("已取消连接".into(),0));}
    let base=validate_base_url(base_url).map_err(|e|(e,0))?;
    let key=key.trim();
    if key.is_empty() || key.len()>8192 || key.chars().any(|c|c.is_whitespace()||c.is_control()) {return Err(("请输入有效的 API Key".into(),0));}
    let result=match kind {
        Kind::NewApi=>parse_new_api(&get(agent,&format!("{base}/api/usage/token/"),key)?),
        Kind::OneApi=>{
            let subscription=get(agent,&format!("{base}/dashboard/billing/subscription"),key)?;
            if cancelled(){return Err(("已取消连接".into(),0));}
            let usage=get(agent,&format!("{base}/dashboard/billing/usage"),key)?;
            parse_one_api(&subscription,&usage)
        }
        Kind::Sub2Api=>parse_sub2api(&get(agent,&format!("{base}/v1/usage"),key)?),
    };
    result.map_err(|e|(e,60))
}

#[cfg(test)]
mod tests {
    use super::*;use serde_json::json;
    #[test] fn cancelled_validation_makes_no_request(){
        let agent=ureq::AgentBuilder::new().redirects(0).build();
        assert_eq!(request_cancellable(&agent,Kind::OneApi,"https://must-not-resolve.invalid","synthetic",||true).unwrap_err().0,"已取消连接");
    }
    #[test] fn persisted_prefix_is_not_normalized_again(){
        let base=normalize_base_url("https://relay.example/v1/v1").unwrap();assert_eq!(base,"https://relay.example/v1/v1");
        assert_eq!(validate_base_url(&base).unwrap(),base);
    }
    #[test] fn compact_names_the_exhausted_limiting_window(){
        let q=Quota{source:"Sub2API".into(),expires_at:None,amounts:vec![amount("每日","USD",Some(10.0),Some(10.0),Some(0.0),false),amount("每周","USD",Some(0.0),Some(20.0),Some(20.0),false)]};
        assert_eq!(q.compact(),"0 USD · 每周");
    }
    #[test] fn bases_are_https_and_never_contain_credentials_or_parameters(){
        for bad in ["http://example.com","http://localhost:8080","https://u:p@example.com","https://example.com?key=secret","https://example.com/#secret","file:///tmp/a","https://example.com\\evil","https://exa mple.com"] {assert!(normalize_base_url(bad).is_err(),"{bad}");}
        assert_eq!(normalize_base_url(" https://relay.example/prefix/v1/ ").unwrap(),"https://relay.example/prefix/v1");
        assert_eq!(normalize_base_url("https://relay.example/").unwrap(),"https://relay.example");
    }
    fn new_fixture()->Value {json!({"code":true,"data":{"object":"token_usage","total_granted":500000,"total_available":250000,"total_used":250000,"unlimited_quota":false,"expires_at":1900000000}})}
    #[test] fn new_api_preserves_units_and_expiry_is_not_reset(){
        let s=parse_new_api(&new_fixture()).unwrap();assert_eq!(s.windows[0].remaining,50.0);assert_eq!(s.windows[0].reset,None);
        let q=s.relay.unwrap();assert_eq!(q.amounts[0].unit,"站点配额");assert!(!q.full().contains('$'));assert_eq!(q.expires_at,Some(1900000000));
    }
    #[test] fn missing_zero_and_unlimited_are_distinct(){
        let mut v=new_fixture();v["data"]["total_available"]=Value::Null;assert!(parse_new_api(&v).is_err());
        v["data"]["total_available"]=json!(0);assert_eq!(parse_new_api(&v).unwrap().windows[0].remaining,0.0);
        v["data"]["unlimited_quota"]=json!(true);let s=parse_new_api(&v).unwrap();assert!(s.windows.is_empty());let q=s.relay.unwrap();assert!(q.amounts[0].unlimited);assert_eq!(q.amounts[0].remaining,None);
        v["code"]=json!(false);assert!(parse_new_api(&v).is_err());
    }
    #[test] fn one_api_has_configurable_units_and_cent_scaled_usage(){
        let sub=json!({"object":"billing_subscription","hard_limit_usd":100});let usage=json!({"object":"list","total_usage":2500});
        let s=parse_one_api(&sub,&usage).unwrap();assert_eq!(s.windows[0].remaining,75.0);let q=s.relay.unwrap();assert_eq!(q.amounts[0].remaining,Some(75.0));assert_eq!(q.amounts[0].unit,"站点单位");
        let mut sentinel=sub.clone();sentinel["hard_limit_usd"]=json!(100000000);let s=parse_one_api(&sentinel,&usage).unwrap();assert!(s.windows.is_empty());assert_eq!(s.relay.unwrap().amounts[0].remaining,None);
        assert!(parse_one_api(&sub,&json!({"object":"list"})).is_err());assert!(parse_one_api(&json!({"error":{"message":"secret"}}),&usage).is_err());
    }
    #[test] fn sub2api_wallet_does_not_become_a_percentage(){
        let s=parse_sub2api(&json!({"mode":"unrestricted","isValid":true,"unit":"USD","balance":0,"remaining":0})).unwrap();assert!(s.windows.is_empty());assert_eq!(s.relay.unwrap().compact(),"0 USD");
        assert!(parse_sub2api(&json!({"mode":"unrestricted","isValid":true,"unit":"USD"})).is_err());
    }
    #[test] fn sub2api_retains_all_key_windows_and_explicit_reset(){
        let s=parse_sub2api(&json!({"mode":"quota_limited","isValid":true,"quota":{"limit":10,"used":2,"remaining":8,"unit":"USD"},"rate_limits":[{"window":"5h","limit":5,"used":1,"remaining":4,"reset_at":"2030-01-01T00:00:00Z"},{"window":"1d","limit":20,"used":0,"remaining":20}]})).unwrap();
        assert_eq!(s.windows.len(),3);assert_eq!(s.windows[1].remaining,80.0);assert!(s.windows[1].reset.is_some());assert_eq!(s.windows[2].reset,None);
        assert!(parse_sub2api(&json!({"mode":"quota_limited","isValid":true,"quota":{"limit":10,"used":2,"unit":"USD"}})).is_err());
    }
    #[test] fn duplicate_sub2api_periods_are_rejected(){
        let row=json!({"window":"5h","limit":5,"used":1,"remaining":4});
        assert!(parse_sub2api(&json!({"mode":"quota_limited","isValid":true,"rate_limits":[row.clone(),row]})).is_err());
    }
    #[test] fn sub2api_subscription_missing_and_unlimited_do_not_become_zero(){
        let s=parse_sub2api(&json!({"mode":"unrestricted","isValid":true,"unit":"USD","remaining":3,"subscription":{"daily_limit_usd":10,"daily_usage_usd":7,"weekly_limit_usd":null}})).unwrap();assert_eq!(s.windows.len(),1);assert_eq!(s.windows[0].remaining,30.0);
        let s=parse_sub2api(&json!({"mode":"unrestricted","isValid":true,"unit":"USD","remaining":-1,"subscription":{"daily_limit_usd":null}})).unwrap();assert!(s.relay.unwrap().amounts[0].unlimited);
        assert!(parse_sub2api(&json!({"mode":"unrestricted","isValid":true,"unit":"USD","remaining":0,"subscription":{}})).is_err());
    }
    #[test] fn finite_zero_limit_is_not_unlimited(){
        let s=parse_one_api(&json!({"object":"billing_subscription","hard_limit_usd":0}),&json!({"object":"list","total_usage":0})).unwrap();
        assert!(s.windows.is_empty());let q=s.relay.unwrap();assert_eq!(q.amounts[0].remaining,Some(0.0));assert!(!q.amounts[0].unlimited);
        let mut v=new_fixture();v["data"]["total_granted"]=json!(0);v["data"]["total_available"]=json!(0);v["data"]["total_used"]=json!(0);
        let s=parse_new_api(&v).unwrap();assert!(s.windows.is_empty());assert!(!s.relay.unwrap().amounts[0].unlimited);
    }
    #[test] fn tiny_positive_amounts_are_not_displayed_as_zero(){assert_ne!(number(0.000001),"0");assert_eq!(number(0.0),"0");}
    #[test] fn quota_cache_roundtrip_preserves_unknown_and_unlimited(){
        let mut v=new_fixture();v["data"]["unlimited_quota"]=json!(true);
        let q=parse_new_api(&v).unwrap().relay.unwrap();let raw=serde_json::to_string(&q).unwrap();
        let restored:Quota=serde_json::from_str(&raw).unwrap();assert_eq!(restored.amounts[0].remaining,None);assert!(restored.amounts[0].unlimited);assert_eq!(restored.compact(),"令牌不限额");
    }
    #[test] fn untrusted_errors_and_nonfinite_numbers_are_not_exposed(){
        let v=json!({"code":false,"message":"sk-secret https://private.example"});let error=parse_new_api(&v).unwrap_err();assert!(!error.contains("secret"));assert!(!error.contains("private"));
        let mut v=new_fixture();v["data"]["total_used"]=json!("NaN");assert!(parse_new_api(&v).is_err());
        assert_eq!(response_error(429).1,180);assert!(response_error(302).0.contains("重定向"));
    }
}
