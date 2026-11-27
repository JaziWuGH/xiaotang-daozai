use super::*;
use crate::calendar::{Config,Event,Provider};

#[cfg(windows)]
#[test]
fn caldav_login_page_blocks_saving_but_empty_calendar_can_be_saved() {
    use crate::calendar::{compatibility_tests::mock_caldav_connection, load_configs, secrets};

    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cases = [
        (200, "text/html", "<!DOCTYPE html><html><body><form>登录</form></body></html>", false),
        (200, "application/xml", r#"<html><body><multistatus xmlns="DAV:"/></body></html>"#, false),
        (200, "application/xml", "", false),
        (200, "application/xml", r#"<multistatus xmlns="urn:other"/>"#, false),
        (207, "application/xml", r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"/>"#, true),
        (200, "application/xml", r#"<multistatus xmlns="DAV:"></multistatus>"#, true),
    ];
    for (status, content_type, body, expected_saved) in cases {
        let mut h = H::new();
        let cfg = Config {
            provider: Provider::Custom,
            username: format!("caldav-xml-test-{}-{}@example.invalid", std::process::id(), rand_id()),
            href: None,
            server: None,
        };
        h.app.calendar.test_password = "test-password".into();
        h.app.calendar.testing = true;
        let result = mock_caldav_connection(cfg.provider, &cfg.username, status, content_type, body);
        h.app.calendar_event(result);
        let saved = load_configs();
        let password = secrets::load(&cfg);
        secrets::delete(&cfg);

        assert_eq!(h.app.calendar.connected, expected_saved, "{status} {body}");
        assert_eq!(h.app.calendar.note_error, !expected_saved);
        if expected_saved {
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0].username, cfg.username);
            assert_eq!(saved[0].href, Some(format!("{}calendar/", saved[0].base())));
            assert_eq!(password.as_deref(), Ok("test-password"));
            assert!(h.app.calendar.note.contains("0 个日程"));
        } else {
            assert!(saved.is_empty());
            assert!(password.is_err());
            assert!(h.app.calendar.note.contains("服务器未返回有效的 CalDAV"));
        }
    }
}

#[cfg(windows)]
#[test]
fn calendar_connection_with_parse_warning_is_saved_and_sync_keeps_cached_events() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = H::new();
    let config = Config {
        provider: Provider::Custom,
        username: format!("calendar-test-{}-{}@example.invalid", std::process::id(), rand_id()),
        href: Some("https://calendar.example.invalid/dav/calendar/".into()),
        server: Some("https://calendar.example.invalid/dav/".into()),
    };
    let warning = "1 条日程无法解析".to_string();
    h.app.calendar.test_password = "test-password".into();
    h.app.calendar.testing = true;
    h.app.calendar_event(Event::Test { ok: true, message: warning.clone(), count: 0, config: Some(config.clone()) });
    let saved = crate::calendar::load_configs();
    let password = crate::calendar::secrets::load(&config);
    crate::calendar::secrets::delete(&config);
    assert_eq!(saved, [config.clone()]);
    assert_eq!(password.as_deref(), Ok("test-password"));
    assert!(h.app.calendar.connected && h.app.calendar.feeds[0].connected);
    assert!(!h.app.calendar.note_error);
    assert_eq!(h.app.calendar.warning, warning);
    assert_eq!(h.app.calendar.feeds[0].warning, warning);
    assert!(h.app.calendar.last_sync.is_empty());

    let events = vec![crate::mail::Meeting {
        uid: "cached".into(), title: "已同步的日程".into(), start: 1, end: None, all_day: false, at: String::new(),
    }];
    h.app.calendar_event(Event::Synced { config: config.clone(), events, at: "12:00".into() });
    h.app.calendar_event(Event::Failed { config: config.clone(), message: "上次网络连接失败".into() });
    h.app.calendar_event(Event::Warning { config, month: None, message: warning.clone() });
    assert!(h.app.calendar.connected && h.app.calendar.feeds[0].connected);
    assert!(h.app.calendar.error.is_empty());
    assert_eq!(h.app.calendar.events[0].uid, "cached");
    assert_eq!(h.app.calendar.last_sync, "12:00");
    assert_eq!(h.app.calendar.warning, warning);
}

#[test]
fn ics_import_retains_good_records_and_distinguishes_duplicates_and_bad_records(){
    let _guard=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());let mut h=H::new();
    let at=(chrono::Utc::now()+chrono::Duration::days(1)).format("%Y%m%dT%H%M%SZ").to_string();
    let ics=format!("BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:{at}\nSUMMARY:同一时间的面试甲\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART:{at}\nSUMMARY:同一时间的面试乙\nEND:VEVENT\nBEGIN:VEVENT\nUID:bad\nDTSTART;TZID=Unsupported/Zone:20261112T100000\nEND:VEVENT\nBEGIN:VEVENT\nUID:old\nDTSTART:20000101T000000Z\nEND:VEVENT\nBEGIN:VEVENT\nUID:future\nDTSTART:21000101T000000Z\nEND:VEVENT\nBEGIN:VTODO\nSUMMARY:待办\nEND:VTODO\nEND:VCALENDAR");
    let files=vec![("mock.ics".into(),ics)];let note=h.app.import_ics(&files);
    assert_eq!(h.app.calendar.local.len(),2,"缺少 UID 的同一时刻不同日程不能互相覆盖");
    for phrase in ["已导入 2 个","1 条日程无法解析","2 个超出导入范围","1 条待办"]{assert!(note.contains(phrase),"{note}");}
    let ids=h.app.calendar.local.iter().map(|entry|entry.event.uid.clone()).collect::<Vec<_>>();assert_ne!(ids[0],ids[1]);
    let note=h.app.import_ics(&files);assert_eq!(h.app.calendar.local.len(),2);assert!(note.contains("2 个已存在已跳过"));
    let legacy=format!("ics--{}",h.app.calendar.local[0].event.start);h.app.calendar.local[0].event.uid=legacy.clone();
    h.app.import_ics(&files);assert_eq!(h.app.calendar.local.len(),2);assert_eq!(h.app.calendar.local[0].event.uid,legacy,"已有本机记录应原样保留");
}

