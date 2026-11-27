use super::{egui, plugin_switch, settings_control_row, Field, Values};
use crate::{
    extras::{settings_segmented, time_stepper, SETTINGS_LABEL_W},
    plugin_setting_catalog::{setting_color, timezone_cities, COLORS, ICONS},
};
use chrono::{Datelike, Local, Months, NaiveDate, Timelike};

pub(super) fn field_row(ui: &mut egui::Ui, plugin: &str, field: &Field, value: &mut String) -> bool {
    ui.push_id(&field.key, |ui| {
        if field.kind == "list" {
            list_field(ui, plugin, field, value)
        } else {
            let label = field_label(plugin, field);
            let label_width = ui.fonts(|fonts| fonts.layout_no_wrap(
                label.to_owned(), egui::TextStyle::Body.resolve(ui.style()), ui.visuals().text_color(),
            ).size().x);
            let mut submit = false;
            let mut controls = |ui: &mut egui::Ui| {
                if field.multiline == Some(true) {
                    submit = multiline_text(ui, value);
                } else {
                    editor(ui, plugin, field, value);
                }
            };
            if ui.available_width() < 290.0 || label_width > SETTINGS_LABEL_W - 6.0 {
                ui.label(label);
                controls(ui);
            } else {
                settings_control_row(ui, label, SETTINGS_LABEL_W, |ui| ui.scope(controls).response);
            }
            submit
        }
    }).inner
}

fn field_label<'a>(plugin: &str, field: &'a Field) -> &'a str {
    match (plugin, field.key.as_str()) {
        ("daozai.gold", "alert_usd") => "美元提醒价",
        ("daozai.gold", "alert_cny") => "人民币提醒价",
        ("daozai.live-pay", "salary") => "月薪",
        ("daozai.live-pay", "paid") => "照常计薪",
        ("daozai.live-pay", "pinned_show") => "显示内容",
        ("daozai.world-clock", "name") => "城市名称",
        ("daozai.days", "name") => "名称",
        ("daozai.days", "pinned_text") => "常驻文字",
        ("daozai.days", "pinned_day") => "常驻日子",
        _ => &field.label,
    }
}

