//! 天气页与天气设置：实况、七日预报与日照时段，图标全部用面板矢量绘制。
use crate::app::{App, Cmd, Env, WEATHER_PAGE, WEATHER_H};
use crate::paint::{self, Card, H};
use crate::theme::{self, W};
use crate::weather::{self, Kind};
use eframe::egui::{self, Color32};

const SUN: Color32 = Color32::from_rgb(255, 198, 88);
const CLOUD: Color32 = Color32::from_rgb(206, 214, 228);
const RAIN: Color32 = Color32::from_rgb(116, 176, 255);
const SNOW: Color32 = Color32::from_rgb(228, 238, 255);
const BOLT: Color32 = Color32::from_rgb(255, 214, 92);
const WARN: Color32 = Color32::from_rgb(255, 166, 77);

/// 常驻胶囊上迷你天气图标的边长。
const PILL_ICON: f32 = 15.0;

/// 胶囊上的温度文字：取整并带度符，负零归零。
fn pill_temp_text(snapshot: &crate::weather::Snapshot) -> String {
    format!("{}°", snapshot.temp.round() as i32)
}

fn sun(c: &Card, cx: f32, cy: f32, s: f32) {
    c.circle(cx, cy, s * 0.40, SUN);
    for i in 0..8 {
        let angle = i as f32 * std::f32::consts::TAU / 8.0;
        c.seg((cx + angle.cos() * s * 0.58, cy + angle.sin() * s * 0.58), (cx + angle.cos() * s * 0.90, cy + angle.sin() * s * 0.90), 1.7, paint::alpha(SUN, 0.9));
    }
}

/// 弯月：外圆与偏右内圆之间的三角带，保留透明月牙缺口。
fn moon(c: &Card, cx: f32, cy: f32, s: f32) {
    let (outer, inner, offset) = (0.62 * s, 0.55 * s, 0.30 * s);
    let x = (offset * offset + outer * outer - inner * inner) / (2.0 * offset);
    let y = (outer * outer - x * x).max(0.0).sqrt();
    let (outer_top,inner_top)=(y.atan2(x),y.atan2(x-offset));
    let mut mesh=egui::Mesh::default();
    let steps = 18;
    for i in 0..=steps {
        let t=i as f32/steps as f32;
        let a=outer_top+(std::f32::consts::TAU-2.0*outer_top)*t;
        let b=inner_top+(std::f32::consts::TAU-2.0*inner_top)*t;
        mesh.colored_vertex(c.pos(cx+a.cos()*outer,cy+a.sin()*outer),SNOW);
        mesh.colored_vertex(c.pos(cx+offset+b.cos()*inner,cy+b.sin()*inner),SNOW);
        if i>0 {let n=i*2;mesh.add_triangle(n-2,n,n+1);mesh.add_triangle(n-2,n+1,n-1);}
    }
    c.painter.add(egui::Shape::mesh(mesh));
}

fn cloud(c: &Card, cx: f32, cy: f32, s: f32, color: Color32) {
    c.circle(cx - s * 0.36, cy + s * 0.04, s * 0.32, color);
    c.circle(cx, cy - s * 0.18, s * 0.42, color);
    c.circle(cx + s * 0.38, cy + s * 0.02, s * 0.31, color);
    c.fill(cx - s * 0.42, cy + s * 0.02, s * 0.84, s * 0.34, s * 0.17, color);
}

