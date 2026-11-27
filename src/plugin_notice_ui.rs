use crate::app::{App, BAR_H, Cmd, Env};
use crate::extras::{Alert, AlertSource};
use crate::paint::{self, Card, H};
use crate::plugin_notice::{Completion, Event, MAX_SOURCES};
use crate::theme::{self, W, size};
use eframe::egui::{self, Color32};
use std::collections::VecDeque;

pub(crate) const DETAIL_HEIGHT: f32 = 204.0;
const MAX_UNREAD_PER_SOURCE: usize = 4;
const MAX_UNREAD: usize = MAX_SOURCES * MAX_UNREAD_PER_SOURCE;
const COLLAPSED_BADGES: usize = 3;
const RED_DOT: Color32 = Color32::from_rgb(255, 59, 48);
const STATUS_DOT: Color32 = Color32::from_rgb(85, 180, 255);

#[derive(Default)]
pub(crate) struct NoticeUi {
    unread: VecDeque<Completion>,
    status: Vec<Completion>,
    detail: Option<Completion>,
    opening: Vec<(String, String)>,
}

impl NoticeUi {
    pub(crate) fn item_count(&self) -> usize {
        self.unread.len() + self.status.len()
    }

    fn entries(&self) -> impl Iterator<Item = (&Completion, bool)> {
        self.unread.iter().map(|notice| (notice, false))
            .chain(self.status.iter().map(|card| (card, true)))
    }

    fn sources(&self) -> Vec<(&Completion, bool)> {
        let mut sources: Vec<(&Completion, bool)> = Vec::new();
        for (notice, status) in self.entries() {
            if !sources.iter().any(|(current, _)| current.source == notice.source) {
                sources.push((notice, status));
            }
        }
        sources
    }

    pub(crate) fn collapsed_count(&self) -> usize {
        self.sources().len().min(COLLAPSED_BADGES)
    }

    fn hide_content(&mut self) {
        for notice in self.unread.iter_mut().chain(&mut self.status).chain(self.detail.iter_mut()) {
            hide_content(notice);
        }
    }

    fn find(&self, source: &str, id: &str) -> Option<&Completion> {
        self.unread.iter().chain(&self.status).chain(self.detail.iter()).find(|notice| {
            notice.source == source && notice.id == id
        })
    }

    fn is_status(&self, source: &str, id: &str) -> bool {
        self.status.iter().any(|card| card.source == source && card.id == id)
    }

    fn change_status(&mut self, source: &str, next: Option<Completion>) {
        let Some(index) = self.status.iter().position(|card| card.source == source) else {
            if let Some(card) = next {
                self.status.push(card);
            }
            return;
        };
        let previous = &self.status[index];
        let same_action = next.as_ref().is_some_and(|card| card.id == previous.id && card.session == previous.session);
        if !same_action {
            self.opening.retain(|(s, i)| s != source || i != &previous.id);
        }
        if self.detail.as_ref().is_some_and(|card| card.source == source && card.id == previous.id) {
            self.detail = if same_action { next.clone() } else { None };
        }
        if let Some(card) = next {
            self.status[index] = card;
        } else {
            self.status.remove(index);
        }
    }
}

