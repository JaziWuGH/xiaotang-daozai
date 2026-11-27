use super::*;
use crate::{app::{Cmd,MAIL_PAGE},mail::{self,Account,Item,Provider}};

#[test]
fn mail_module_closes_reader_and_clears_pending_requests() {
    let _g=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());
    let mut h=H::new();
    let account=Account{email:"reader@qq.com".into(),provider:Provider::Qq,custom:None,enabled:true,use_windows_proxy:false};
    let item=Item{
        account:account.email.clone(),from:"发件人".into(),subject:"测试邮件".into(),uid:42,
        time:"09:41".into(),received_unix:None,code:Some("482913".into()),html:"".into(),
        summary:"邮件正文".into(),links:vec![],meeting:None,
    };
    h.app.mail.accounts.push(account.clone());
    h.app.mail_event(mail::Event::Mail{item:item.clone(),new:false});
    h.app.mail_open_reader(&item);
    h.app.mail.open_uid=Some((account.email.clone(),item.uid));
    h.app.mail.testing=Some(account.email.clone());
    h.app.mail.note="正在测试连接…".into();
    h.app.mail.form.open=true;
    h.app.mail.form.password="临时授权码".into();
    h.app.mail.manage.pending.insert(account.email.clone());
    h.app.mail.manage.auto_read=Some((account.email.clone(),item.uid));
    h.app.mail.manage.next_refresh=600.0;
    h.app.pending.push(Cmd::MailTest(account.clone(),"临时授权码".into()));
    h.app.pending.push(Cmd::MailSync);

    h.app.set_page_visible(MAIL_PAGE,false);
    assert!(h.app.mail.reading.is_none() && !h.app.mail.reader_focus);
    assert!(h.app.mail.open_uid.is_none() && h.app.mail.testing.is_none());
    assert!(h.app.mail.note.is_empty() && !h.app.mail.form.open && h.app.mail.form.password.is_empty());
    assert!(h.app.mail.manage.pending.is_empty() && h.app.mail.manage.auto_read.is_none());
    assert_eq!(h.app.mail.manage.next_refresh,0.0);
    assert!(!h.app.pending.iter().any(|cmd|matches!(cmd,Cmd::MailTest(..)|Cmd::MailSync)));
    h.app.mail_event(mail::Event::Test{account:account.email.clone(),ok:true,note:String::new()});
    h.app.mail_open_reader(&item);
    assert!(h.app.mail.note.is_empty() && h.app.mail.reading.is_none());

    h.app.set_page_visible(MAIL_PAGE,true);
    assert_eq!(h.app.mail.recent[0].uid,item.uid,"关闭模块时保留邮件历史");
    h.app.mail_open_reader(&item);
    assert_eq!(h.app.mail.reading.as_ref().map(|m|m.uid),Some(item.uid));
}

#[test]
fn mail_module_hides_its_bell_and_keeps_other_notifications() {
    let _g=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());
    let mut h=H::new();
    let account=Account{email:"bell@qq.com".into(),provider:Provider::Qq,custom:None,enabled:true,use_windows_proxy:false};
    let item=Item{
        account:account.email.clone(),from:"发件人".into(),subject:"新邮件".into(),uid:1,
        time:"09:41".into(),received_unix:None,code:None,html:"".into(),
        summary:"".into(),links:vec![],meeting:None,
    };
    h.app.mail.accounts.push(account);
    h.app.t=100.0;
    h.app.notification_bell_at=99.0;
    h.app.notification_bell_until=101.0;
    h.app.mail_event(mail::Event::Mail{item:item.clone(),new:true});
    assert_eq!(h.app.notification_bell(),Some(100.0));

    h.app.set_page_visible(MAIL_PAGE,false);
    assert_eq!(h.app.notification_bell(),Some(99.0),"已有的独立提醒不受影响");
    h.app.t=102.0;
    assert!(h.app.notification_bell().is_none(),"关闭邮箱后只收到邮件的铃铛应消失");
    h.app.set_page_visible(MAIL_PAGE,true);
    assert!(h.app.notification_bell().is_none(),"重新开启后不恢复关闭前的铃铛");
    h.app.mail_event(mail::Event::Mail{item:Item{uid:2,..item},new:true});
    assert_eq!(h.app.notification_bell(),Some(102.0));
}