fn icon(c: &Card, cx: f32, cy: f32, size: f32, kind: Kind) {
    let s = size / 2.0;
    match kind {
        Kind::Sun => sun(c, cx, cy, s),
        Kind::Moon => moon(c, cx, cy, s * 1.05),
        Kind::Partly => { sun(c, cx + s * 0.30, cy - s * 0.30, s * 0.60); cloud(c, cx - s * 0.10, cy + s * 0.22, s * 0.90, CLOUD); }
        Kind::PartlyNight => {moon(c,cx+s*0.30,cy-s*0.30,s*0.65);cloud(c,cx-s*0.10,cy+s*0.22,s*0.90,CLOUD);}
        Kind::Cloud => cloud(c, cx, cy, s * 0.98, CLOUD),
        Kind::Fog => {
            cloud(c, cx, cy - s * 0.14, s * 0.90, CLOUD);
            for i in 0..3 { c.fill(cx - s * 0.58, cy + s * 0.30 + i as f32 * s * 0.16, s * 1.16, 1.4, 0.7, paint::alpha(CLOUD, 0.72)); }
        }
        Kind::Drizzle => {
            cloud(c, cx, cy - s * 0.16, s * 0.90, CLOUD);
            for i in 0..3 {
                let x = cx - s * 0.32 + i as f32 * s * 0.32;
                c.seg((x, cy + s * 0.32), (x - s * 0.06, cy + s * 0.50), 1.5, RAIN);
            }
        }
        Kind::Rain => {
            cloud(c, cx, cy - s * 0.16, s * 0.90, CLOUD);
            for i in 0..3 {
                let x = cx - s * 0.32 + i as f32 * s * 0.32;
                c.seg((x, cy + s * 0.30), (x - s * 0.12, cy + s * 0.64), 1.8, RAIN);
            }
        }
        Kind::Shower => {
            cloud(c, cx, cy - s * 0.16, s * 0.90, CLOUD);
            for i in 0..2 {
                let x = cx - s * 0.18 + i as f32 * s * 0.38;
                c.seg((x, cy + s * 0.32), (x - s * 0.12, cy + s * 0.60), 1.9, RAIN);
            }
            c.circle(cx + s * 0.34, cy + s * 0.68, s * 0.08, RAIN);
        }
        Kind::Snow => {
            cloud(c, cx, cy - s * 0.16, s * 0.90, CLOUD);
            for i in 0..3 {
                let x = cx - s * 0.30 + i as f32 * s * 0.30;
                c.circle(x, cy + s * 0.42 + (i % 2) as f32 * s * 0.14, s * 0.10, SNOW);
            }
        }
        Kind::Thunder => {
            cloud(c, cx, cy - s * 0.18, s * 0.90, CLOUD);
            c.poly(&[
                (cx - s * 0.04, cy + s * 0.26), (cx + s * 0.18, cy + s * 0.26), (cx + s * 0.02, cy + s * 0.56),
                (cx + s * 0.18, cy + s * 0.56), (cx - s * 0.12, cy + s * 0.88), (cx - s * 0.01, cy + s * 0.54),
                (cx - s * 0.16, cy + s * 0.54),
            ], BOLT);
        }
    }
}

fn weather_now(env:&Env)->i64 {
    use chrono::TimeZone;
    chrono::NaiveDateTime::parse_from_str(&format!("{} {}",env.today,env.clock),"%Y-%m-%d %H:%M:%S").ok()
        .and_then(|time|chrono::Local.from_local_datetime(&time).earliest()).map(|time|time.timestamp())
        .unwrap_or_else(||chrono::Utc::now().timestamp())
}