impl App {
    pub(crate) fn receive_plugin_notice(&mut self, event: Event) {
        if !self.settings.notifications {
            return;
        }
        match event {
            Event::Completed(mut completion) => {
                if self.plugin_notice_ui.find(&completion.source, &completion.id).is_some() {
                    return;
                }
                if !self.settings.notification_preview {
                    hide_content(&mut completion);
                }
                let alert = Alert {
                    title: completion.name.clone(),
                    source: AlertSource::Plugin { source: completion.source.clone(), id: completion.id.clone() },
                    body: String::new(),
                    page: usize::MAX,
                    at: self.t,
                    duration: 4.5,
                    bluetooth: None,
                    meeting: None,
                    unread: None,
                };
                let source = &completion.source;
                let oldest_from_source = self.plugin_notice_ui.unread.iter().enumerate()
                    .filter(|(_, notice)| &notice.source == source)
                    .nth(MAX_UNREAD_PER_SOURCE - 1)
                    .map(|(index, _)| index);
                if let Some(index) = oldest_from_source {
                    let removed = self.plugin_notice_ui.unread.remove(index).unwrap();
                    self.clear_plugin_notice_alerts(Some(&removed.source), Some(&removed.id));
                } else if self.plugin_notice_ui.unread.len() == MAX_UNREAD {
                    let removed = self.plugin_notice_ui.unread.pop_back().unwrap();
                    self.clear_plugin_notice_alerts(Some(&removed.source), Some(&removed.id));
                }
                self.plugin_notice_ui.unread.push_front(completion);
                self.enqueue_alert(alert, false);
            }
            Event::StatusChanged { source, mut card } => {
                if !self.settings.notification_preview {
                    if let Some(card) = &mut card {
                        hide_content(card);
                    }
                }
                self.plugin_notice_ui.change_status(&source, card);
            }
            Event::Opened { source, id, ok } => {
                let previous = self.plugin_notice_ui.opening.len();
                self.plugin_notice_ui.opening.retain(|(s, i)| *s != source || *i != id);
                if self.plugin_notice_ui.opening.len() == previous {
                    return;
                }
                let status = self.plugin_notice_ui.is_status(&source, &id);
                if ok {
                    if !status {
                        self.mark_plugin_notice_read(&source, &id);
                        if self.plugin_notice_ui.detail.as_ref().is_some_and(|notice| notice.source == source && notice.id == id) {
                            self.plugin_notice_ui.detail = None;
                        }
                    }
                } else {
                    let generic = status || self.plugin_notice_ui.find(&source, &id).is_some_and(|notice| notice.title.is_some());
                    self.messenger_unavailable(if generic { "暂时无法执行操作" } else { "暂时无法打开会话" });
                }
            }
        }
    }

    pub(crate) fn remove_plugin_notice_source(&mut self, source: &str) {
        self.plugin_notice_ui.unread.retain(|notice| notice.source != source);
        self.plugin_notice_ui.status.retain(|card| card.source != source);
        self.plugin_notice_ui.opening.retain(|(id, _)| id != source);
        if self.plugin_notice_ui.detail.as_ref().is_some_and(|notice| notice.source == source) {
            self.plugin_notice_ui.detail = None;
        }
        self.clear_plugin_notice_alerts(Some(source), None);
    }

    fn mark_plugin_notice_read(&mut self, source: &str, id: &str) {
        self.plugin_notice_ui.unread.retain(|notice| notice.source != source || notice.id != id);
        self.clear_plugin_notice_alerts(Some(source), Some(id));
    }

    fn clear_plugin_notice_alerts(&mut self, source: Option<&str>, id: Option<&str>) {
        let matches_notice = |alert: &Alert| match &alert.source {
            AlertSource::Plugin { source: s, id: i } => source.is_none_or(|want| want == s) && id.is_none_or(|want| want == i),
            _ => false,
        };
        self.alerts.retain(|alert| !matches_notice(alert));
        if self.notice.as_ref().is_some_and(matches_notice) {
            self.notice = None;
        }
        if self.hud_notice.as_ref().is_some_and(matches_notice) {
            self.hud_notice = None;
        }
    }

    pub(crate) fn plugin_notice_detail_open(&self) -> bool {
        self.plugin_notice_ui.detail.is_some()
    }

