//! 岛内本机日程表单。
use crate::{app::{App,Cmd,CALENDAR_PAGE},local_calendar::{self,Draft},paint::{self,Card,H},theme::{self,W}};
#[cfg(all(windows, feature = "sdk-material"))]
use crate::material::ControlRole;
use eframe::egui::{self,Color32};
use chrono::{Datelike, Local, Months, NaiveDate, NaiveTime, Timelike};
use crate::task_picker::WheelInput;

const PICKER_SELECTED: Color32 = Color32::from_rgb(28,65,100);

pub(crate) enum Picker {
    Date { end: bool, month: NaiveDate },
    Time { end: bool, time: NaiveTime, wheels: [WheelInput; 2] },
}

fn picker_button(c: &Card, x: f32, width: f32, id: &str, label: &str, color: Color32) -> bool {
    let hover = if color==PICKER_SELECTED {paint::mix(color,theme::WHITE,0.14)} else {theme::BTN_HOVER};
    let (response, button) = paint::capsule(c,x,39.0,width,21.0,id,color,hover);
    button.centered_text(x+width/2.0,49.5,label,10.0,W::Regular,theme::WHITE,H::Center);
    response.clicked()
}

fn picker_cell(c: &Card, (x,y,width,height): (f32,f32,f32,f32), id: impl std::hash::Hash, label: &str, selected: bool) -> bool {
    let response = c.hit(x,y,width,height,id);
    let fill = if selected {PICKER_SELECTED} else if response.hovered() {theme::BTN_HOVER} else {theme::CARD};
    c.fill(x,y,width,height,5.0,fill);
    c.centered_text(x+width/2.0,y+height/2.0,label,10.0,if selected {W::Semibold} else {W::Regular},theme::WHITE,H::Center);
    response.clicked()
}

fn picker_entry(c: &Card, end: bool, date: bool, y: f32) -> bool {
    let (x, width, label) = match (end,date) {
        (false,true) => (180.0,26.0,"选择开始日期"),
        (true,true) => (180.0,26.0,"选择结束日期"),
        (false,false) => (258.0,22.0,"选择开始时间"),
        (true,false) => (258.0,22.0,"选择结束时间"),
    };
    let response = c.hit(x,y,width,23.0,("calendar-picker-entry",end,date)).on_hover_text(label);
    if response.hovered() { c.fill(x+2.0,y+2.0,width-4.0,19.0,5.0,theme::BTN_HOVER); }
    let color = if response.hovered() {theme::WHITE} else {theme::GRAY};
    let cx = x+width/2.0;
    let cy = y+11.5;
    if date {
        c.stroke(cx-4.5,cy-4.0,9.0,8.5,1.4,1.0,color);
        c.seg((cx-4.0,cy-1.1),(cx+4.0,cy-1.1),1.0,color);
        for dx in [-2.3,2.3] { c.seg((cx+dx,cy-5.5),(cx+dx,cy-2.6),1.0,color); }
    } else {
        c.ring(cx,cy,4.7,1.0,color);
        c.path(&[(cx,cy-2.5),(cx,cy),(cx+2.1,cy+1.2)],1.0,color);
    }
    response.clicked()
}

fn time_wheel(c: &Card, x: f32, label: &str, value: u32, count: i64, wheel: &mut WheelInput) -> u32 {
    const CENTER: f32 = 121.0;
    const ROW: f32 = 21.0;
    let response = c.hit_drag(x,68.5,78.0,105.0,("calendar-time-wheel",label));
    let (scroll,drag) = wheel.steps(c,&response,ROW);
    let click = response.clicked().then(||response.interact_pointer_pos()).flatten()
        .map_or(0,|pos|((c.to_local(pos).y-CENTER)/ROW).round() as i64);
    let keys = if response.has_focus() {
        c.ui.input(|i| i64::from(i.key_pressed(egui::Key::ArrowDown))-i64::from(i.key_pressed(egui::Key::ArrowUp)))
    } else {0};
    let value = (i64::from(value)+scroll%count+drag%count+click+keys).rem_euclid(count);
    c.centered_text(x+39.0,55.0,label,10.0,W::Regular,theme::GRAY,H::Center);
    for offset in -2_i64..=2 {
        let text = format!("{:02}",(value+offset).rem_euclid(count));
        let (size,weight,color) = match offset.abs() {
            0 => (18.0,W::Semibold,theme::WHITE),
            1 => (12.0,W::Regular,theme::GRAY),
            _ => (10.0,W::Regular,theme::DIM),
        };
        c.centered_text(x+39.0,CENTER+offset as f32*ROW,&text,size,weight,color,H::Center);
    }
    value as u32
}