fn editor(ui: &mut egui::Ui, plugin: &str, field: &Field, value: &mut String) {
    match field.kind.as_str() {
        "switch" => {
            let on = value == "true";
            if plugin_switch(ui, &format!("{plugin}:{}", field.key), on, field_label(plugin, field)) {
                *value = (!on).to_string();
            }
        }
        "choice" => {
            let choices = field.choices.as_ref().unwrap();
            let options = choices
                .iter()
                .map(|choice| {
                    let label = match (plugin, field.key.as_str(), choice.value.as_str()) {
                        ("daozai.live-pay", "pinned_show", "amount") => "今日金额",
                        ("daozai.live-pay", "pinned_show", "countdown") => "倒计时",
                        ("daozai.live-pay", "pinned_show", "alternate") => "轮换",
                        ("daozai.world-clock", "pinned_city", "first") => "列表第一城",
                        ("daozai.world-clock", "pinned_city", "selected") => "页面选中",
                        _ => choice.label.as_str(),
                    };
                    (choice.value.as_str(), label)
                })
                .collect::<Vec<_>>();
            let mut selected = value.as_str();
            if options.len() <= 4 {
                settings_segmented(ui, &mut selected, &options, 24.0);
            } else {
                let label = options
                    .iter()
                    .find_map(|(key, label)| (*key == selected).then_some(*label));
                egui::ComboBox::from_id_salt(&field.key)
                    .selected_text(label.unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (key, label) in options {
                            ui.selectable_value(&mut selected, key, label);
                        }
                    });
            }
            if selected != value.as_str() {
                *value = selected.to_owned();
            }
        }
        "number" => {
            let min = f64::from(field.min.unwrap());
            let mut number = value.parse::<f64>().unwrap_or(min);
            let suffix = match (plugin, field.key.as_str()) {
                ("daozai.gold", "alert_usd") => " 美元/盎司",
                ("daozai.gold", "alert_cny") => " 元/克",
                ("daozai.live-pay", "salary") => " 元",
                _ => field.unit.as_deref().unwrap_or_default(),
            };
            let mut control = egui::DragValue::new(&mut number)
                .range(min..=f64::from(field.max.unwrap()))
                .speed(f64::from(field.step.unwrap()))
                .suffix(suffix);
            if let Some(precision) = field.precision {
                control = control.fixed_decimals(precision.into());
            }
            let response = ui.scope(|ui| {
                if matches!(plugin, "daozai.gold" | "daozai.live-pay") {
                    ui.spacing_mut().interact_size = egui::vec2(124.0, 28.0);
                }
                ui.add(control)
            }).inner;
            if response.changed() {
                *value = number.to_string();
            }
        }
        "time" => {
            let time = chrono::NaiveTime::parse_from_str(value, "%H:%M").unwrap_or_default();
            let mut minutes = time.hour() * 60 + time.minute();
            if time_stepper(ui, &mut minutes, &field.key) {
                *value = format!("{:02}:{:02}", minutes / 60, minutes % 60);
            }
        }
        "date" => date_editor(ui, value),
        "timezone" => timezone_picker(ui, &field.key, value),
        "icon" => {
            let selected = ICONS.iter().find(|icon| icon.name == value.as_str()).map_or("", |icon| icon.label);
            let [r, g, b, _] = ui.visuals().text_color().to_array();
            egui::ComboBox::from_id_salt(&field.key).selected_text(selected).show_ui(ui, |ui| {
                for icon in ICONS {
                    ui.horizontal(|ui| {
                        let svg = format!(
                            r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="{}" fill="none" stroke="#{r:02x}{g:02x}{b:02x}" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>"##,
                            icon.path,
                        );
                        ui.add(egui::Image::from_bytes(
                            format!("bytes://plugin-setting-{}-{r:02x}{g:02x}{b:02x}.svg", icon.name),
                            svg.into_bytes(),
                        ).fit_to_exact_size(egui::vec2(18.0, 18.0)));
                        if ui.selectable_label(value == icon.name, icon.label).clicked() {
                            *value = icon.name.to_owned();
                        }
                    });
                }
            });
        }
        "color" => {
            let selected = setting_color(value).map_or("", |color| color.label);
            egui::ComboBox::from_id_salt(&field.key).selected_text(selected).show_ui(ui, |ui| {
                for color in COLORS {
                    ui.horizontal(|ui| {
                        let (swatch, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        let [r, g, b] = color.rgb;
                        ui.painter().circle_filled(swatch.center(), 5.0, egui::Color32::from_rgb(r, g, b));
                        if ui.selectable_label(value == color.name, color.label).clicked() {
                            *value = color.name.to_owned();
                        }
                    });
                }
            });
        }
        "weekdays" => {
            let mut days = super::script_settings::weekdays(value);
            ui.horizontal_wrapped(|ui| {
                for (index, caption) in ["一", "二", "三", "四", "五", "六", "日"]
                    .into_iter()
                    .enumerate()
                {
                    let day = index as u8 + 1;
                    let on = days.contains(&day);
                    let button = egui::Button::new(caption)
                        .selected(on)
                        .min_size(egui::vec2(24.0, 24.0))
                        .corner_radius(12.0);
                    if ui
                        .add(button)
                        .on_hover_text(format!("星期{caption}"))
                        .clicked()
                    {
                        if on {
                            days.remove(&day);
                        } else {
                            days.insert(day);
                        }
                        *value = days.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
                    }
                }
            });
        }
        _ => text_editor(ui, plugin, field, value),
    }
}

fn text_editor(ui: &mut egui::Ui, plugin: &str, field: &Field, value: &mut String) {
    let (hint, limit, empty_default) = match (plugin, field.key.as_str()) {
        ("daozai.world-clock", "name") => ("留空使用时区默认名", 80, true),
        ("daozai.days", "name") => ("最多 12 字", 12, false),
        ("daozai.days", "pinned_day") => ("最近的那一天，或填写名称", 80, true),
        ("daozai.days", "pinned_text") => ("留空使用默认文字", 24, true),
        _ => {
            ui.text_edit_singleline(value);
            return;
        }
    };
    let mut text = if empty_default && *value == field.default { String::new() } else { value.clone() };
    let width = ui.available_width().min(260.0);
    let input = egui::TextEdit::singleline(&mut text).hint_text(hint)
        .char_limit(limit).desired_width(width);
    let response = ui.add(input);
    let response = if plugin == "daozai.days" && field.key == "pinned_text" {
        response.on_hover_text("可插入 {名称}、{天数}、{日期}、{星期}")
    } else {
        response
    };
    if response.changed() {
        *value = if empty_default && text.trim().is_empty() { field.default.clone() } else { text };
    }
}