    pub(crate) fn tick_plugin_notices(&mut self, ctx: &egui::Context, env: &Env) {
        if !self.settings.notifications {
            self.plugin_notice_ui = NoticeUi::default();
            self.clear_plugin_notice_alerts(None, None);
            return;
        }
        if !self.settings.notification_preview || env.screen_locked {
            self.plugin_notice_ui.hide_content();
        }
        if self.plugin_notice_detail_open() && (env.screen_locked || env.fullscreen_hidden
            || ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))) {
            self.plugin_notice_ui.detail = None;
        }
    }

    fn show_plugin_notice_detail(&mut self, source: &str, id: &str) {
        if let Some(notice) = self.plugin_notice_ui.find(source, id).cloned() {
            if !self.plugin_notice_ui.is_status(source, id) {
                self.mark_plugin_notice_read(source, id);
            }
            self.plugin_notice_ui.detail = Some(notice);
            self.reveal_unread();
        }
    }

    fn open_plugin_notice(&mut self, source: &str, id: &str, cmds: &mut Vec<Cmd>) {
        if self.plugin_notice_ui.opening.iter().any(|(s, i)| s == source && i == id) {
            return;
        }
        let session = self.plugin_notice_ui.find(source, id).and_then(|notice| notice.session.clone());
        if let Some(session) = session.filter(|value| !value.is_empty()) {
            self.plugin_notice_ui.opening.push((source.into(), id.into()));
            cmds.push(Cmd::OpenPluginNotice { source: source.into(), id: id.into(), session });
        } else {
            self.show_plugin_notice_detail(source, id);
        }
    }

    pub(crate) fn draw_plugin_notice_banner(&mut self, c: &Card, alert: &Alert, source: &str, id: &str, cmds: &mut Vec<Cmd>) {
        let Some(notice) = self.plugin_notice_ui.find(source, id) else { return; };
        source_icon(c, 22.0, 22.0, &alert.title);
        c.centered_text(42.0, 14.0, &c.fit(&alert.title, 11.0, W::Semibold, 84.0), 11.0, W::Semibold, theme::WHITE, H::Left);
        let preview = self.settings.notification_preview && !notice.markdown.trim().is_empty();
        let session = notice.session.is_some();
        let caption = notice_title(notice, false);
        let caption_width = if session && preview { 84.0 } else if session || preview { 154.0 } else { 223.0 };
        c.centered_text(42.0, 30.0, &c.fit(caption, 10.5, W::Regular, caption_width), 10.5, W::Regular, theme::GRAY, H::Left);
        let open = session && button(c, 204.0, 10.0, 64.0, action_label(notice, false), ("notice-banner-open", source, id));
        let details_label = if notice.title.is_some() { "查看详情" } else { "查看回复" };
        let details = preview && button(c, if session { 132.0 } else { 204.0 }, 10.0, 66.0, details_label, ("notice-banner-body", source, id));
        let close = button(c, 274.0, 10.0, 47.0, "关闭", ("notice-banner-close", source, id));
        if close {
            self.notice = None;
        } else if details {
            self.show_plugin_notice_detail(source, id);
        } else if open {
            self.open_plugin_notice(source, id, cmds);
        }
    }

    pub(crate) fn draw_plugin_notice_capsule(&mut self, c: &Card, offset: usize, slot: f32, height: f32, scroll: f32, width: f32, cmds: &mut Vec<Cmd>) {
        let mut clicked = None;
        for (index, (notice, status)) in self.plugin_notice_ui.entries().enumerate() {
            let x = width / 2.0;
            let y = 2.0 + slot * ((offset + index) as f32 + 0.5) - scroll;
            if y < -slot || y > height + slot { continue; }
            source_icon(c, x, y, &notice.name);
            c.circle(x + 10.0, y - 9.0, 2.6, if status { STATUS_DOT } else { RED_DOT });
            if c.hit(0.0, y - slot / 2.0, width, slot, ("plugin-capsule", &notice.source, &notice.id))
                .on_hover_text(format!("{} · {}", notice.name, notice_title(notice, status))).clicked() {
                clicked = Some((notice.source.clone(), notice.id.clone(), status));
            }
        }
        if let Some((source, id, status)) = clicked {
            if status {
                self.show_plugin_notice_detail(&source, &id);
            } else {
                self.open_plugin_notice(&source, &id, cmds);
            }
        }
    }

    pub(crate) fn draw_plugin_notice_stack(&mut self, c: &Card, horizontal: bool, offset: usize, step: f32, length: f32, cmds: &mut Vec<Cmd>) {
        let sources = self.plugin_notice_ui.sources();
        let count = sources.len().min(COLLAPSED_BADGES);
        let mut expand = false;
        let mut clicked = None;
        for (i, &(notice, status)) in sources.iter().take(count).enumerate() {
            let index = offset + i;
            let (x, y) = if horizontal { (16.0 + step * index as f32, 14.0) } else { (16.0, 14.0 + step * index as f32) };
            let multiple_sources = i + 1 == count && sources.len() > count;
            if multiple_sources {
                c.centered_text(x, y, "+", 14.0, W::Semibold, theme::WHITE, H::Center);
            } else {
                source_icon(c, x, y, &notice.name);
            }
            c.circle(x + 8.0, y - 7.0, 2.6, if status { STATUS_DOT } else { RED_DOT });
            let center = if horizontal { x } else { y };
            let start = if index == 0 { 0.0 } else { center - step / 2.0 };
            let end = if i + 1 == count { length } else { center + step / 2.0 };
            let id = ("plugin-stack", &notice.source);
            let hit = if horizontal { c.hit(start, 0.0, end - start, BAR_H, id) } else { c.hit(0.0, start, 32.0, end - start, id) };
            let multiple_notices = self.plugin_notice_ui.entries().filter(|(item, _)| item.source == notice.source).take(2).count() > 1;
            let label = if multiple_sources || multiple_notices { "展开查看提醒与状态".into() } else { format!("{} · {}", notice.name, notice_title(notice, status)) };
            if hit.on_hover_text(label).clicked() {
                if multiple_sources || multiple_notices {
                    expand = true;
                } else {
                    clicked = Some((notice.source.clone(), notice.id.clone(), status));
                }
            }
        }
        if expand {
            self.reveal_unread();
        } else if let Some((source, id, status)) = clicked {
            if status {
                self.show_plugin_notice_detail(&source, &id);
            } else {
                self.open_plugin_notice(&source, &id, cmds);
            }
        }
    }

    pub(crate) fn draw_plugin_notice_detail(&mut self, c: &Card, cmds: &mut Vec<Cmd>) {
        let Some(notice) = self.plugin_notice_ui.detail.as_ref() else { return; };
        let status = self.plugin_notice_ui.is_status(&notice.source, &notice.id);
        source_icon(c, 24.0, 27.0, &notice.name);
        let heading = format!("{} · {}", notice.name, notice_title(notice, status));
        c.centered_text(43.0, 27.0, &c.fit(&heading, size::TITLE, W::Semibold, 220.0), size::TITLE, W::Semibold, theme::WHITE, H::Left);
        let close = button(c, 364.0, 15.0, 42.0, "关闭", "plugin-detail-close");
        let open = notice.session.is_some() && button(c, 287.0, 15.0, 70.0, action_label(notice, status), "plugin-detail-open");
        c.fill(15.0, 46.0, 389.0, 144.0, 9.0, theme::CARD);
        let rect = c.rect(27.0, 55.0, 365.0, 127.0);
        let body = if !self.settings.notification_preview {
            "通知正文已隐藏"
        } else if notice.markdown.trim().is_empty() {
            if notice.title.is_some() { "暂无详情" } else { "没有可预览的回复正文" }
        } else {
            &notice.markdown
        };
        let scroll_id = egui::Id::new(("plugin-notice-markdown", &notice.source, &notice.id));
        let mut ui = egui::Ui::new(c.ui.ctx().clone(), scroll_id, egui::UiBuilder::new()
            .max_rect(rect).layer_id(c.painter.layer_id()).style(c.ui.style().clone()));
        ui.set_clip_rect(rect.intersect(c.painter.clip_rect()));
        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(rect.height()).show(&mut ui, |ui| {
            draw_markdown(ui, body);
        });
        if close {
            self.plugin_notice_ui.detail = None;
        } else if open {
            let (source, id) = (notice.source.clone(), notice.id.clone());
            self.open_plugin_notice(&source, &id, cmds);
        }
    }
}