fn paint_sun_times(c:&Card,snapshot:&weather::Snapshot,now:i64) {
    let times=snapshot.sun_times.as_ref();
    let progress=times.and_then(|times|times.progress(now));
    let today=times.filter(|times|(times.day_start..times.day_end).contains(&now));
    let rise=snapshot.sunrise.as_deref().unwrap_or("--:--");
    let set=snapshot.sunset.as_deref().unwrap_or("--:--");
    c.text(390.0,47.0,&c.fit(&snapshot.place,10.5,W::Regular,180.0),10.5,W::Regular,theme::GRAY,H::Right);
    let mut target=None;
    if let Some(times)=today {
        if now<times.sunset {
            let (title,event,label,index)=if now<times.sunrise {("距离日出",times.sunrise,rise,0)}else{("距离日落",times.sunset,set,2)};
            let minutes=((event-now).max(0)+59)/60;
            c.text(40.0,77.0,title,10.5,W::Regular,theme::GRAY,H::Left);
            c.text(40.0,110.0,&format!("{:02}:{:02}",minutes/60,minutes%60),29.0,W::Bold,theme::WHITE,H::Left);
            c.text(40.0,125.0,"小时 : 分钟",9.5,W::Regular,theme::DIM,H::Left);
            c.text(40.0,147.0,&format!("{} {label}",if index==0{"日出"}else{"日落"}),11.0,W::Semibold,SUN,H::Left);
            target=Some(index);
        }else{
            c.text(40.0,77.0,"今日已日落",10.5,W::Regular,theme::GRAY,H::Left);
            c.text(40.0,110.0,set,29.0,W::Bold,theme::WHITE,H::Left);
            c.text(40.0,130.0,"白昼已经结束",10.0,W::Regular,theme::DIM,H::Left);
        }
    }else{
        c.text(40.0,77.0,"日出时间",10.5,W::Regular,theme::GRAY,H::Left);
        c.text(40.0,110.0,rise,29.0,W::Bold,theme::WHITE,H::Left);
        c.text(40.0,134.0,"日照进度待更新",9.5,W::Regular,WARN,H::Left);
    }
    if let Some(times)=times {
        let minutes=(times.sunset-times.sunrise)/60;
        c.text(40.0,178.0,&format!("白昼 {}小时{:02}分",minutes/60,minutes%60),10.0,W::Regular,theme::GRAY,H::Left);
    }

    // 连续轨迹只映射白昼，保持日出、白昼中点、日落的相对时间位置。
    let point=|pos:f32|(180.0+pos*202.0,120.0-32.0*(1.0-(2.0*pos-1.0).powi(2)));
    let elapsed=today.map(|times|((now-times.sunrise) as f32/(times.sunset-times.sunrise).max(1) as f32).clamp(0.0,1.0));
    c.text(281.0,64.0,if today.is_some(){"今日日照"}else{"日照时段"},10.0,W::Regular,theme::GRAY,H::Center);
    let track:Vec<_>=(0..=48).map(|index|point(index as f32/48.0)).collect();
    let screen:Vec<_>=track.iter().map(|&(x,y)|c.pos(x,y)).collect();
    c.painter.extend(egui::Shape::dashed_line(&screen,egui::Stroke::new(1.2*c.s,Color32::from_white_alpha(65)),3.0*c.s,3.0*c.s));
    if let Some(elapsed)=elapsed.filter(|at|*at>0.0) {
        // 沿用同一条采样轨迹裁出已过时段，保持虚线起点与间隔一致。
        let mut lit:Vec<_>=track.iter().take_while(|&&(x,_)|x<point(elapsed).0).map(|&(x,y)|c.pos(x,y)).collect();
        let (x,y)=point(elapsed);lit.push(c.pos(x,y));
        c.painter.extend(egui::Shape::dashed_line(&lit,egui::Stroke::new(1.4*c.s,paint::alpha(SUN,0.85)),3.0*c.s,3.0*c.s));
    }
    c.seg((180.0,121.0),(382.0,121.0),0.7,Color32::from_white_alpha(18));
    let midpoint=times.map(|times|times.clock(times.midpoint())).unwrap_or_else(||"--:--".into());
    for (index,(pos,label,time)) in [(0.0,"日出",rise),(0.5,"白昼中点",midpoint.as_str()),(1.0,"日落",set)].into_iter().enumerate() {
        let (x,y)=point(pos);
        let next=target==Some(index);
        if index!=1 {c.circle(x,y,2.2,if next{SUN}else{theme::GRAY});}
        c.text(x,144.0,label,9.5,W::Regular,if next{SUN}else{theme::GRAY},H::Center);
        c.text(x,160.0,time,11.0,W::Semibold,theme::WHITE,H::Center);
    }
    if let Some(progress)=progress {
        let (x,y)=point(progress);
        c.glow(x,y,16.0,paint::alpha(SUN,0.18));
        sun(c,x,y,6.0);
    }else if today.is_some() {
        c.glow(281.0,107.0,13.0,paint::alpha(RAIN,0.10));
        moon(c,281.0,107.0,8.0);
    }
}