fn date_editor(ui: &mut egui::Ui, value: &mut String) {
    let id = ui.make_persistent_id("date-month");
    ui.horizontal_wrapped(|ui| {
        let input = egui::TextEdit::singleline(value).hint_text("年-月-日").desired_width(104.0);
        if ui.add(input).changed() {
            ui.data_mut(|data| data.remove::<NaiveDate>(id));
        }
        ui.menu_button("选择日期", |ui| {
            let selected = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok();
            let mut month = ui.data(|data| data.get_temp::<NaiveDate>(id))
                .unwrap_or_else(|| selected.unwrap_or_else(|| Local::now().date_naive()));
            month = month.with_day(1).unwrap_or(month);
            ui.horizontal(|ui| {
                if ui.button("‹").on_hover_text("上个月").clicked() {
                    month = month.checked_sub_months(Months::new(1)).unwrap_or(month);
                }
                ui.label(format!("{}年{}月", month.year(), month.month()));
                if ui.button("›").on_hover_text("下个月").clicked() {
                    month = month.checked_add_months(Months::new(1)).unwrap_or(month);
                }
            });
            ui.data_mut(|data| data.insert_temp(id, month));
            egui::Grid::new(id.with("days")).spacing([3.0, 3.0]).show(ui, |ui| {
                for weekday in ["一", "二", "三", "四", "五", "六", "日"] {
                    ui.weak(weekday);
                }
                ui.end_row();
                let first = month.weekday().num_days_from_monday();
                for index in 0..42 {
                    let date = (index >= first).then(|| month.with_day(index - first + 1)).flatten();
                    if let Some(date) = date {
                        let button = egui::Button::new(date.day().to_string()).selected(selected == Some(date));
                        if ui.add_sized([25.0, 23.0], button).clicked() {
                            *value = date.to_string();
                            ui.data_mut(|data| data.remove::<NaiveDate>(id));
                            ui.close_menu();
                        }
                    } else {
                        ui.allocate_space(egui::vec2(25.0, 23.0));
                    }
                    if index % 7 == 6 { ui.end_row(); }
                }
            });
        });
    });
}

fn timezone_picker(ui: &mut egui::Ui, key: &str, value: &mut String) {
    let search_id = ui.make_persistent_id(("timezone-search", key));
    let width = ui.available_width().min(210.0).max(72.0);
    egui::ComboBox::from_id_salt(key)
        .selected_text(value.as_str())
        .width(width)
        .close_behavior(egui::popup::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            let mut query = ui.data(|data| data.get_temp::<String>(search_id)).unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut query)
                .hint_text("搜索城市、国家或时区")
                .desired_width(220.0)).changed() {
                ui.data_mut(|data| data.insert_temp(search_id, query.clone()));
            }
            let matches: Vec<_> = timezone_cities(&query).collect();
            if matches.is_empty() {
                ui.weak("没有符合条件的时区");
            }
            let row_height = ui.spacing().interact_size.y
                .max(ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().button_padding.y * 2.0);
            egui::ScrollArea::vertical().id_salt(search_id).max_height(170.0)
                .show_rows(ui, row_height, matches.len(), |ui, rows| {
                for city in &matches[rows] {
                    let label = format!("{} · {} · {}", city.city, city.region, city.zone);
                    if ui.selectable_label(value == city.zone, label).clicked() {
                        *value = city.zone.to_owned();
                        ui.data_mut(|data| data.remove::<String>(search_id));
                        ui.memory_mut(|memory| memory.close_popup());
                    }
                }
            });
        });
}

fn multiline_text(ui: &mut egui::Ui, value: &mut String) -> bool {
    let id = ui.make_persistent_id("multiline");
    let submit = ui.memory(|memory| memory.has_focus(id))
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter));
    let width = ui.available_width().min(370.0);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    let max_rows = 6;
    let rows = ui.fonts(|fonts| fonts.layout(value.clone(), font.clone(), color, (width - 8.0).max(1.0)).rows.len());
    let row_height = ui.text_style_height(&egui::TextStyle::Body);
    let height = row_height * rows.clamp(1, max_rows) as f32 + 8.0;
    let response = egui::ScrollArea::vertical().id_salt(id).max_height(height).auto_shrink([true, false])
        .show(ui, |ui| ui.add(egui::TextEdit::multiline(value).id(id).font(font).desired_width(width).desired_rows(1).char_limit(80)))
        .inner;
    if response.changed() {
        value.retain(|ch| ch == '\n' || !ch.is_control());
    }
    submit
}

fn list_add_label(plugin: &str, key: &str) -> Option<&'static str> {
    match (plugin, key) {
        ("daozai.live-pay", "breaks") => Some("添加休息时段"),
        ("daozai.world-clock", "cities") => Some("添加城市"),
        ("daozai.days", "days") => Some("添加日子"),
        _ => None,
    }
}

fn list_item_title(plugin: &str, row: &Values) -> String {
    let get = |key| row.get(key).map(String::as_str).unwrap_or("");
    match plugin {
        "daozai.live-pay" => {
            let paid = if get("paid") == "true" { "计薪" } else { "不计薪" };
            format!("{}–{} · {paid}", get("from"), get("to"))
        }
        "daozai.world-clock" => {
            let zone = get("zone");
            let name = get("name");
            let name = if name.is_empty() { "时区默认名" } else { name };
            format!("{name} · {zone}")
        }
        _ => {
            let kind = match get("kind") {
                "anniversary" => "纪念日",
                "birthday" => "生日",
                _ => "倒数",
            };
            format!("{} · {kind} · {}", get("name"), get("date"))
        }
    }
}