impl Picker {
    /// 返回是否继续显示选择器，确认时仅回填对应输入框。
    fn draw(&mut self, c: &Card, draft: &mut Draft) -> bool {
        let title = match self {
            Self::Date {end:false,..} => "开始日期",
            Self::Date {end:true,..} => "结束日期",
            Self::Time {end:false,..} => "开始时间",
            Self::Time {end:true,..} => "结束时间",
        };
        c.centered_text(43.0,20.0,title,13.0,W::Semibold,theme::WHITE,H::Left);
        match self {
            Self::Date {end,month} => {
                if picker_button(c,306.0,64.0,"calendar-date-cancel","取消",theme::BTN) { return false; }
                let date = if *end {&mut draft.end_date} else {&mut draft.start_date};
                let selected = NaiveDate::parse_from_str(date.trim(),"%Y-%m-%d").ok();
                let mut choice = picker_button(c,214.0,53.0,"calendar-date-today","今天",theme::BTN).then(||Local::now().date_naive());
                if picker_button(c,43.0,26.0,"calendar-date-prev","‹",theme::BTN) {
                    *month = month.checked_sub_months(Months::new(1)).unwrap_or(*month);
                }
                if picker_button(c,164.0,26.0,"calendar-date-next","›",theme::BTN) {
                    *month = month.checked_add_months(Months::new(1)).unwrap_or(*month);
                }
                c.centered_text(116.5,49.5,&format!("{}年{}月",month.year(),month.month()),11.0,W::Semibold,theme::WHITE,H::Center);
                for (i,day) in ["一","二","三","四","五","六","日"].into_iter().enumerate() {
                    c.centered_text(65.5+47.0*i as f32,74.0,day,9.0,W::Regular,theme::GRAY,H::Center);
                }
                let offset = month.weekday().num_days_from_monday();
                for day in month.iter_days().take_while(|day| day.month()==month.month()) {
                    let i = offset+day.day()-1;
                    let cell = (43.0+47.0*(i%7) as f32,87.0+20.0*(i/7) as f32,45.0,19.0);
                    if picker_cell(c,cell,("calendar-day-option",day),&day.day().to_string(),selected==Some(day)) {
                        choice = Some(day);
                    }
                }
                if let Some(day) = choice {
                    *date = day.format("%Y-%m-%d").to_string();
                    draft.error.clear();
                    return false;
                }
            }
            Self::Time {end,time,wheels} => {
                c.fill(111.0,65.0,192.0,112.0,10.0,theme::CARD);
                c.fill(120.0,108.0,174.0,26.0,6.0,PICKER_SELECTED);
                c.centered_text(207.0,121.0,":",16.0,W::Semibold,theme::WHITE,H::Center);
                let hour = time_wheel(c,124.0,"小时",time.hour(),24,&mut wheels[0]);
                let minute = time_wheel(c,208.0,"分钟",time.minute(),60,&mut wheels[1]);
                *time = NaiveTime::from_hms_opt(hour,minute,0).unwrap();
                let footer = c.shifted(0.0,149.0);
                if picker_button(&footer,224.0,64.0,"calendar-time-cancel","取消",theme::BTN) { return false; }
                if picker_button(&footer,298.0,72.0,"calendar-time-save","完成",PICKER_SELECTED) {
                    let field = if *end {&mut draft.end_time} else {&mut draft.start_time};
                    *field = time.format("%H:%M").to_string();
                    draft.error.clear();
                    return false;
                }
            }
        }
        true
    }
}

fn fallback_ics_uid(event:&crate::mail::Meeting)->String{
    let key=format!("t{}|{}",event.start,event.title.trim());
    let hash=key.bytes().fold(1469598103934665603u64,|hash,byte|(hash^byte as u64).wrapping_mul(1099511628211));
    format!("ics-missing-{hash:016x}-{}",event.start)
}

