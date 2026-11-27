//! 主岛材质的装载、底片与绘制节拍。

mod backdrop;
mod controls;
mod pipeline;
pub use backdrop::{Backdrop, CleanPlate, Geometry, ScreenRect};
pub(crate) use controls::{control, control_scale, ControlRole};
#[cfg(feature = "sdk-js")]
pub(crate) use pipeline::validate_plugin_shader;

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Pos2, Rect};

use crate::{app::FrameOut, model::Dock};
use controls::{Controls, Registration};
use pipeline::{Frame, Pipeline};

pub const FALLBACK_ARG: &str = "--material-fallback";

pub fn falling_back() -> bool {
    std::env::args_os().any(|arg| arg == FALLBACK_ARG)
}

pub fn debug_requested() -> bool {
    std::env::var_os("DAOZAI_MATERIAL_DEBUG").is_some_and(|value| value == "1") && !falling_back()
}

pub fn configure_startup(
    options: &mut eframe::NativeOptions,
    compatible_rendering: bool,
    material_active: bool,
) {
    options.wgpu_options.direct_composition =
        compatible_rendering && material_active && !falling_back();
}

pub fn take_startup_failure() -> Option<String> {
    let error = eframe::egui_wgpu::winit::take_direct_composition_error();
    if let Some(error) = &error {
        log(&format!("透明交换链初始化失败，退回普通绘制：{error}"));
    }
    error
}

pub fn report_run_failure(result: &eframe::Result<()>) -> bool {
    let _ = take_startup_failure();
    if let Err(eframe::Error::Wgpu(eframe::egui_wgpu::WgpuError::DirectComposition(error))) = result
    {
        log(&format!("透明交换链运行失败，退回普通绘制：{error}"));
        true
    } else {
        false
    }
}

fn log(message: &str) {
    crate::resources::append_log(
        &crate::model::config_dir().join("diagnostics"),
        "material",
        message,
    );
}

#[derive(Clone)]
pub struct Options {
    pub controls: bool,
    pub stiffness: f32,
    pub damping: f32,
    pub params: [f32; 32],
    pub shadow: Option<f32>,
    pub glass_foreground: bool,
}