fn list_field(ui: &mut egui::Ui, plugin: &str, field: &Field, value: &mut String) -> bool {
    let children = field.item.as_ref().unwrap();
    let mut rows: Vec<Values> = serde_json::from_str(value).unwrap_or_default();
    let add_label = list_add_label(plugin, &field.key);
    let open_id = ui.make_persistent_id("open-row");
    let mut open = ui.data(|data| data.get_temp::<usize>(open_id)).filter(|&index| index < rows.len());
    let mut edit = None;
    let mut submit = false;
    let max_items = field.max_items.unwrap();
    ui.horizontal_wrapped(|ui| {
        if add_label.is_none() { ui.label(&field.label); }
        if ui
            .add_enabled(rows.len() < max_items, egui::Button::new(add_label.unwrap_or("添加一行")))
            .clicked()
        {
            open = Some(rows.len());
            rows.push(
                children
                    .iter()
                    .map(|child| (child.key.clone(), child.default.clone()))
                    .collect(),
            );
        }
        if add_label.is_some() { ui.weak(format!("{} / {max_items}", rows.len())); }
    });
    let last = rows.len().saturating_sub(1);
    for (i, row) in rows.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            if add_label.is_some() {
                let selected = open == Some(i);
                let marker = if selected { "−" } else { "›" };
                let title = format!("{marker}  {}", list_item_title(plugin, row));
                if ui.add(egui::Button::new(title).selected(selected).wrap()).clicked() {
                    open = (!selected).then_some(i);
                }
                if open != Some(i) { return; }
            }
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(i > 0, egui::Button::new("上移")).clicked() {
                        edit = Some((i, Some(i - 1)));
                    }
                    if ui
                        .add_enabled(i < last, egui::Button::new("下移"))
                        .clicked()
                    {
                        edit = Some((i, Some(i + 1)));
                    }
                    if ui.button("删除").clicked() {
                        edit = Some((i, None));
                    }
                });
                for child in children {
                    let repeats = plugin == "daozai.days" && child.key == "yearly"
                        && row.get("kind").is_some_and(|kind| kind == "birthday");
                    let value = row.entry(child.key.clone()).or_insert_with(|| child.default.clone());
                    if !repeats { submit |= field_row(ui, plugin, child, value); }
                }
            });
        });
    }
    if let Some((i, target)) = edit {
        if let Some(target) = target {
            rows.swap(i, target);
            open = Some(target);
        } else {
            rows.remove(i);
            open = None;
        }
    }
    if let Some(index) = open {
        ui.data_mut(|data| data.insert_temp(open_id, index));
    } else {
        ui.data_mut(|data| data.remove::<usize>(open_id));
    }
    if let Ok(encoded) = serde_json::to_string(&rows) {
        *value = encoded;
    }
    submit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_setting_lists_show_the_saved_item_in_collapsed_state() {
        let title = |plugin, row| list_item_title(plugin, &serde_json::from_value(row).unwrap());
        assert_eq!(title("daozai.live-pay", serde_json::json!({
            "from":"12:00", "to":"13:30", "paid":"false"
        })), "12:00–13:30 · 不计薪");
        assert_eq!(title("daozai.world-clock", serde_json::json!({
            "name":"北京", "zone":"Asia/Shanghai"
        })), "北京 · Asia/Shanghai");
        assert_eq!(title("daozai.days", serde_json::json!({
            "name":"旅行", "kind":"countdown", "date":"2026-12-24"
        })), "旅行 · 倒数 · 2026-12-24");
    }

    #[test]
    fn multiline_settings_preserve_enter_and_submit_on_ctrl_enter() {
        let ctx = egui::Context::default();
        let mut value = String::new();
        let enter = |modifiers| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let frame = |events, value: &mut String| {
            let mut submitted = false;
            let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let id = ui.make_persistent_id("multiline");
                    ui.memory_mut(|memory| memory.request_focus(id));
                    submitted |= multiline_text(ui, value);
                });
            });
            submitted
        };
        assert!(!frame(vec![], &mut value));
        assert!(!frame(vec![egui::Event::Text("上".into()), enter(egui::Modifiers::NONE), egui::Event::Text("下".into())], &mut value));
        assert_eq!(value, "上\n下");
        assert!(frame(vec![enter(egui::Modifiers::CTRL)], &mut value));
        assert_eq!(value, "上\n下");
    }
}