fn paint_forecast(c:&Card,snapshot:&weather::Snapshot) {
    // 上方实况；整块内容区支持轻点切换。
    icon(c,52.0,65.0,34.0,weather::kind(snapshot.code,snapshot.day));
    c.text(84.0,73.0,&pill_temp_text(snapshot),30.0,W::Bold,theme::WHITE,H::Left);
    c.text(84.0,89.0,weather::describe(snapshot.code),11.5,W::Semibold,theme::WHITE,H::Left);
    c.text(84.0,103.0,&c.fit(&snapshot.place,10.5,W::Regular,118.0),10.5,W::Regular,theme::GRAY,H::Left);
    c.text(84.0,117.0,"轻点切换日照",9.5,W::Regular,theme::DIM,H::Left);
    if let Some(feels)=snapshot.feels {c.text(236.0,54.0,&format!("体感 {feels:.0}°"),11.0,W::Regular,theme::GRAY,H::Left);}
    if let Some(humidity)=snapshot.humidity {c.text(236.0,75.0,&format!("湿度 {humidity}%"),11.0,W::Regular,theme::GRAY,H::Left);}
    if let Some(wind)=snapshot.wind {c.text(236.0,96.0,&format!("风 {wind:.0} km/h"),11.0,W::Regular,theme::GRAY,H::Left);}

    // 下方七日预报，旧缓存中缺少的日期保持空位，刷新后补齐。
    c.seg((24.0,123.0),(395.0,123.0),0.8,Color32::from_white_alpha(16));
    let span=371.0/weather::FORECAST_DAYS as f32;
    for index in 0..weather::FORECAST_DAYS {
        let x=24.0+span*(index as f32+0.5);
        if index==0 {c.fill(x-23.0,126.0,46.0,76.0,7.0,Color32::from_white_alpha(8));}
        let Some(day)=snapshot.days.get(index) else {c.text(x,165.0,"—",12.0,W::Regular,theme::DIM,H::Center);continue;};
        c.text(x,138.0,&day.label,9.5,W::Regular,if index==0{theme::WHITE}else{theme::GRAY},H::Center);
        icon(c,x,153.0,18.0,weather::kind(day.code,true));
        c.text(x,175.0,&format!("{:.0}°",day.high),11.5,W::Semibold,theme::WHITE,H::Center);
        c.text(x,189.0,&format!("{:.0}°",day.low),10.0,W::Regular,theme::GRAY,H::Center);
        if day.rain>=20 {c.text(x,200.0,&format!("{}%",day.rain),8.5,W::Regular,RAIN,H::Center);}
    }
}

impl App {
    /// 后台取数的轮询与自动刷新；数据过期就重新拉一次。
    pub(crate) fn weather_tick(&mut self) {
        let enabled=self.module_on(WEATHER_PAGE);
        self.weather.poll(&self.settings.weather_city,enabled);
        if self.weather.should_refresh(&self.settings.weather_city,self.t,chrono::Utc::now().timestamp(),enabled) {
            // 单元测试仍执行模块开关与刷新判定，仅跳过实际联网。
            if !cfg!(test) {self.weather.refresh(&self.settings.weather_city,self.t);}
        }
    }

    pub(crate) fn apply_weather_city(&mut self) {
        if !self.module_on(WEATHER_PAGE) {return;}
        let city=self.weather.city_input.trim().to_string();
        if self.weather.refresh(&city,self.t) {
            self.settings.weather_city=city.clone();
            self.weather.city_input=city;
            self.weather.solar_open=false;
        }
    }

    /// 常驻胶囊上的天气徽标：有快照、天气页可见且非极简时才占位。
    pub(crate) fn weather_pill(&self) -> Option<&crate::weather::Snapshot> {
        self.weather.snapshot.as_ref()
            .filter(|_| self.module_on(WEATHER_PAGE) && !self.effective_minimal())
    }

    /// 温度文字比基础槽宽时需要补的宽度（负数会被调用方忽略）。
    pub(crate) fn weather_pill_extra(&self, ctx: &egui::Context) -> Option<f32> {
        let snapshot = self.weather_pill()?;
        let text = pill_temp_text(snapshot);
        let text_w = ctx.fonts(|fonts| fonts.layout_no_wrap(text, theme::font(theme::size::PILL_CLOCK, W::Bold), theme::WHITE).size().x).ceil();
        let chip = PILL_ICON + 4.0 + text_w + 2.0;
        Some(chip - crate::bar_layout::WEATHER_WIDTH)
    }

    /// 常驻胶囊的天气徽标：小图标 + 温度，紧跟时钟；点击进入天气页。
    pub(crate) fn draw_pill_weather(&mut self, c: &Card, x: f32, layout: &crate::bar_layout::Layout) {
        let Some(snapshot) = self.weather_pill().cloned() else { return };
        self.paint_pill_weather(c, x, layout, &snapshot);
        let width = layout.widths[crate::bar_layout::WEATHER];
        let tip = format!("{} · {}\n更新于 {} · Open-Meteo", snapshot.place, weather::describe(snapshot.code), weather::clock(snapshot.at));
        // 用 bar_tooltip：常驻胶囊上的提示不能用 on_hover_text，悬浮层会盖住指针、抢走点击。
        if crate::quota_ui::bar_tooltip(c.hit(x - width / 2.0, 1.0, width, 28.0, "weather-pill"), tip).clicked() {
            self.weather.solar_open=false;
            self.show_page(WEATHER_PAGE);
        }
    }

