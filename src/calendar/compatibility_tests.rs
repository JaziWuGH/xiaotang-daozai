use super::*;
use std::{io::{Read,Write},net::TcpListener};

fn read_request(socket:&mut std::net::TcpStream)->String{
    socket.set_nonblocking(false).unwrap();socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut bytes=Vec::new();loop{let mut buffer=[0;4096];let n=socket.read(&mut buffer).unwrap();if n==0{break;}bytes.extend_from_slice(&buffer[..n]);
        let text=String::from_utf8_lossy(&bytes);if let Some(end)=text.find("\r\n\r\n"){
            let length=text[..end].lines().find_map(|line|line.split_once(':').filter(|(key,_)|key.eq_ignore_ascii_case("Content-Length")).and_then(|(_,value)|value.trim().parse::<usize>().ok())).unwrap_or(0);
            if bytes.len()>=end+4+length{break;}
        }
    }String::from_utf8(bytes).unwrap()
}

fn mock_fetch(provider:Provider,nested:bool,broken:bool)->Result<Fetched,Failure>{
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
    let base=format!("http://{}/",listener.local_addr().unwrap());let(stop,done)=channel();
    let worker=std::thread::spawn(move||{
        let collection=|path:&str|format!("<d:response><d:href>{path}</d:href><d:propstat><d:prop><d:resourcetype><c:calendar/></d:resourcetype><c:supported-calendar-component-set><c:comp name=\"VEVENT\"/></c:supported-calendar-component-set></d:prop></d:propstat></d:response>");
        while done.try_recv().is_err(){
            let Ok((mut socket,_))=listener.accept()else{std::thread::sleep(Duration::from_millis(2));continue;};
            let request=read_request(&mut socket);let mut words=request.split_whitespace();let method=words.next().unwrap();let path=words.next().unwrap();
            let (status,body)=if method=="PROPFIND" && path=="/"{
                let home=if nested{"<d:response><d:href>/</d:href><d:propstat><d:prop><c:calendar-home-set><d:href>/home/</d:href></c:calendar-home-set></d:prop></d:propstat></d:response>"}else{""};
                (207,format!("{home}{}{}",collection("/holiday/"),if nested{String::new()}else{collection("/work/")}))
            }else if method=="PROPFIND" && path=="/home/"{(207,collection("/work/"))}
            else if method=="REPORT" && path=="/work/" && broken{(403,String::new())}
            else if method=="REPORT"{
                let ics=if path=="/holiday/"{"BEGIN:VEVENT\nUID:holiday\nDTSTART;VALUE=DATE:20261122\nSUMMARY:节气\nEND:VEVENT"}
                    else{"BEGIN:VEVENT\nUID:work\nDTSTART;TZID=Asia/Shanghai:20261112T100000\nSUMMARY:应用面试\nX-APP-ID:mock\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:test@example.invalid\nEND:VEVENT"};
                (207,format!("<d:response><d:href>{path}one.ics</d:href><c:calendar-data>BEGIN:VCALENDAR\n{ics}\nEND:VCALENDAR</c:calendar-data></d:response>"))
            }else{(404,String::new())};
            let body=format!("<d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\">{body}</d:multistatus>");
            write!(socket,"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let cfg=Config{provider,username:"mock".into(),href:None,server:Some(base)};
    let result=fetch_month(&cfg,"mock","2026-11-01".parse().unwrap());stop.send(()).unwrap();worker.join().unwrap();result
}

pub(crate) fn mock_caldav_connection(
    provider: Provider, username: &str, status: u16, content_type: &str, report: &str,
) -> Event {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let report = report.to_owned();
    let content_type = content_type.to_owned();
    let worker = std::thread::spawn(move || {
        let discovery = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/calendar/</d:href><d:resourcetype><c:calendar/></d:resourcetype></d:response></d:multistatus>"#;
        for (method, code, mime, body) in [("PROPFIND", 207, "application/xml", discovery), ("REPORT", status, &content_type, &report)] {
            let (mut socket, _) = listener.accept().unwrap();
            assert!(read_request(&mut socket).starts_with(method));
            write!(socket, "HTTP/1.1 {code} Test\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let service = Service::start();
    let cfg = Config { provider, username: username.into(), href: None, server: Some(base) };
    service.test(cfg, "test-password".into());
    let result = service.ev.recv_timeout(Duration::from_secs(3)).unwrap();
    worker.join().unwrap();
    result
}

#[test]
fn caldav_connection_test_accepts_parse_errors_but_requires_read_access() {
    let invalid = "BEGIN:VEVENT\nUID:bad\nDTSTART;TZID=Unsupported/Zone:20261112T100000\nEND:VEVENT";
    let start = (chrono::Utc::now() + chrono::Duration::minutes(30)).format("%Y%m%dT%H%M%SZ");
    let mixed = format!("{invalid}\nBEGIN:VEVENT\nUID:good\nDTSTART:{start}\nEND:VEVENT");
    for provider in [Provider::DingTalk, Provider::Feishu, Provider::ICloud, Provider::Custom] {
        for (status, ics, count) in [(207, invalid, 0), (207, mixed.as_str(), 1), (207, "", 0), (401, "", 0), (403, "", 0)] {
            let data = if ics.is_empty() { String::new() } else { format!("<d:response><d:href>/calendar/1.ics</d:href><c:calendar-data>{ics}</c:calendar-data></d:response>") };
            let xml = format!(r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">{data}</d:multistatus>"#);
            let result = mock_caldav_connection(provider, "mock", status, "application/xml", &xml);
            let Event::Test { ok, message, count: actual, config } = result else { panic!("应返回连接测试结果"); };
            assert_eq!(ok, status == 207, "{provider:?} {status} {message}");
            assert_eq!(actual, count);
            assert_eq!(config.is_some(), ok);
            if let Some(cfg) = config { assert_eq!(cfg.href, Some(format!("{}calendar/", cfg.base()))); }
            if status != 207 { assert!(message.contains(&status.to_string())); }
            else { assert_eq!(message.contains("1 条日程无法解析"), !ics.is_empty()); }
        }
    }
}

#[test]
fn discovery_keeps_root_events_and_follows_calendar_home_for_every_caldav_provider(){
    for provider in [Provider::DingTalk,Provider::Feishu,Provider::ICloud,Provider::Custom]{
        let result=mock_fetch(provider,true,false).unwrap();
        assert_eq!(result.events.len(),2,"{provider:?} 不能因找到节气日历而漏掉主目录中的应用日历");
    }
}

#[test]
fn malformed_single_event_keeps_other_valid_events(){
    let ics="BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:bad\nDTSTART;TZID=Unsupported/Zone:20261112T100000\nEND:VEVENT\nBEGIN:VEVENT\nUID:good\nDTSTART:20261112T020000Z\nSUMMARY:有效面试\nEND:VEVENT\nEND:VCALENDAR";
    let events=events_from_ics(ics,"2026-11-01T00:00:00Z".parse().unwrap(),"2026-12-01T00:00:00Z".parse().unwrap()).unwrap();
    assert_eq!(events.len(),1);assert_eq!(events[0].uid,"good");
}

#[test]
fn external_ical_properties_and_components_are_case_insensitive(){
    let ics="\u{feff}begin:vcalendar\nversion:2.0\nprodid:-//mock app//calendar//cn\nmethod:request\nbegin:vevent\nuid:external\nsummary:第三方面试\ndtstart;tzid=\"Asia/Shanghai\":20261112T100000\nattendee;partstat=NEEDS-ACTION:mailto:test@example.invalid\nx-app:mock\nend:vevent\nend:vcalendar";
    let events=events_from_ics(ics,"2026-11-01T00:00:00Z".parse().unwrap(),"2026-12-01T00:00:00Z".parse().unwrap()).unwrap();
    assert_eq!(events.len(),1);assert_eq!(events[0].title,"第三方面试");
}

#[test]
fn partial_collection_failure_is_visible_for_every_caldav_provider(){
    for provider in [Provider::DingTalk,Provider::Feishu,Provider::ICloud,Provider::Custom]{
        let fetched=mock_fetch(provider,false,true).unwrap();
        assert_eq!(fetched.events.len(),1);assert_eq!(fetched.report.calendars,2);assert_eq!(fetched.report.failed,1);
        assert!(fetched.report.warning().contains("403"));assert!(fetched.report.warning().contains("读取失败"));
    }
}

#[test]
fn import_report_explains_invalid_outside_cancelled_and_non_event_records(){
    let ics="BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:bad\nDTSTART;TZID=Unsupported/Zone:20261112T100000\nEND:VEVENT\nBEGIN:VEVENT\nUID:past\nDTSTART:20000101T000000Z\nEND:VEVENT\nBEGIN:VEVENT\nUID:cancelled\nDTSTART:20261112T000000Z\nSTATUS:CANCELLED\nEND:VEVENT\nBEGIN:VTODO\nUID:task\nSUMMARY:不是日程\nEND:VTODO\nBEGIN:VEVENT\nUID:good\nDTSTART:20261112T020000Z\nEND:VEVENT\nEND:VCALENDAR";
    let parsed=inspect_ics(ics,"2026-11-01T00:00:00Z".parse().unwrap(),"2026-12-01T00:00:00Z".parse().unwrap()).unwrap();
    assert_eq!(parsed.events.len(),1);assert_eq!(parsed.report.received,4);
    assert_eq!((parsed.report.invalid,parsed.report.outside,parsed.report.cancelled,parsed.report.unsupported),(1,1,1,1));
}

#[test]
fn malformed_recurrence_override_does_not_restore_the_unmodified_series(){
    for recurrence in ["RECURRENCE-ID;RANGE=THISANDFUTURE:20261112T020000Z","RECURRENCE-ID;TZID=Unsupported/Zone:20261112T100000"]{
        let ics=format!("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:series\nDTSTART:20261111T020000Z\nRRULE:FREQ=DAILY;COUNT=3\nEND:VEVENT\nBEGIN:VEVENT\nUID:series\n{recurrence}\nDTSTART:20261112T040000Z\nEND:VEVENT\nBEGIN:VEVENT\nUID:other\nDTSTART:20261112T030000Z\nEND:VEVENT\nEND:VCALENDAR");
        let parsed=inspect_ics(&ics,"2026-11-01T00:00:00Z".parse().unwrap(),"2026-12-01T00:00:00Z".parse().unwrap()).unwrap();
        assert_eq!(parsed.events.iter().map(|event|event.uid.as_str()).collect::<Vec<_>>(),["other"]);
        assert!(parsed.report.invalid>0);assert!(!parsed.report.warning().is_empty());
    }
}

#[test]
fn recurrence_limit_preserves_valid_instances_and_explains_truncation(){
    let ics="BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:many\nDTSTART:20261101T000000Z\nRRULE:FREQ=MINUTELY;COUNT=2500\nEND:VEVENT\nEND:VCALENDAR";
    let parsed=inspect_ics(ics,"2026-11-01T00:00:00Z".parse().unwrap(),"2026-12-01T00:00:00Z".parse().unwrap()).unwrap();
    assert_eq!(parsed.events.len(),2000);assert!(parsed.report.limited);assert!(parsed.report.warning().contains("读取上限"));
}

#[test]
fn mixed_caldav_response_preserves_readable_events_and_reports_resource_errors(){
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://{}/",listener.local_addr().unwrap());
    let worker=std::thread::spawn(move||{
        let (mut socket,_)=listener.accept().unwrap();let _=read_request(&mut socket);
        let xml="<d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:response><d:href>/good.ics</d:href><c:calendar-data>BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:good\nDTSTART:20261112T020000Z\nEND:VEVENT\nBEGIN:VEVENT\nUID:bad\nDTSTART;TZID=Unsupported/Zone:20261112T100000\nEND:VEVENT\nEND:VCALENDAR</c:calendar-data></d:response><d:response><d:href>/private.ics</d:href><d:status>HTTP/1.1 403 Forbidden</d:status></d:response></d:multistatus>";
        write!(socket,"HTTP/1.1 207 Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{xml}",xml.len()).unwrap();
    });
    let config=Config{provider:Provider::DingTalk,username:"mock".into(),href:None,server:Some(base.clone())};
    let result=fetch_collection(&agent().unwrap(),&config,"mock",&base,"2026-11-01T00:00:00Z".parse().unwrap(),"2026-12-01T00:00:00Z".parse().unwrap());worker.join().unwrap();
    let fetched=result.unwrap();assert_eq!(fetched.events.len(),1);assert_eq!(fetched.events[0].uid,"good");
    assert_eq!((fetched.report.failed,fetched.report.invalid,fetched.report.status),(1,1,Some(403)));
}

#[test]
fn third_party_invitation_fields_remain_compatible_with_mail_and_calendar_parsers(){
    for state in ["ACCEPTED","NEEDS-ACTION"]{
        let ics=format!("BEGIN:VCALENDAR\nMETHOD:REQUEST\nPRODID:-//Synthetic Recruitment//CN\nBEGIN:VEVENT\nUID:interview\nSUMMARY:面试\nDTSTART;TZID=Asia/Shanghai:20261112T100000\nDTEND;TZID=Asia/Shanghai:20261112T110000\nCLASS:PRIVATE\nORGANIZER;CN=HR:mailto:hr@example.invalid\nATTENDEE;PARTSTAT={state}:mailto:viewer@example.invalid\nX-APP-ID:mock\nEND:VEVENT\nEND:VCALENDAR");
        let calendar=events_from_ics(&ics,"2026-11-01T00:00:00Z".parse().unwrap(),"2026-12-01T00:00:00Z".parse().unwrap()).unwrap();let mail=crate::mail::meetings_of(&ics);
        assert_eq!(calendar.len(),1);assert_eq!(mail.len(),1);assert_eq!(calendar[0],mail[0]);
    }
}

#[test]
fn multiple_calendar_homes_are_deduplicated_and_task_only_collections_are_excluded(){
    let xml="<response><calendar-home-set><href>/home/</href><href>/work/?x=1&amp;y=2</href><href>/home/</href></calendar-home-set></response>";
    assert_eq!(property_hrefs(xml,"calendar-home-set","https://calendar.example/"),["https://calendar.example/home/","https://calendar.example/work/?x=1&y=2"]);
    let task="<response><href>/tasks/</href><resourcetype><calendar/></resourcetype><supported-calendar-component-set><comp name=\"VTODO\"/></supported-calendar-component-set></response>";
    assert!(calendar_collections(task,"https://calendar.example/").is_empty());
}

#[test]
fn inaccessible_home_does_not_block_other_homes_or_hide_the_warning(){
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://{}/",listener.local_addr().unwrap());
    let worker=std::thread::spawn(move||{
        for expected in ["/","/denied/","/home/","/work/"]{
            listener.set_nonblocking(true).unwrap();let until=Instant::now()+Duration::from_secs(3);
            let(mut socket,_)=loop{if let Ok(socket)=listener.accept(){break socket;}assert!(Instant::now()<until,"缺少预期请求 {expected}");std::thread::sleep(Duration::from_millis(2));};
            let request=read_request(&mut socket);assert_eq!(request.split_whitespace().nth(1),Some(expected));
            let(status,body)=match expected{
                "/"=>(207,"<response><href>/</href><c:calendar-home-set><href>/denied/</href><href>/home/</href></c:calendar-home-set></response>"),
                "/denied/"=>(403,""),
                "/home/"=>(207,"<response><href>/work/</href><resourcetype><c:calendar/></resourcetype></response>"),
                _=>(207,"<response><href>/work/one.ics</href><c:calendar-data>BEGIN:VEVENT\nUID:work\nDTSTART:20261112T020000Z\nEND:VEVENT</c:calendar-data></response>"),
            };
            let body=format!(r#"<multistatus xmlns="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">{body}</multistatus>"#);write!(socket,"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let cfg=Config{provider:Provider::Custom,username:"mock".into(),href:None,server:Some(base)};
    let result=fetch_month(&cfg,"mock","2026-11-01".parse().unwrap());worker.join().unwrap();
    let fetched=result.unwrap();assert_eq!(fetched.events.len(),1);assert_eq!(fetched.report.failed,1);assert_eq!(fetched.report.status,Some(403));
}
