use super::*;
use crate::app::{Cmd, CALENDAR_PAGE, FOCUS_PAGE, MAIL_PAGE, TODO_PAGE, WATER_PAGE};

fn meeting(uid:&str)->crate::mail::Meeting {
    crate::mail::Meeting{
        uid:uid.into(), title:uid.into(), at:"09:50".into(),
        start:chrono::Local::now().timestamp()+300, end:None, all_day:false,
    }
}

#[test]
fn closing_modules_clears_visible_and_queued_reminders() {
    let _guard=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());
    for (page,title,second) in [
        (FOCUS_PAGE,"专注计时","专注计时"),
        (WATER_PAGE,"喝水时间到了",crate::app::SIT_TITLE),
        (TODO_PAGE,"待办到时间了","待办到时间了"),
        (CALENDAR_PAGE,"会议提醒","会议提醒"),
        (MAIL_PAGE,"新邮件","会议提醒"),
    ] {
        let mut h=H::new();
        let emit=|app:&mut App,title:&str,body:&str| {
            if title=="会议提醒" {
                let source=if page==MAIL_PAGE {"邮件邀请"}else{"本机日程"};
                app.meeting_alert(meeting(body),source.into(),CALENDAR_PAGE);
            }else{app.alert(title,body,page,true);}
        };
        emit(&mut h.app,title,"当前提醒");
        h.app.tick_alerts();
        assert_eq!(h.app.notice.as_ref().unwrap().title,title);
        h.app.hud_notice=h.app.notice.clone();
        emit(&mut h.app,second,"排队提醒");
        if page==MAIL_PAGE {h.app.meeting_alert(meeting("另一个来源"),"本机日程".into(),CALENDAR_PAGE);}
        h.app.alert("系统音量","模拟系统提示",page,true);

        h.app.set_page_visible(page,false);
        assert!(h.app.notice.is_none() && h.app.hud_notice.is_none(),"{page} 的原提醒仍在显示");
        emit(&mut h.app,title,"关闭后进入的提醒");
        emit(&mut h.app,second,"关闭后进入的排队提醒");
        let expected=if page==MAIL_PAGE {vec!["会议提醒","系统音量"]}else{vec!["系统音量"]};
        assert_eq!(h.app.alerts.iter().map(|alert|alert.title.as_str()).collect::<Vec<_>>(),expected);
        assert_eq!(h.app.pending.iter().filter_map(|cmd|match cmd{
            Cmd::Notify(title,_,_)=>Some(title.as_str()),_=>None,
        }).collect::<Vec<_>>(),expected);
        assert!(!h.app.history.iter().any(|alert|alert.body.contains("关闭后进入")));
        assert!(h.app.health_badges().is_empty());

        h.app.set_page_visible(page,true);
        h.app.tick_alerts();
        assert_eq!(h.app.notice.as_ref().unwrap().title,expected[0]);
        if page==WATER_PAGE {
            assert_eq!(h.app.health_badges(),crate::extras::HealthReminder::ALL);
        }
    }
}

#[test]
fn reopening_reminder_modules_checks_due_items_again() {
    let _guard=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());
    let mut h=H::new();h.frame(vec![]);
    for page in [FOCUS_PAGE,WATER_PAGE,TODO_PAGE,CALENDAR_PAGE] {h.app.set_page_visible(page,false);}
    h.app.settings.water_start=0;h.app.settings.water_end=0;h.app.settings.lunch_rest=false;
    h.app.water.remind=true;h.app.last_reminder=h.t-3600.0;
    h.app.todos.items.push(crate::model::TodoItem{text:"交周报".into(),due:Some("2026-10-28 09:40".into()),..Default::default()});
    h.app.calendar.local.push(crate::local_calendar::Entry{event:meeting("周会"),reminder:Some(15)});
    h.frame(vec![]);
    assert!(h.app.history.is_empty());
    assert!(!h.app.todos.items[0].reminded && h.app.mail.fired.is_empty());
    for page in [FOCUS_PAGE,WATER_PAGE,TODO_PAGE,CALENDAR_PAGE] {h.app.set_page_visible(page,true);}
    h.frame(vec![]);
    for title in ["喝水时间到了","待办到时间了","会议提醒"] {
        assert!(h.app.history.iter().any(|alert|alert.title==title),"重新开启后应检查 {title}");
    }
    assert!(!h.app.pomo.running,"重新开启专注不自动启动计时");
}

#[test]
fn reopening_calendar_retries_interrupted_month_sync() {
    let _guard=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());
    let mut h=H::new();
    let cfg=crate::calendar::Config{provider:crate::calendar::Provider::WeCom,username:"test".into(),href:None,server:None};
    crate::calendar::save_config(&cfg).unwrap();
    h.app.calendar=crate::app::CalendarState::load();
    h.app.request_calendar_month(false);
    assert!(h.app.calendar.view.loading && h.app.calendar.feeds[0].month.loading);
    h.app.calendar.testing=true;
    h.app.calendar.form.open=true;
    h.app.pending.push(Cmd::CalendarSync);
    h.app.set_page_visible(CALENDAR_PAGE,false);
    assert!(!h.app.calendar.view.loading && !h.app.calendar.feeds[0].month.loading);
    assert!(!h.app.calendar.testing && !h.app.calendar.form.open);
    assert!(h.app.pending.is_empty());
    h.app.open_calendar(None);
    h.app.request_calendar_month(true);
    assert!(h.app.settings_open);
    assert!(!h.app.calendar.view.loading && h.app.pending.is_empty());
    h.app.set_page_visible(CALENDAR_PAGE,true);
    h.app.open_calendar(None);
    assert!(h.app.calendar.view.loading);
    assert!(h.app.pending.iter().any(|cmd|matches!(cmd,Cmd::CalendarMonth(_))));
}