#[cfg(feature = "sdk-js")]
impl From<&crate::sdk_spike::local::MaterialSpec<'_>> for Options {
    fn from(spec: &crate::sdk_spike::local::MaterialSpec<'_>) -> Self {
        Self {
            controls: spec.controls,
            stiffness: spec.stiffness,
            damping: spec.damping,
            params: spec.params,
            shadow: spec.shadow,
            glass_foreground: spec.glass_foreground,
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            controls: true,
            stiffness: 1.0,
            damping: 0.5,
            params: [0.0; 32],
            shadow: None,
            glass_foreground: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Appearance {
    pub shadow: Option<f32>,
    pub glass_foreground: bool,
    pub light: bool,
    pub controls: bool,
    pub ready: bool,
}

pub(crate) struct Layer {
    painter: egui::Painter,
    slot: egui::layers::ShapeIdx,
    body: Rect,
    capture: Rect,
    radius: f32,
    expansion: f32,
    notch: Option<(f32, f32)>,
    dock: Dock,
    artwork: Option<(egui::TextureId, Arc<egui::ColorImage>)>,
}

impl Layer {
    pub(crate) fn reserve(
        painter: &egui::Painter,
        canvas: Rect,
        body: Rect,
        radius: f32,
        expansion: f32,
        notch: Option<(f32, f32)>,
        dock: Dock,
        artwork: Option<&(Arc<egui::ColorImage>, egui::TextureHandle)>,
        fallback: egui::Shape,
    ) -> Self {
        let padding = if dock.is_side() {
            egui::vec2(48.0, 64.0)
        } else {
            egui::vec2(64.0, 48.0)
        };
        Self {
            painter: painter.clone(),
            slot: painter.add(fallback),
            body,
            capture: body.expand2(padding).intersect(canvas),
            radius,
            expansion,
            notch,
            dock,
            artwork: artwork.map(|(image, texture)| (texture.id(), Arc::clone(image))),
        }
    }
}

#[derive(Default)]
struct Cadence {
    last_activity: Option<Instant>,
    last_sample: Option<Instant>,
}

impl Cadence {
    fn interval(&mut self, now: Instant, active: bool) -> Duration {
        let last = self.last_activity.get_or_insert(now);
        if active {
            *last = now;
            Duration::from_secs_f64(1.0 / 60.0)
        } else if now.duration_since(*last) >= Duration::from_secs(120) {
            Duration::from_secs(1)
        } else {
            Duration::from_secs_f64(1.0 / 15.0)
        }
    }

    fn remaining(&self, now: Instant, interval: Duration) -> Duration {
        self.last_sample.map_or(Duration::ZERO, |at| {
            interval.saturating_sub(now.duration_since(at))
        })
    }
}

#[derive(Default)]
struct Budget(u32);

impl Budget {
    fn exceeded(&mut self, elapsed: Duration) -> bool {
        if elapsed > Duration::from_millis(4) {
            self.0 += 1;
        } else {
            self.0 = 0;
        }
        self.0 >= 120
    }
}

struct Motion {
    expansion: Option<f32>,
    light: f32,
    morph: f32,
}

impl Default for Motion {
    fn default() -> Self {
        Self {
            expansion: None,
            light: 1.0,
            morph: 0.0,
        }
    }
}

impl Motion {
    fn sample_expansion(&mut self, expansion: f32, dt: f32) -> (f32, f32) {
        let speed = self
            .expansion
            .replace(expansion)
            .map_or(0.0, |previous| (expansion - previous) / dt.max(0.001));
        // 沿用 #235：取本帧速度贡献和上一帧衰减值中的较大者。
        self.morph = (self.morph * 0.74).max((speed.abs() * 0.30).min(1.0));
        (speed, self.morph)
    }

    fn update_light(&mut self, brightness: f32, dt: f32) -> bool {
        let target = if brightness > 0.66 {
            1.0
        } else if brightness < 0.52 {
            0.0
        } else {
            self.light
        };
        self.light += (target - self.light) * (dt.clamp(0.0, 0.1) * 6.0);
        (target - self.light).abs() > 0.001
    }
}

pub struct Runtime {
    pipeline: Pipeline,
    options: Options,
    backdrop: Option<Backdrop>,
    controls: Controls,
    registrations: Vec<Registration>,
    cadence: Cadence,
    budget: Budget,
    last_frame: Option<Instant>,
    pointer: Option<Pos2>,
    screen: Option<ScreenRect>,
    tint: Option<(egui::TextureId, [f32; 3])>,
    motion: Motion,
    ready: bool,
    capture_ready: bool,
    has_drawn: bool,
}

impl Runtime {
    /// 用宿主已选定的材质创建管线。调用方负责先建立主岛透明交换链。
    pub fn load(
        render_state: &eframe::egui_wgpu::RenderState,
        shader: &str,
        options: Options,
    ) -> Result<Self, String> {
        if render_state.adapter.get_info().backend != eframe::wgpu::Backend::Dx12 {
            return Err("材质需要 Direct3D 12 兼容渲染".into());
        }
        Ok(Self {
            pipeline: Pipeline::new(render_state, shader)?,
            options,
            backdrop: None,
            controls: Controls::default(),
            registrations: Vec::new(),
            cadence: Cadence::default(),
            budget: Budget::default(),
            last_frame: None,
            pointer: None,
            screen: None,
            tint: None,
            motion: Motion::default(),
            ready: false,
            capture_ready: false,
            has_drawn: false,
        })
    }

    pub fn debug(render_state: &eframe::egui_wgpu::RenderState) -> Result<Self, String> {
        let material = Self::load(render_state, pipeline::DEBUG_SHADER, Options::default())?;
        log("调试材质已加载");
        Ok(material)
    }

    pub fn set_options(&mut self, options: Options) {
        self.options = options;
    }

    pub fn capture_ready(&self) -> bool {
        self.capture_ready
    }

    pub fn appearance(&self) -> Appearance {
        Appearance {
            shadow: self.options.shadow,
            glass_foreground: self.options.glass_foreground,
            light: self.motion.light > 0.5,
            controls: self.options.controls,
            ready: self.ready,
        }
    }

    fn clear_backdrop(&mut self) {
        self.backdrop = None;
        self.pipeline.clear_backdrop();
        self.screen = None;
        self.ready = false;
    }

    pub fn draw(&mut self, draw_island: impl FnOnce() -> FrameOut) -> FrameOut {
        if self.options.controls {
            let (out, registrations) = controls::collect(draw_island);
            self.registrations = registrations;
            out
        } else {
            draw_island()
        }
    }

    /// 隐藏或锁屏期间释放桌面捕获，并在恢复后重新定位底片。
    pub fn suspend(&mut self) {
        if self.backdrop.is_some() {
            self.clear_backdrop();
            self.controls = Controls::default();
            self.registrations.clear();
            self.cadence = Cadence::default();
            self.budget = Budget::default();
            self.last_frame = None;
            self.motion.expansion = None;
            self.motion.morph = 0.0;
            self.pointer = None;
        }
    }

    pub fn finish(
        &mut self,
        ctx: &egui::Context,
        window: &mut crate::native::Window,
        out: &mut FrameOut,
        time: f64,
        screen_pointer: Option<(i32, i32)>,
    ) -> Result<Option<Duration>, String> {
        let Some(layer) = out.material_layer.take() else {
            return Ok(None);
        };
        if ctx.will_discard() {
            return Ok(None);
        }
        let hwnd = window.material_window().ok_or("透明交换链不可用")?;
        if !unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd) }.as_bool() {
            return Ok(Some(Duration::from_millis(20)));
        }
        let started = Instant::now();
        if window.ensure_material_capture_exclusion()? {
            // 原生样式恢复后，旧的抓屏会话可能含有岛体画面。
            self.clear_backdrop();
        }
        self.capture_ready = true;
        let ppp = ctx.pixels_per_point();
        let Some((geometry, viewport)) =
            Geometry::for_window(hwnd, layer.body, layer.capture, ppp)?
        else {
            self.ready = false;
            return Ok(Some(self.cadence.interval(started, out.animating)));
        };
        let [bx, by, bw, bh] = geometry.body;
        let body_pixels = Rect::from_min_size(egui::pos2(bx, by), egui::vec2(bw, bh));
        let pointer = screen_pointer.map(|(x, y)| {
            egui::pos2(
                (x - geometry.screen.x) as f32,
                (y - geometry.screen.y) as f32,
            )
        });
        let pointer_in_island = pointer.filter(|&p| body_pixels.contains(p));
        let moved = self.pointer != pointer_in_island;
        self.pointer = pointer_in_island;
        let dt = self
            .last_frame
            .replace(started)
            .map_or(1.0 / 60.0, |at| (started - at).as_secs_f32());
        let (speed, morph) = self.motion.sample_expansion(layer.expansion, dt);
        let moving = out.animating
            || morph > 0.01
            || moved
            || pointer_in_island.is_some()
                && ctx.input(|i| i.pointer.any_down() || i.pointer.any_released());
        let capture_origin = (viewport.min.to_vec2() * ppp).round().to_pos2();
        let mut registrations = std::mem::take(&mut self.registrations);
        if self.options.glass_foreground && layer.expansion < 0.985 {
            registrations.clear();
        }
        let controls = self.controls.frame(
            ctx,
            registrations,
            ppp,
            capture_origin,
            dt,
            self.options.stiffness,
            self.options.damping,
        );
        let interval = self.cadence.interval(started, moving || controls.animating);
        let backdrop = self.backdrop.get_or_insert_with(Backdrop::default);
        let plate = if self.screen != Some(geometry.screen)
            || self.cadence.remaining(started, interval).is_zero()
        {
            self.cadence.last_sample = Some(started);
            self.screen = Some(geometry.screen);
            backdrop.capture(geometry).map_err(str::to_owned)?
        } else {
            backdrop.current(geometry)
        };

        let mut light_changing = false;
        let shape = if let Some(plate) = plate {
            let mut frame = Frame::default();
            let [x, y, w, h] = [
                geometry.screen.x,
                geometry.screen.y,
                geometry.screen.width,
                geometry.screen.height,
            ];
            frame.capture = [x as f32, y as f32, w as f32, h as f32];
            frame.body = geometry.body;
            let pointer = pointer.unwrap_or(egui::pos2(-1.0, -1.0));
            frame.shape = [layer.radius * ppp, layer.expansion, pointer.x, pointer.y];
            let (flare, height) = layer.notch.unwrap_or_default();
            frame.notch = [
                flare * ppp,
                height * ppp,
                u8::from(layer.notch.is_some()) as f32,
                0.0,
            ];
            frame.env = [time as f32, plate.brightness, ppp, 1.0];
            // 前景与着色器使用同一帧的明暗值，下一帧再应用平滑后的值。
            frame.motion = [speed, self.motion.light, morph, 0.0];
            light_changing = self.motion.update_light(plate.brightness, dt);
            if let Some((id, image)) = layer.artwork {
                if !self.tint.is_some_and(|(known, _)| known == id) {
                    self.tint = Some((id, crate::fluid::Fluid::tint_from_image(&image)));
                }
                if let Some((_, [red, green, blue])) = self.tint {
                    frame.tint = [red, green, blue, 1.0];
                }
            }
            frame.count[0] = controls.count;
            frame.ctl_rect = controls.rects;
            frame.ctl_shape = controls.shapes;
            frame.ctl_spring = controls.springs;
            for (dest, source) in frame.params.iter_mut().flatten().zip(self.options.params) {
                *dest = source;
            }
            Some(self.pipeline.callback(viewport, frame, plate, layer.dock)?)
        } else {
            self.pipeline.clear_backdrop();
            None
        };

        if self.budget.exceeded(started.elapsed()) {
            return Err("材质通道连续 120 帧超过 4 ms".into());
        }
        self.ready = shape.is_some();
        if let Some(shape) = shape {
            layer.painter.set(layer.slot, shape);
            if !self.has_drawn {
                self.has_drawn = true;
                log(&format!(
                    "材质首帧就绪（{} × {}）",
                    geometry.screen.width, geometry.screen.height
                ));
            }
        }
        let delay = self.cadence.remaining(Instant::now(), interval);
        Ok(Some(if light_changing {
            delay.min(Duration::from_secs_f64(1.0 / 60.0))
        } else {
            delay
        }))
    }

    pub fn report_failure(error: &str) {
        log(&format!("材质已停用，退回普通绘制：{error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_swapchain_requires_compatible_rendering_and_an_active_material() {
        let mut options = eframe::NativeOptions::default();
        assert!(!options.wgpu_options.direct_composition);
        configure_startup(&mut options, true, true);
        assert!(options.wgpu_options.direct_composition);
        configure_startup(&mut options, true, false);
        assert!(!options.wgpu_options.direct_composition);
        configure_startup(&mut options, false, true);
        assert!(!options.wgpu_options.direct_composition);
    }

    #[test]
    fn material_refresh_slows_after_two_minutes_and_wakes_on_activity() {
        let start = Instant::now();
        let mut cadence = Cadence::default();
        let idle = Duration::from_secs_f64(1.0 / 15.0);
        assert_eq!(cadence.interval(start, false), idle);
        assert_eq!(
            cadence.interval(start + Duration::from_secs(119), false),
            idle
        );
        let wake = start + Duration::from_secs(120);
        assert_eq!(cadence.interval(wake, false), Duration::from_secs(1));
        assert_eq!(
            cadence.interval(wake, true),
            Duration::from_secs_f64(1.0 / 60.0)
        );
        assert_eq!(cadence.interval(wake + Duration::from_secs(1), false), idle);
    }

    #[test]
    fn material_budget_only_trips_after_120_consecutive_slow_frames() {
        let slow = Duration::from_millis(5);
        let mut budget = Budget::default();
        for _ in 0..119 {
            assert!(!budget.exceeded(slow));
        }
        assert!(!budget.exceeded(Duration::from_millis(4)));
        for _ in 0..119 {
            assert!(!budget.exceeded(slow));
        }
        assert!(budget.exceeded(slow));
    }

    #[test]
    fn material_light_holds_within_the_hysteresis_band() {
        let mut motion = Motion::default();
        let dt = 1.0 / 60.0;
        assert!(motion.update_light(0.51, dt));
        assert!((motion.light - 0.9).abs() < 1e-5);
        let light = motion.light;
        for brightness in [0.52, 0.53, 0.65, 0.66, 0.53] {
            assert!(!motion.update_light(brightness, dt));
            assert_eq!(motion.light, light);
        }
        assert!(motion.update_light(0.67, dt));
        assert!(motion.light > light);
    }

    #[test]
    fn material_light_still_transitions_after_an_idle_frame() {
        let mut motion = Motion::default();
        assert!(motion.update_light(0.40, 1.0));
        assert!((motion.light - 0.4).abs() < 1e-5);
    }

    #[test]
    fn material_expansion_keeps_signed_speed_and_decays_morph_like_the_preview() {
        let mut motion = Motion::default();
        assert_eq!(motion.sample_expansion(0.0, 0.125), (0.0, 0.0));
        assert_eq!(motion.sample_expansion(0.25, 0.125), (2.0, 0.6));
        assert_eq!(motion.sample_expansion(0.75, 0.125), (4.0, 1.0));
        assert_eq!(motion.sample_expansion(0.5, 0.125), (-2.0, 0.74));
        assert_eq!(motion.sample_expansion(0.5, 0.125), (0.0, 0.74 * 0.74));
        for _ in 0..14 {
            assert_eq!(motion.sample_expansion(0.5, 0.125).0, 0.0);
        }
        assert!(motion.morph < 0.01);
    }
}