#[test]
fn ics_import_reports_the_capacity_limit_without_calling_new_records_duplicates(){
    let _guard=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());let mut h=H::new();
    let at=(chrono::Utc::now()+chrono::Duration::days(1)).format("%Y%m%dT%H%M%SZ").to_string();
    let first=format!("BEGIN:VEVENT\nUID:existing\nDTSTART:{at}\nEND:VEVENT");h.app.import_ics(&[("first.ics".into(),first)]);
    let entry=h.app.calendar.local[0].clone();h.app.calendar.local=(0..5000).map(|i|{let mut entry=entry.clone();entry.event.uid=format!("existing-{i}");entry}).collect();
    let new=format!("BEGIN:VEVENT\nUID:new\nDTSTART:{at}\nEND:VEVENT");let note=h.app.import_ics(&[("new.ics".into(),new)]);
    assert_eq!(h.app.calendar.local.len(),5000);assert!(note.contains("5000"));assert!(note.contains("1 个未导入"));assert!(!note.contains("已存在"));
}

#[test]
fn calendar_partial_warnings_are_scoped_and_clear_after_a_complete_refresh(){
    let _guard=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());let mut h=H::new();
    let config=Config{provider:Provider::DingTalk,username:"mock".into(),href:None,server:None};
    h.app.calendar.config=Some(config.clone());let month=h.app.calendar.view.month;
    h.app.calendar_event(Event::MonthLoaded{config:config.clone(),month,result:Ok(vec![])});
    let message="1 处日历读取失败；读取日程失败（403）".to_string();
    h.app.calendar_event(Event::Warning{config:config.clone(),month:Some(month),message:message.clone()});
    assert_eq!(h.app.calendar.view.warning,message);assert!(h.app.calendar.view.error.is_empty());
    let other=Config{provider:Provider::Feishu,..config.clone()};
    h.app.calendar_event(Event::Warning{config:other,month:Some(month),message:"其他账号".into()});
    h.app.calendar_event(Event::Warning{config:config.clone(),month:Some(month+chrono::Duration::days(32)),message:"旧月份".into()});
    assert_eq!(h.app.calendar.view.warning,message);
    h.app.show_page(crate::app::CALENDAR_PAGE);h.hover(H::local(200.0,12.0),1.0);
    assert!(h.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t)if t.galley.text()=="部分同步")));
    assert!(h.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t)if t.galley.text()=="部分日程未同步")));
    for shape in &h.shapes{if let egui::Shape::Text(text)=&shape.shape{if ["部分同步","部分日程未同步"].contains(&text.galley.text()){assert!(shape.clip_rect.contains_rect(text.visual_bounding_rect()));}}}
    h.app.calendar_event(Event::MonthLoaded{config:config.clone(),month,result:Ok(vec![])});
    h.app.calendar_event(Event::Warning{config,month:Some(month),message:String::new()});assert!(h.app.calendar.view.warning.is_empty());
}

#[test]
#[ignore="按需导出日历部分同步提示的实际样式"]
fn calendar_partial_sync_preview(){
    let _guard=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());
    let destination=std::path::PathBuf::from(std::env::var_os("DAOZAI_CALENDAR_PREVIEW").expect("需指定预览目录"));std::fs::create_dir_all(&destination).unwrap();
    for empty in [false,true]{
        let mut h=H::new();let config=Config{provider:Provider::DingTalk,username:"mock".into(),href:None,server:None};
        h.app.calendar.config=Some(config.clone());let month=h.app.calendar.view.month;
        let start=(chrono::Local::now()+chrono::Duration::minutes(30)).timestamp();
        let events=if empty{vec![]}else{vec![crate::mail::Meeting{uid:"preview".into(),title:"面试安排".into(),start,end:Some(start+1800),all_day:false,at:"面试时间".into()}]};
        h.app.calendar_event(Event::MonthLoaded{config:config.clone(),month,result:Ok(events)});
        h.app.calendar_event(Event::Warning{config,month:Some(month),message:"1 处日历读取失败；日历服务器拒绝访问（403）".into()});h.app.show_page(crate::app::CALENDAR_PAGE);
        let mut raster=feature_raster();let day=chrono::Local::now().date_naive().to_string();
        shoot_feature(&mut h,&mut raster,&destination,if empty{"calendar-partial-empty"}else{"calendar-partial-events"},100,Some(H::local(200.0,12.0)),&day);
    }
}