    pub(crate) fn draw_side_weather(&mut self,c:&Card,cx:f32,cy:f32) {
        let Some(snapshot)=self.weather_pill() else{return;};
        icon(c,cx,cy-8.0,PILL_ICON,weather::kind(snapshot.code,snapshot.day));
        let stale=chrono::Utc::now().timestamp()-snapshot.at>weather::INTERVAL*2;
        c.text(cx,cy+14.0,&c.fit(&pill_temp_text(snapshot),10.5,W::Bold,30.0),10.5,W::Bold,if stale{theme::GRAY}else{theme::WHITE},H::Center);
        let tip=format!("{} · {}\n更新于 {} · Open-Meteo",snapshot.place,weather::describe(snapshot.code),weather::clock(snapshot.at));
        let response=crate::quota_ui::bar_tooltip(c.hit(cx-17.0,cy-19.0,34.0,38.0,"weather-side"),tip);
        if response.clicked(){self.weather.solar_open=false;self.show_page(WEATHER_PAGE);}
    }

    /// 只画不响应命中，设置页的常驻预览复用。
    pub(crate) fn paint_pill_weather(&self, c: &Card, x: f32, layout: &crate::bar_layout::Layout, snapshot: &crate::weather::Snapshot) {
        // 吉祥物游到徽标附近时先淡出，避免遮挡。
        let alpha = layout.pet_clearance(crate::bar_layout::WEATHER);
        let faded = c.faded(alpha);
        let faded = if alpha < 0.8 { faded.ghost() } else { faded };
        let c = &faded;
        let width = layout.widths[crate::bar_layout::WEATHER];
        let left = x - width / 2.0;
        icon(c, left + 7.5, 15.0, PILL_ICON, weather::kind(snapshot.code, snapshot.day));
        // 超过半小时没更新就降为灰色，和额度徽标的过期处理一致。
        let stale = chrono::Utc::now().timestamp() - snapshot.at > weather::INTERVAL * 2;
        c.text(left + 16.0, 21.0, &pill_temp_text(snapshot), theme::size::PILL_CLOCK, W::Bold, if stale { theme::GRAY } else { theme::WHITE }, H::Left);
    }

    pub(crate) fn page_weather(&mut self, c: &Card, env: &Env, cmds: &mut Vec<Cmd>) {
        let loading = self.weather.loading;
        let ready=self.weather.can_refresh(&self.settings.weather_city,self.t);
        c.text(326.0, 23.0, if loading { "更新中" } else if ready { "刷新" } else { "稍后刷新" }, 10.5, W::Regular, if ready { theme::GRAY } else { theme::DIM }, H::Center);
        if c.hit(304.0, 8.0, 44.0, 21.0, "weather-refresh").on_hover_text("重新获取当地天气；请求间隔至少 30 秒，失败后至少 2 分钟").clicked() && ready {
            self.weather.refresh(&self.settings.weather_city,self.t);
        }
        let footer=WEATHER_H-4.0;
        c.text(404.0,footer,"Open-Meteo",8.5,W::Regular,theme::DIM,H::Right);
        if c.hit(348.0,footer-9.0,58.0,13.0,"weather-source").on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("天气数据来自 Open-Meteo，按 CC BY 4.0 授权").clicked() {
            cmds.push(Cmd::OpenUrl("https://open-meteo.com/".into()));
        }

        let Some(snapshot) = self.weather.snapshot.clone() else {
            let (title, detail) = if loading {
                ("正在获取当地天气…", "首次定位需要几秒钟".to_string())
            } else if let Some(error) = &self.weather.error {
                ("天气暂时取不到", error.clone())
            } else {
                ("还没有天气数据", "点右上「刷新」重试".to_string())
            };
            c.text(209.5, 104.0, title, 13.0, W::Semibold, theme::WHITE, H::Center);
            c.text(209.5, 124.0, &c.fit(&detail, 10.5, W::Regular, 320.0), 10.5, W::Regular, theme::GRAY, H::Center);
            return;
        };

        let age = chrono::Utc::now().timestamp() - snapshot.at;
        let offline = self.weather.error.is_some();
        let stamp = weather::clock(snapshot.at);
        let stale = offline || age > 3600;
        c.text(344.0, footer, &stamp, 8.5, W::Regular, if stale { WARN } else { theme::DIM }, H::Right);
        c.hit(312.0, footer-9.0, 32.0, 13.0, "weather-stamp").on_hover_text(format!(
            "上次更新 {stamp}{}",
            if offline { "，当前显示的是缓存" } else { "" }
        ));

        let response=c.hit(36.0,34.0,347.0,168.0,"weather-view").on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked(){self.weather.solar_open=!self.weather.solar_open;}
        let mix=c.ui.ctx().animate_bool_with_time(egui::Id::new(("weather-view-blend",c.live)),self.weather.solar_open,0.18);
        if mix<1.0 {
            let forecast=c.faded(1.0-mix).shifted(-6.0*mix,0.0).ghost();
            paint_forecast(&forecast,&snapshot);
        }
        if mix>0.0 {
            let solar=c.faded(mix).shifted(6.0*(1.0-mix),0.0).ghost();
            solar.text(40.0,48.0,"轻点返回天气",10.5,W::Regular,theme::GRAY,H::Left);
            paint_sun_times(&solar,&snapshot,weather_now(env));
        }
    }