fn hide_content(notice: &mut Completion) {
    if let Some(title) = &mut notice.title {
        title.clear();
    }
    notice.markdown.clear();
    notice.action_label = None;
}

fn notice_title(notice: &Completion, status: bool) -> &str {
    match notice.title.as_deref() {
        Some(title) if !title.is_empty() => title,
        _ if status => "当前状态",
        Some(_) => "收到新通知",
        None => "本轮已结束",
    }
}

fn action_label(notice: &Completion, status: bool) -> &str {
    notice.action_label.as_deref().unwrap_or(if status { "执行操作" } else if notice.title.is_some() { "打开" } else { "回会话" })
}

fn button(c: &Card, x: f32, y: f32, width: f32, text: &str, id: impl std::hash::Hash) -> bool {
    let hit = c.hit(x, y, width, 24.0, id);
    let (hover, press) = c.hover_t(&hit);
    let inner = c.scaled(x + width / 2.0, y + 12.0, 1.0 - 0.05 * press);
    inner.fill(x, y, width, 24.0, 12.0, paint::mix(theme::BTN, theme::BTN_HOVER, hover));
    inner.centered_text(x + width / 2.0, y + 12.0, &c.fit(text, size::SMALL, W::Semibold, width - 10.0), size::SMALL, W::Semibold, theme::WHITE, H::Center);
    hit.clicked()
}

