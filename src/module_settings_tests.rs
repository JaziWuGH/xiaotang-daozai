use super::*;

#[test]
fn disabled_module_settings_dismiss_existing_popups_and_allow_reenabling() {
    use crate::app::{CALENDAR_PAGE, MAIL_PAGE, PAGE_SETTINGS_TABS, QUOTA_PAGE, WEATHER_PAGE};
    let _g=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());
    for page in [MAIL_PAGE,CALENDAR_PAGE,QUOTA_PAGE,WEATHER_PAGE] {
        let mut h=H::new();
        h.app.set_page_visible(page,false);
        h.app.settings_tab=PAGE_SETTINGS_TABS[page];
        h.app.open_settings();
        h.ctx.set_embed_viewports(false);
        match page {
            MAIL_PAGE=>h.app.mail.form.open=true,
            CALENDAR_PAGE=>h.app.calendar.form.open=true,
            QUOTA_PAGE=>h.app.quota.connect_open=true,
            WEATHER_PAGE=>h.app.weather_source.tutorial_open=true,
            _=>unreachable!(),
        }

        for _ in 0..3 {draw_page_management(&mut h,vec![]);}
        assert!(!h.app.mail.form.open && !h.app.calendar.form.open
            && !h.app.quota.connect_open && !h.app.weather_source.tutorial_open,
            "关闭的模块应收起旧设置弹窗：{page}");
        let enable=h.shapes.iter().find_map(|shape|match &shape.shape {
            egui::Shape::Text(text) if text.galley.text()=="开启" =>Some(text.visual_bounding_rect().center()),
            _=>None,
        }).expect("关闭的功能页应保留开启入口");
        for pressed in [true,false] {
            draw_page_management(&mut h,vec![egui::Event::PointerMoved(enable),egui::Event::PointerButton{
                pos:enable,button:egui::PointerButton::Primary,pressed,modifiers:Default::default(),
            }]);
        }
        assert!(h.app.module_on(page),"独立设置页应可重新启用模块：{page}");
        assert!(crate::model::Settings::load().visible_pages[page]);
    }
}