impl App {
    pub(crate) fn open_calendar_editor(&mut self,uid:Option<&str>){
        let view=&self.calendar.view;
        let day=if self.calendar.agenda {view.month.max(chrono::Local::now().date_naive())}else{view.day};
        let draft=uid.and_then(|uid|self.calendar.local.iter().find(|entry|entry.event.uid==uid)).map(Draft::edit)
            .unwrap_or_else(||Draft::new(day));
        self.show_page(CALENDAR_PAGE);self.calendar.draft=Some(draft);
    }
    pub(crate) fn save_calendar_editor(&mut self){
        let Some(mut draft)=self.calendar.draft.take() else{return;};
        let result=(||{
            if !self.calendar.local_error.is_empty(){return Err(self.calendar.local_error.clone());}
            let entry=draft.entry()?;let mut entries=self.calendar.local.clone();
            if let Some(saved)=entries.iter_mut().find(|saved|saved.event.uid==entry.event.uid){*saved=entry.clone();}else{entries.push(entry.clone());}
            local_calendar::save(&entries)?;
            self.alerts.retain(|alert|!alert.meeting.as_ref().is_some_and(|(event,_)|event.uid==entry.event.uid));
            if self.calendar.local.iter().any(|old|old.event.uid==entry.event.uid && (old.event.start!=entry.event.start || old.reminder!=entry.reminder)) {
                self.mail.fired.remove(&crate::calendar::dedupe_key(&entry.event));
            }
            self.calendar.local=entries;
            self.open_calendar(Some(entry.event));
            Ok::<_,String>(())
        })();
        if let Err(error)=result{draft.error=error;self.calendar.draft=Some(draft);}
    }
    /// 导入 .ics 文件到本机日程：过去 30 天到未来一年，重复日程展开；同一日程再次导入会跳过。
    pub fn import_ics(&mut self,files:&[(String,String)])->String{
        if !self.calendar.local_error.is_empty(){return self.calendar.local_error.clone();}
        let now=chrono::Utc::now();
        let (lo,hi)=(now-chrono::Duration::days(30),now+chrono::Duration::days(366));
        let mut entries=self.calendar.local.clone();
        let mut known:std::collections::HashSet<_>=entries.iter().map(|entry|entry.event.uid.clone()).collect();
        // 兼容旧版本对缺少 UID 的日程生成的标识，重新导入时保留已有本机记录。
        for entry in &entries{if entry.event.uid==format!("ics--{}",entry.event.start){known.insert(fallback_ics_uid(&entry.event));}}
        let (mut added,mut duplicates,mut capacity,mut errors)=(0usize,0usize,0usize,Vec::new());
        let mut report=crate::calendar::ReadReport::default();
        for (name,text) in files {
            match crate::calendar::inspect_ics(text,lo,hi) {
                Ok(parsed)=>{report.merge(parsed.report);for mut event in parsed.events {
                    event.uid=if event.uid.is_empty(){fallback_ics_uid(&event)}else{format!("ics-{}-{}",event.uid,event.start)};
                    if known.contains(&event.uid){duplicates+=1;continue;}
                    if entries.len()>=5000 {capacity+=1;continue;}
                    known.insert(event.uid.clone());
                    let end=event.end.filter(|end|*end>event.start).unwrap_or(event.start+if event.all_day{86400}else{3600});
                    event.end=Some(end);
                    let reminder=(!event.all_day).then_some(15);
                    entries.push(local_calendar::Entry{event,reminder});added+=1;
                }},
                Err(error)=>errors.push(format!("{name}：{error}")),
            }
        }
        if added>0 {
            if let Err(error)=local_calendar::save(&entries){return error;}
            self.calendar.local=entries;
        }
        let mut note=if added>0{format!("已导入 {added} 个日程")}else{"没有新增日程".into()};
        if duplicates>0{note.push_str(&format!("；{duplicates} 个已存在已跳过"));}
        if capacity>0{note.push_str(&format!("；已达到 5000 个本机日程上限，{capacity} 个未导入"));}
        if report.outside>0{note.push_str(&format!("；{} 个超出导入范围（过去 30 天至未来一年）",report.outside));}
        if report.cancelled>0{note.push_str(&format!("；{} 个已取消日程未导入",report.cancelled));}
        let warning=report.warning();if !warning.is_empty(){note.push('；');note.push_str(&warning);}
        if !errors.is_empty(){if !note.is_empty(){note.push('；');}note.push_str(&errors.join("；"));}
        if !cfg!(test){crate::resources::append_log(&crate::model::config_dir().join("diagnostics"),"calendar.log",&format!("{} source=ics files={} received={} imported={added} duplicates={duplicates} capacity={capacity} outside={} invalid={} unsupported={} file_errors={} limited={}",chrono::Utc::now().timestamp(),files.len(),report.received,report.outside,report.invalid,report.unsupported,errors.len(),report.limited));}
        self.calendar.import_note=note.clone();
        note
    }
    fn delete_calendar_editor(&mut self){
        let Some(draft)=self.calendar.draft.as_mut() else{return;};
        let Some(uid)=draft.uid.as_ref() else{return;};
        let entries:Vec<_>=self.calendar.local.iter().filter(|entry|entry.event.uid!=*uid).cloned().collect();
        match local_calendar::save(&entries){
            Ok(())=>{
                self.alerts.retain(|alert|!alert.meeting.as_ref().is_some_and(|(event,_)|event.uid==*uid));
                if self.notice.as_ref().is_some_and(|alert|alert.meeting.as_ref().is_some_and(|(event,_)|event.uid==*uid)){self.notice=None;}
                self.calendar.local=entries;self.calendar.draft=None;self.calendar.view.focus=None;
            }
            Err(error)=>draft.error=error,
        }
    }
    pub(crate) fn draw_calendar_editor(&mut self,c:&Card){
        let glassy=self.glassy();
        let glass_gpu=self.glass_controls_ready();
        let glass=|level|glassy.then_some((glass_gpu,level,0.42));
        let Some(d)=self.calendar.draft.as_mut() else{return;};
        if let Some(mut picker)=d.picker.take() {
            if picker.draw(c,d) { d.picker=Some(picker); }
            c.ui.input_mut(|input|input.consume_key(egui::Modifiers::NONE,egui::Key::Enter));
            return;
        }
        let accent=Color32::from_rgb(82,162,255);
        c.centered_text(43.0,20.0,if d.uid.is_some(){"编辑日程"}else{"新建日程"},13.0,W::Semibold,theme::WHITE,H::Left);
        // 图钉占用右侧时，将说明放到表单末行，给时间和图钉留出空间。
        if self.hold_slot<1.0 {
            c.faded(1.0-self.hold_slot).centered_text(370.0,20.0,"仅保存在本机",9.0,W::Regular,theme::GRAY,H::Right);
        }
        if self.hold_slot>0.0 {
            c.faded(self.hold_slot).centered_text(370.0,172.0,"仅保存在本机",9.0,W::Regular,theme::GRAY,H::Right);
        }
        for (label,y) in [("名称",49.0),("开始",82.0),("结束",114.0),("提醒",147.0)]{c.centered_text(43.0,y,label,10.0,W::Regular,theme::GRAY,H::Left);}
        for (x,y,w) in [(86.0,38.0,284.0),(86.0,71.0,120.0),(86.0,103.0,120.0)]{c.fill(x,y,w,23.0,6.0,theme::CARD);}
        if !d.all_day {for y in [71.0,103.0]{c.fill(214.0,y,66.0,23.0,6.0,theme::CARD);}}
        let (all_day,b)=paint::material_capsule(c,294.0,72.0,76.0,21.0,"calendar-editor-all-day",
            if d.all_day{Color32::from_rgb(28,65,100)}else{theme::BTN},theme::BTN_HOVER,
            glass(if d.all_day {0.55}else{0.12}),#[cfg(all(windows, feature = "sdk-material"))] ControlRole::CalAllDay);
        if glass_gpu && d.all_day {
            let from=paint::chrome_mark(c);
            b.fill(294.0,72.0,76.0,21.0,10.5,Color32::from_white_alpha(28));
            paint::chrome_since(c,from);
        }
        b.centered_text(332.0,82.5,if d.all_day{"✓ 全天"}else{"全天"},10.0,W::Regular,if glassy && d.all_day {accent}else{theme::WHITE},H::Center);
        if all_day.clicked(){d.all_day=!d.all_day;d.error.clear();}
        if d.all_day {c.centered_text(214.0,114.0,"包含结束当天",8.5,W::Regular,theme::GRAY,H::Left);}
        for (end,y) in [(false,71.0),(true,103.0)] {
            if picker_entry(c,end,true,y) {
                let field=if end {&d.end_date} else {&d.start_date};
                let date=NaiveDate::parse_from_str(field.trim(),"%Y-%m-%d").unwrap_or(self.calendar.view.day);
                d.picker=Some(Picker::Date {end,month:crate::calendar::month_start(date)});
            }
            if !d.all_day && picker_entry(c,end,false,y) {
                let field=if end {&d.end_time} else {&d.start_time};
                let time=NaiveTime::parse_from_str(field.trim(),"%H:%M").unwrap_or_else(|_|Local::now().time());
                d.picker=Some(Picker::Time {end,time,wheels:[WheelInput::default();2]});
            }
        }
        let options=if d.all_day{vec![(None,"不提醒"),(Some(0),"当天 09:00")]}else{vec![(None,"不提醒"),(Some(0),"准时"),(Some(5),"提前5分"),(Some(15),"提前15分"),(Some(30),"提前30分")]};
        let width=if d.all_day{126.0}else{54.0};
        for (i,(value,label)) in options.into_iter().enumerate(){
            let x=86.0+i as f32*(width+3.0);let on=if d.all_day{d.reminder.is_some()==value.is_some()}else{d.reminder==value};
            let (r,b)=paint::material_capsule(c,x,136.0,width,21.0,&format!("calendar-editor-reminder-{i}"),
                if on{Color32::from_rgb(28,65,100)}else{theme::BTN},theme::BTN_HOVER,
                glass(if on {0.55}else{0.12}),#[cfg(all(windows, feature = "sdk-material"))] ControlRole::CalReminder);
            if glass_gpu && on {
                let from=paint::chrome_mark(c);
                b.fill(x,136.0,width,21.0,10.5,Color32::from_white_alpha(28));
                paint::chrome_since(c,from);
            }
            b.centered_text(x+width/2.0,146.5,label,8.4,W::Regular,if glassy && on {accent}else{theme::WHITE},H::Center);
            if r.clicked(){d.reminder=value;}
        }
        let message=if d.error.is_empty(){"日期 年-月-日 · 时间 时:分"}else{&d.error};
        c.centered_text(86.0,172.0,&c.fit(message,9.0,W::Regular,284.0-68.0*self.hold_slot),9.0,W::Regular,if d.error.is_empty(){theme::GRAY}else{theme::RED},H::Left);
        let mut delete=false;
        if d.uid.is_some(){
            let (r,b)=paint::material_capsule(c,43.0,188.0,82.0,22.0,"calendar-editor-delete",theme::BTN,theme::BTN_HOVER,
                glass(0.16),#[cfg(all(windows, feature = "sdk-material"))] ControlRole::CalDelete);
            b.centered_text(84.0,199.0,if d.confirm_delete{"确认删除"}else{"删除日程"},9.5,W::Regular,theme::RED,H::Center);
            if r.clicked(){if d.confirm_delete{delete=true;}else{d.confirm_delete=true;}}
        }
        let (cancel,b)=paint::material_capsule(c,224.0,188.0,64.0,22.0,"calendar-editor-cancel",theme::BTN,theme::BTN_HOVER,
            glass(0.16),#[cfg(all(windows, feature = "sdk-material"))] ControlRole::CalCancel);
        b.centered_text(256.0,199.0,"取消",10.0,W::Regular,theme::WHITE,H::Center);
        let (save,b)=paint::material_capsule(c,298.0,188.0,72.0,22.0,"calendar-editor-save",accent,Color32::from_rgb(110,180,255),
            glass(0.55),#[cfg(all(windows, feature = "sdk-material"))] ControlRole::CalSave);
        if glassy {
            let (hover,_)=c.hover_t(&save);
            b.fill(298.0,188.0,72.0,22.0,11.0,paint::alpha(accent,0.42+hover*0.16));
        }
        b.centered_text(334.0,199.0,"保存到本机",10.0,W::Semibold,theme::WHITE,H::Center);
        if delete{self.delete_calendar_editor();}else if cancel.clicked(){self.calendar.draft=None;}else if save.clicked(){self.save_calendar_editor();}
    }
    pub(crate) fn calendar_editor_inputs(&mut self,ui:&mut egui::Ui,origin:egui::Pos2,cmds:&mut Vec<Cmd>){
        let Some(d)=self.calendar.draft.as_mut() else{return;};
        if d.picker.is_some() {return;}
        let mut input=|text:&mut String,id:&str,x:f32,y:f32,width:f32|ui.put(egui::Rect::from_min_size(origin+egui::vec2(x,y),egui::vec2(width,19.0)),
            egui::TextEdit::singleline(text).id(egui::Id::new(id)).frame(false).margin(egui::Margin::ZERO).text_color(theme::WHITE).font(theme::font(11.0,W::Regular)));
        let name=input(&mut d.title,"calendar-local-title",92.0,41.0,272.0);
        input(&mut d.start_date,"calendar-local-start-date",92.0,74.0,86.0);
        input(&mut d.end_date,"calendar-local-end-date",92.0,106.0,86.0);
        if !d.all_day {input(&mut d.start_time,"calendar-local-start-time",220.0,74.0,36.0);input(&mut d.end_time,"calendar-local-end-time",220.0,106.0,36.0);}
        let first=!d.focused;if first{name.request_focus();d.focused=true;cmds.push(Cmd::Focus);}
        if !first && ui.input(|i|i.key_pressed(egui::Key::Enter)){self.save_calendar_editor();}
    }
}