fn source_icon(c: &Card, x: f32, y: f32, name: &str) {
    c.circle(x, y, 9.0, theme::BTN);
    let letter = name.chars().find(|ch| ch.is_alphanumeric()).unwrap_or('·');
    c.centered_text(x, y, &letter.to_string(), 10.5, W::Bold, theme::WHITE, H::Center);
}

fn draw_markdown(ui: &mut egui::Ui, text: &str) {
    let mut code = false;
    for (index, line) in text.lines().enumerate() {
        if index == 128 {
            ui.label("其余内容已省略");
            break;
        }
        let line = line.trim_end();
        if line.trim_start().starts_with("```") {
            code = !code;
            continue;
        }
        if line.is_empty() {
            ui.add_space(4.0);
            continue;
        }
        let line = if code {
            egui::RichText::new(line).monospace().size(size::BODY)
        } else if let Some((prefix, title)) = line.split_once(' ').filter(|(prefix, _)| {
            (1..=6).contains(&prefix.len()) && prefix.bytes().all(|byte| byte == b'#')
        }) {
            let font = if prefix.len() < 3 { size::TITLE } else { size::BODY };
            egui::RichText::new(title).font(theme::font(font, W::Semibold))
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            egui::RichText::new(format!("•  {item}")).size(size::BODY)
        } else {
            egui::RichText::new(line).size(size::BODY)
        };
        ui.add(egui::Label::new(line.color(theme::WHITE)).wrap().selectable(false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(source: &str, id: &str) -> Completion {
        Completion {
            source: source.into(),
            name: "示例助手".into(),
            id: id.into(),
            session: Some("session-1".into()),
            markdown: "# 私有回复".into(),
            title: None,
            action_label: None,
        }
    }

    fn completion(id: &str) -> Event {
        Event::Completed(notice("example.agent", id))
    }

    #[test]
    fn plugin_notice_remains_unread_until_open_is_confirmed() {
        let _guard = crate::ui_tests::ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, Default::default());
        app.settings.notifications = true;
        app.receive_plugin_notice(completion("one"));
        app.receive_plugin_notice(completion("two"));
        app.tick_alerts();
        app.t += 5.0;
        app.tick_alerts();
        assert!(!matches!(&app.notice, Some(alert) if matches!(&alert.source, AlertSource::Plugin { id, .. } if id == "one")));
        assert_eq!(app.plugin_notice_ui.unread.len(), 2);

        let mut cmds = Vec::new();
        app.open_plugin_notice("example.agent", "one", &mut cmds);
        app.open_plugin_notice("example.agent", "one", &mut cmds);
        assert!(matches!(cmds.as_slice(), [Cmd::OpenPluginNotice { id, session, .. }] if id == "one" && session == "session-1"));
        app.receive_plugin_notice(Event::Opened { source: "example.agent".into(), id: "one".into(), ok: false });
        assert_eq!(app.plugin_notice_ui.unread.len(), 2);

        app.open_plugin_notice("example.agent", "one", &mut cmds);
        app.receive_plugin_notice(Event::Opened { source: "example.agent".into(), id: "one".into(), ok: true });
        assert_eq!(app.plugin_notice_ui.unread.len(), 1);
        assert_eq!(app.plugin_notice_ui.unread[0].id, "two");
    }

    #[test]
    fn plugin_notice_drops_private_content_when_preview_is_disabled() {
        let _guard = crate::ui_tests::ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, Default::default());
        app.settings.notifications = true;
        app.settings.notification_preview = true;
        app.receive_plugin_notice(completion("one"));
        app.show_plugin_notice_detail("example.agent", "one");
        assert!(app.plugin_notice_ui.unread.is_empty());
        assert_eq!(app.plugin_notice_ui.detail.as_ref().unwrap().markdown, "# 私有回复");

        app.settings.notification_preview = false;
        app.tick_plugin_notices(&ctx, &Env::default());
        app.receive_plugin_notice(completion("two"));
        assert!(app.plugin_notice_ui.detail.as_ref().unwrap().markdown.is_empty());
        assert!(app.plugin_notice_ui.unread[0].markdown.is_empty());
        assert!(app.history.iter().all(|alert| !alert.body.contains("私有回复")));
        app.remove_plugin_notice_source("example.agent");
        assert!(app.plugin_notice_ui.unread.is_empty());
        assert!(!app.plugin_notice_detail_open());
    }

    #[test]
    fn plugin_status_updates_in_place_and_actions_do_not_consume_it() {
        let _guard = crate::ui_tests::ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, Default::default());
        app.settings.notifications = true;
        app.settings.notification_preview = true;
        let source = "example.agent";
        let mut card = notice(source, "state-one");
        card.title = Some("正在处理".into());
        card.action_label = Some("打开详情".into());
        app.receive_plugin_notice(Event::StatusChanged { source: source.into(), card: Some(card.clone()) });
        assert_eq!(app.plugin_notice_ui.item_count(), 1);
        assert!(app.plugin_notice_ui.unread.is_empty());
        assert!(app.alerts.is_empty());

        app.show_plugin_notice_detail(source, "state-one");
        let mut cmds = Vec::new();
        app.open_plugin_notice(source, "state-one", &mut cmds);
        assert!(matches!(cmds.as_slice(), [Cmd::OpenPluginNotice { id, .. }] if id == "state-one"));
        card.title = Some("处理进度已更新".into());
        app.receive_plugin_notice(Event::StatusChanged { source: source.into(), card: Some(card.clone()) });
        assert_eq!(app.plugin_notice_ui.detail.as_ref().unwrap().title, card.title);
        app.receive_plugin_notice(Event::Opened { source: source.into(), id: card.id.clone(), ok: true });
        assert!(app.plugin_notice_detail_open());
        assert_eq!(app.plugin_notice_ui.item_count(), 1);

        app.settings.notification_preview = false;
        app.tick_plugin_notices(&ctx, &Env::default());
        let hidden = app.plugin_notice_ui.detail.as_ref().unwrap();
        assert_eq!(notice_title(hidden, true), "当前状态");
        assert_eq!(action_label(hidden, true), "执行操作");
        assert!(hidden.markdown.is_empty());
        app.open_plugin_notice(source, "state-one", &mut cmds);
        card.session = Some("session-2".into());
        app.receive_plugin_notice(Event::StatusChanged { source: source.into(), card: Some(card) });
        assert!(!app.plugin_notice_detail_open());
        assert!(app.plugin_notice_ui.opening.is_empty());
        app.receive_plugin_notice(Event::Opened { source: source.into(), id: "state-one".into(), ok: true });
        assert_eq!(app.plugin_notice_ui.item_count(), 1);

        app.show_plugin_notice_detail(source, "state-one");
        app.receive_plugin_notice(Event::StatusChanged { source: source.into(), card: None });
        assert!(!app.plugin_notice_detail_open());
        assert_eq!(app.plugin_notice_ui.item_count(), 0);
    }

    #[test]
    fn plugin_notice_source_cannot_evict_other_unread_or_builtin_alerts() {
        let _guard = crate::ui_tests::ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, Default::default());
        app.settings.notifications = true;
        for index in 0..7 {
            app.alert(format!("内置提醒 {index}"), "正文", 0, false);
        }
        app.receive_plugin_notice(Event::Completed(notice("example.quiet", "keep")));
        for index in 0..MAX_UNREAD + 1 {
            app.receive_plugin_notice(completion(&index.to_string()));
        }
        assert_eq!(app.plugin_notice_ui.unread.len(), MAX_UNREAD_PER_SOURCE + 1);
        assert!(app.plugin_notice_ui.unread.iter().any(|notice| notice.source == "example.quiet"));
        app.alert("最新内置提醒", "正文", 0, false);
        assert_eq!(app.alerts.len(), 8);
        assert!(app.alerts.iter().all(|alert| alert.source == AlertSource::Internal));
        assert_eq!(app.alerts.front().unwrap().title, "内置提醒 0");

        for index in 0..MAX_SOURCES - 2 {
            let source = format!("example.other-{index}");
            for id in 0..MAX_UNREAD_PER_SOURCE {
                app.receive_plugin_notice(Event::Completed(notice(&source, &id.to_string())));
            }
        }
        for index in 0..MAX_UNREAD_PER_SOURCE {
            app.receive_plugin_notice(Event::Completed(notice("example.quiet", &index.to_string())));
        }
        assert_eq!(app.plugin_notice_ui.unread.len(), MAX_UNREAD);
        for index in 0..MAX_SOURCES - 2 {
            let source = format!("example.other-{index}");
            assert_eq!(app.plugin_notice_ui.unread.iter().filter(|notice| notice.source == source).count(),
                MAX_UNREAD_PER_SOURCE);
        }
    }
}