    pub(crate) fn weather_settings(&mut self, ui: &mut egui::Ui) {
        use crate::extras::{settings_card, settings_control_row, settings_hint, settings_label};
        settings_label(ui, "位置");
        settings_card(ui, |ui| {
            let response=settings_control_row(ui, "城市", 56.0, |ui| {
                let width = (ui.available_width() - 8.0).min(300.0);
                ui.add_sized([width, 30.0], egui::TextEdit::singleline(&mut self.weather.city_input)
                    .vertical_align(egui::Align::Center).hint_text("留空则按网络出口自动定位，例如：杭州"))
            });
            if response.lost_focus() && ui.input(|input|input.key_pressed(egui::Key::Enter)) {self.apply_weather_city();}
            settings_hint(ui, "输入中文或英文城市名，按回车或点「刷新天气」应用。自动定位使用网络出口的大致位置，可能受代理影响。");
        });

        settings_label(ui, "当前");
        settings_card(ui, |ui| {
            let place = self.weather.snapshot.as_ref().map(|s| (s.place.clone(), s.temp, s.at));
            let text = match &place {
                Some((place, temp, _)) => format!("{place} · {temp:.0}°"),
                None if self.weather.loading => "正在定位…".to_string(),
                None => "尚未获取".to_string(),
            };
            ui.label(egui::RichText::new(text).size(13.0));
            if let Some((_, _, at)) = place {
                ui.label(egui::RichText::new(format!("更新于 {}", weather::clock(at))).size(11.5).color(ui.visuals().weak_text_color()));
            }
            if let Some(error) = &self.weather.error {
                ui.add(egui::Label::new(egui::RichText::new(error).size(11.5).color(WARN)).wrap());
            }
            ui.horizontal(|ui| {
                let ready=self.weather.can_refresh(&self.weather.city_input,self.t);
                if ui.add_enabled(ready, egui::Button::new(if self.weather.loading { "正在获取…" } else { "刷新天气" })).clicked() {
                    self.apply_weather_city();
                }
                if ui.add_enabled(self.weather.can_refresh("",self.t), egui::Button::new("自动定位")).on_hover_text("清空城市，按当前网络出口重新定位").clicked() {
                    self.weather.city_input.clear();
                    self.apply_weather_city();
                }
            });
            settings_hint(ui, "同一城市请求间隔至少 30 秒，失败后至少 2 分钟。只查询位置与天气，不涉及账号信息。");
            ui.horizontal(|ui|{
                ui.hyperlink_to("天气数据：Open-Meteo","https://open-meteo.com/");
                ui.hyperlink_to("CC BY 4.0","https://creativecommons.org/licenses/by/4.0/");
            });
        });
    }
}
