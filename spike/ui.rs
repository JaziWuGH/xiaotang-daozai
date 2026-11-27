use super::*;
use crate::{
    paint::{Card, H},
    theme::{self, W},
};
use eframe::egui;

struct View {
    session: Session,
    title: String,
    date: String,
    error: String,
    tick: Instant,
    kind: String,
    calls: u64,
    work: Duration,
}
impl View {
    fn load(kind: &str) -> Result<Self> {
        let mut session = Session::new(kind, data().join(kind).join("ui"))?;
        session.refresh()?;
        Ok(Self {
            title: session.settings.title.clone(),
            date: session.settings.target.clone(),
            session,
            error: String::new(),
            tick: Instant::now(),
            kind: kind.into(),
            calls: 1,
            work: Duration::ZERO,
        })
    }
    fn draw(&mut self, c: &Card) {
        if self.tick.elapsed() >= Duration::from_secs(1) {
            let started = Instant::now();
            if let Err(e) = self.session.refresh() {
                self.error = e.to_string()
            }
            self.calls += 1;
            self.work += started.elapsed();
            self.tick = Instant::now()
        }
        c.ui.ctx().request_repaint_after(Duration::from_secs(1));
        let s = &self.session;
        c.fill(14., 36., 390., 76., 12., theme::CARD);
        c.text(
            28.,
            57.,
            &format!("倒计时 · {}", self.kind),
            13.,
            W::Semibold,
            theme::CYAN,
            H::Left,
        );
        c.text(
            28.,
            83.,
            &format!("距离{}还有 {} 天", s.settings.title, s.days),
            19.,
            W::Bold,
            theme::WHITE,
            H::Left,
        );
        let last = s
            .previous
            .map(|d| format!("{d} 天"))
            .unwrap_or_else(|| "首次显示".into());
        c.text(
            28.,
            103.,
            &format!("目标 {}　上次显示 {last}", s.settings.target),
            12.,
            W::Regular,
            theme::GRAY,
            H::Left,
        );
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        crate::extras::settings_card(ui, |ui| {
            crate::extras::settings_control_row(ui, "标题", 64., |ui| {
                ui.text_edit_singleline(&mut self.title)
            });
            crate::extras::settings_control_row(ui, "目标日期", 64., |ui| {
                ui.text_edit_singleline(&mut self.date)
            });
            if ui.button("保存设置").clicked() {
                self.error = self
                    .session
                    .save(self.title.clone(), self.date.clone())
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_default()
            }
            if !self.error.is_empty() {
                ui.label(&self.error);
            }
        });
    }
}
thread_local! {
    static LIVE:Option<std::cell::RefCell<View>>={
        let mut args=env::args().skip_while(|a|a!="--sdk-live");args.next();
        args.next().and_then(|kind|View::load(&kind).map_err(|e|eprintln!("{e}")).ok()).map(std::cell::RefCell::new)
    };
}
pub fn live_card(c: &Card) -> bool {
    LIVE.with(|v| {
        v.as_ref().is_some_and(|s| {
            s.borrow_mut().draw(c);
            true
        })
    })
}
pub fn live_settings(ui: &mut egui::Ui) {
    LIVE.with(|v| {
        if let Some(s) = v {
            s.borrow_mut().settings(ui)
        }
    })
}

struct Preview {
    view: View,
    path: Option<PathBuf>,
    frames: u8,
    profile: Option<PathBuf>,
    drawn: u64,
    last_report: Instant,
    repaint_interval: Option<Duration>,
}
impl eframe::App for Preview {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("岛仔 · 待办与置物");
            let (rect, _) = ui.allocate_exact_size(egui::vec2(419., 125.), egui::Sense::hover());
            ui.painter().rect_filled(rect, 15., egui::Color32::BLACK);
            let card = Card::new(ui, rect.min, rect);
            self.view.draw(&card);
            ui.label("倒计时设置");
            self.view.settings(ui);
        });
        self.drawn += 1;
        if let Some(interval) = self.repaint_interval {
            ctx.request_repaint_after(interval);
        }
        if let Some(path) = &self.profile {
            if self.last_report.elapsed() >= Duration::from_secs(1) {
                let _ = fs::write(
                    path,
                    format!(
                        "{} {} {}",
                        self.drawn,
                        self.view.calls,
                        self.view.work.as_micros()
                    ),
                );
                self.last_report = Instant::now();
            }
        }
        if let Some(path) = &self.path {
            self.frames = self.frames.saturating_add(1);
            if self.frames == 4 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()))
            }
            let screenshot = ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = screenshot {
                let rgba: Vec<_> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                image::save_buffer(
                    path,
                    &rgba,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .expect("保存验证截图");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }
}
pub(super) fn window(kind: &str, path: Option<PathBuf>) -> Result<()> {
    let view = View::load(kind)?;
    let opts = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        // 与正常岛体一致；测量时可显式恢复旧验证窗口的垂直同步。
        vsync: !cfg!(windows) || env::var_os("DAOZAI_SPIKE_LEGACY_VSYNC").is_some(),
        viewport: egui::ViewportBuilder::default().with_inner_size([445., 318.]),
        ..Default::default()
    };
    eframe::run_native(
        "岛仔 SDK 验证",
        opts,
        Box::new(move |cc| {
            theme::load_fonts(&cc.egui_ctx);
            Ok(Box::new(Preview {
                view,
                path,
                frames: 0,
                profile: env::var_os("DAOZAI_SPIKE_COUNTERS").map(PathBuf::from),
                drawn: 0,
                repaint_interval: env::var("DAOZAI_SPIKE_REDRAW_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|v| *v > 0)
                    .map(Duration::from_millis),
                last_report: Instant::now(),
            }))
        }),
    )
    .map_err(|e| fail(&e.to_string()))
}
