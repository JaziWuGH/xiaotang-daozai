#![allow(clippy::missing_errors_doc)]
#![allow(clippy::undocumented_unsafe_blocks)]

use crate::capture::{capture_channel, CaptureReceiver, CaptureSender, CaptureState};
use crate::{renderer, RenderState, SurfaceErrorAction, WgpuConfiguration};
use egui::{Context, Event, UserData, ViewportId, ViewportIdMap, ViewportIdSet};
use std::{num::NonZeroU32, sync::Arc};

struct SurfaceState {
    surface: wgpu::Surface<'static>,
    alpha_mode: wgpu::CompositeAlphaMode,
    width: u32,
    height: u32,
    #[cfg(all(windows, feature = "direct-composition"))]
    dcomp: Option<DirectCompositionTarget>,
}

#[cfg(all(windows, feature = "direct-composition"))]
use std::sync::atomic::{AtomicIsize, Ordering};
#[cfg(all(windows, feature = "direct-composition"))]
use windows::Win32::Graphics::DirectComposition::IDCompositionDevice;

#[cfg(all(windows, feature = "direct-composition"))]
static DCOMP_HWND: AtomicIsize = AtomicIsize::new(0);
#[cfg(all(windows, feature = "direct-composition"))]
static DCOMP_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 查询此窗口是否已经启用 DirectComposition 透明交换链。
#[cfg(all(windows, feature = "direct-composition"))]
pub fn direct_composition_active(hwnd: isize) -> bool {
    hwnd != 0 && DCOMP_HWND.load(Ordering::Relaxed) == hwnd
}

/// 判断根窗口能否使用空闲等待的重绘节奏。
#[cfg(all(windows, feature = "direct-composition"))]
pub fn has_direct_composition_surface() -> bool {
    DCOMP_HWND.load(Ordering::Relaxed) != 0
}

/// 取出最近一次 DirectComposition 初始化或配置错误。
#[cfg(all(windows, feature = "direct-composition"))]
pub fn take_direct_composition_error() -> Option<String> {
    DCOMP_ERROR.lock().unwrap().take()
}

/// 保存错误，供宿主写入材质诊断日志。
#[cfg(all(windows, feature = "direct-composition"))]
pub fn record_direct_composition_error(error: String) {
    *DCOMP_ERROR.lock().unwrap() = Some(error);
}

#[cfg(all(windows, feature = "direct-composition"))]
struct DirectCompositionTarget {
    hwnd: isize,
    device: IDCompositionDevice,
    _target: windows::Win32::Graphics::DirectComposition::IDCompositionTarget,
    _window: Arc<winit::window::Window>,
}

#[cfg(all(windows, feature = "direct-composition"))]
impl Drop for DirectCompositionTarget {
    fn drop(&mut self) {
        let _ = DCOMP_HWND.compare_exchange(self.hwnd, 0, Ordering::Relaxed, Ordering::Relaxed);
    }
}

#[cfg(all(windows, feature = "direct-composition"))]
fn create_direct_composition_surface(
    window: Arc<winit::window::Window>,
    instance: &wgpu::Instance,
    adapter: &wgpu::Adapter,
) -> Result<(wgpu::Surface<'static>, DirectCompositionTarget), crate::WgpuError> {
    use crate::WgpuError::DirectComposition;
    use windows::core::Interface as _;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::DirectComposition::DCompositionCreateDevice2;
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};

    let RawWindowHandle::Win32(win32) = window.window_handle()?.as_raw() else {
        return Err(DirectComposition("无法取得 Win32 窗口句柄".into()));
    };
    let hwnd = win32.hwnd.get();

    // 用同一 DXGI 适配器创建 D3D11 设备，兼顾无法从 D3D12 查询 IDXGIDevice 的驱动。
    let dxgi_adapter = unsafe {
        adapter.as_hal::<wgpu::hal::api::Dx12, _, _>(|hal| {
            hal.map(|adapter| adapter.raw_dxgi_adapter().clone())
        })
    }
    .ok_or_else(|| DirectComposition("当前适配器未使用 Direct3D 12".into()))?;
    let mut d3d11: Option<ID3D11Device> = None;
    unsafe {
        D3D11CreateDevice(
            &dxgi_adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            None,
            Default::default(),
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d11),
            None,
            None,
        )
    }
    .map_err(|error| DirectComposition(format!("D3D11CreateDevice：{error}")))?;
    let d3d11 = d3d11.ok_or_else(|| DirectComposition("D3D11CreateDevice 未返回设备".into()))?;
    let dxgi_device: IDXGIDevice = d3d11
        .cast()
        .map_err(|error| DirectComposition(format!("IDXGIDevice：{error}")))?;
    let device: IDCompositionDevice = unsafe { DCompositionCreateDevice2(&dxgi_device) }
        .map_err(|error| DirectComposition(format!("DCompositionCreateDevice2：{error}")))?;
    let visual = unsafe { device.CreateVisual() }
        .map_err(|error| DirectComposition(format!("CreateVisual：{error}")))?;
    let target = unsafe { device.CreateTargetForHwnd(HWND(hwnd as *mut _), true) }
        .map_err(|error| DirectComposition(format!("CreateTargetForHwnd：{error}")))?;
    unsafe { target.SetRoot(&visual) }
        .map_err(|error| DirectComposition(format!("SetRoot：{error}")))?;

    // wgpu-hal 持有 visual 的 COM 引用。SurfaceState 先释放 Surface，再释放 target 和窗口。
    let surface = unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CompositionVisual(
            visual.as_raw(),
        ))?
    };
    Ok((
        surface,
        DirectCompositionTarget {
            hwnd,
            device,
            _target: target,
            _window: window,
        },
    ))
}

#[cfg(all(windows, feature = "direct-composition"))]
fn configure_direct_composition(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    target: &DirectCompositionTarget,
) -> Result<(), crate::WgpuError> {
    use crate::WgpuError::DirectComposition;
    use wgpu::ErrorFilter;

    for filter in [
        ErrorFilter::Validation,
        ErrorFilter::Internal,
        ErrorFilter::OutOfMemory,
    ] {
        device.push_error_scope(filter);
    }
    surface.configure(device, config);
    let mut error = None;
    for _ in 0..3 {
        error = error.or(pollster::block_on(device.pop_error_scope()));
    }
    if let Some(error) = error {
        return Err(DirectComposition(error.to_string()));
    }
    // wgpu-hal 已给 visual 设置交换链内容，此处提交 DComp 合成树。
    unsafe { target.device.Commit() }
        .map_err(|error| DirectComposition(format!("Commit：{error}")))?;
    DCOMP_HWND.store(target.hwnd, Ordering::Relaxed);
    Ok(())
}

/// Everything you need to paint egui with [`wgpu`] on [`winit`].
///
/// Alternatively you can use [`crate::Renderer`] directly.
///
/// NOTE: all egui viewports share the same painter.
pub struct Painter {
    context: Context,
    configuration: WgpuConfiguration,
    msaa_samples: u32,
    support_transparent_backbuffer: bool,
    dithering: bool,
    depth_format: Option<wgpu::TextureFormat>,
    screen_capture_state: Option<CaptureState>,

    instance: wgpu::Instance,
    render_state: Option<RenderState>,
    #[cfg(all(windows, feature = "direct-composition"))]
    direct_composition_error: Option<crate::WgpuError>,

    // Per viewport/window:
    depth_texture_view: ViewportIdMap<wgpu::TextureView>,
    msaa_texture_view: ViewportIdMap<wgpu::TextureView>,
    surfaces: ViewportIdMap<SurfaceState>,
    capture_tx: CaptureSender,
    capture_rx: CaptureReceiver,
}

impl Painter {
    /// Manages [`wgpu`] state, including surface state, required to render egui.
    ///
    /// Only the [`wgpu::Instance`] is initialized here. Device selection and the initialization
    /// of render + surface state is deferred until the painter is given its first window target
    /// via [`set_window()`](Self::set_window). (Ensuring that a device that's compatible with the
    /// native window is chosen)
    ///
    /// Before calling [`paint_and_update_textures()`](Self::paint_and_update_textures) a
    /// [`wgpu::Surface`] must be initialized (and corresponding render state) by calling
    /// [`set_window()`](Self::set_window) once you have
    /// a [`winit::window::Window`] with a valid `.raw_window_handle()`
    /// associated.
    pub async fn new(
        context: Context,
        configuration: WgpuConfiguration,
        msaa_samples: u32,
        depth_format: Option<wgpu::TextureFormat>,
        support_transparent_backbuffer: bool,
        dithering: bool,
    ) -> Self {
        let (capture_tx, capture_rx) = capture_channel();
        let instance = configuration.wgpu_setup.new_instance().await;

        Self {
            context,
            configuration,
            msaa_samples,
            support_transparent_backbuffer,
            dithering,
            depth_format,
            screen_capture_state: None,

            instance,
            render_state: None,
            #[cfg(all(windows, feature = "direct-composition"))]
            direct_composition_error: None,

            depth_texture_view: Default::default(),
            surfaces: Default::default(),
            msaa_texture_view: Default::default(),

            capture_tx,
            capture_rx,
        }
    }

    /// 取出根窗口在运行期发生的 DirectComposition 错误。
    #[cfg(all(windows, feature = "direct-composition"))]
    pub fn take_direct_composition_error(&mut self) -> Option<crate::WgpuError> {
        self.direct_composition_error.take()
    }

    /// Get the [`RenderState`].
    ///
    /// Will return [`None`] if the render state has not been initialized yet.
    pub fn render_state(&self) -> Option<RenderState> {
        self.render_state.clone()
    }

    fn configure_surface(
        surface_state: &SurfaceState,
        render_state: &RenderState,
        config: &WgpuConfiguration,
    ) -> Result<(), crate::WgpuError> {
        profiling::function_scope!();

        let width = surface_state.width;
        let height = surface_state.height;

        let mut surf_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: render_state.target_format,
            present_mode: config.present_mode,
            alpha_mode: surface_state.alpha_mode,
            view_formats: vec![render_state.target_format],
            ..surface_state
                .surface
                .get_default_config(&render_state.adapter, width, height)
                .expect("The surface isn't supported by this adapter")
        };

        if let Some(desired_maximum_frame_latency) = config.desired_maximum_frame_latency {
            surf_config.desired_maximum_frame_latency = desired_maximum_frame_latency;
        }

        #[cfg(all(windows, feature = "direct-composition"))]
        if let Some(target) = &surface_state.dcomp {
            return configure_direct_composition(
                &surface_state.surface,
                &render_state.device,
                &surf_config,
                target,
            );
        }

        surface_state
            .surface
            .configure(&render_state.device, &surf_config);
        Ok(())
    }

    /// Updates (or clears) the [`winit::window::Window`] associated with the [`Painter`]
    ///
    /// This creates a [`wgpu::Surface`] for the given Window (as well as initializing render
    /// state if needed) that is used for egui rendering.
    ///
    /// This must be called before trying to render via
    /// [`paint_and_update_textures`](Self::paint_and_update_textures)
    ///
    /// # Portability
    ///
    /// _In particular it's important to note that on Android a it's only possible to create
    /// a window surface between `Resumed` and `Paused` lifecycle events, and Winit will panic on
    /// attempts to query the raw window handle while paused._
    ///
    /// On Android [`set_window`](Self::set_window) should be called with `Some(window)` for each
    /// `Resumed` event and `None` for each `Paused` event. Currently, on all other platforms
    /// [`set_window`](Self::set_window) may be called with `Some(window)` as soon as you have a
    /// valid [`winit::window::Window`].
    ///
    /// # Errors
    /// If the provided wgpu configuration does not match an available device.
    pub async fn set_window(
        &mut self,
        viewport_id: ViewportId,
        window: Option<Arc<winit::window::Window>>,
    ) -> Result<(), crate::WgpuError> {
        profiling::scope!("Painter::set_window"); // profile_function gives bad names for async functions

        if let Some(window) = window {
            let size = window.inner_size();
            if !self.surfaces.contains_key(&viewport_id) {
                #[cfg(all(windows, feature = "direct-composition"))]
                if self.configuration.direct_composition && viewport_id == ViewportId::ROOT {
                    return self.add_direct_composition_surface(window).await;
                }

                let surface = self.instance.create_surface(window)?;
                self.add_surface(surface, viewport_id, size).await?;
            }
        } else {
            #[cfg(all(windows, feature = "direct-composition"))]
            if self.configuration.direct_composition {
                self.surfaces.remove(&viewport_id);
                return Ok(());
            }

            log::warn!("No window - clearing all surfaces");
            self.surfaces.clear();
        }
        Ok(())
    }

    /// Updates (or clears) the [`winit::window::Window`] associated with the [`Painter`] without taking ownership of the window.
    ///
    /// Like [`set_window`](Self::set_window) except:
    ///
    /// # Safety
    /// The user is responsible for ensuring that the window is alive for as long as it is set.
    pub async unsafe fn set_window_unsafe(
        &mut self,
        viewport_id: ViewportId,
        window: Option<&winit::window::Window>,
    ) -> Result<(), crate::WgpuError> {
        profiling::scope!("Painter::set_window_unsafe"); // profile_function gives bad names for async functions

        if let Some(window) = window {
            let size = window.inner_size();
            if !self.surfaces.contains_key(&viewport_id) {
                let surface = unsafe {
                    self.instance
                        .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_window(&window)?)?
                };
                self.add_surface(surface, viewport_id, size).await?;
            }
        } else {
            log::warn!("No window - clearing all surfaces");
            self.surfaces.clear();
        }
        Ok(())
    }

    #[cfg(all(windows, feature = "direct-composition"))]
    async fn add_direct_composition_surface(
        &mut self,
        window: Arc<winit::window::Window>,
    ) -> Result<(), crate::WgpuError> {
        let size = window.inner_size();
        if self.render_state.is_none() {
            self.render_state = Some(
                RenderState::create(
                    &self.configuration,
                    &self.instance,
                    None,
                    self.depth_format,
                    self.msaa_samples,
                    self.dithering,
                )
                .await?,
            );
        }
        let adapter = &self.render_state.as_ref().unwrap().adapter;
        let (surface, target) = create_direct_composition_surface(window, &self.instance, adapter)?;
        if !surface
            .get_capabilities(adapter)
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            return Err(crate::WgpuError::DirectComposition(
                "交换链不支持预乘 alpha".into(),
            ));
        }
        self.install_surface(
            ViewportId::ROOT,
            SurfaceState {
                surface,
                width: size.width.max(1),
                height: size.height.max(1),
                alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
                dcomp: Some(target),
            },
        )
    }

    async fn add_surface(
        &mut self,
        surface: wgpu::Surface<'static>,
        viewport_id: ViewportId,
        size: winit::dpi::PhysicalSize<u32>,
    ) -> Result<(), crate::WgpuError> {
        let render_state = if let Some(render_state) = &self.render_state {
            render_state
        } else {
            let render_state = RenderState::create(
                &self.configuration,
                &self.instance,
                Some(&surface),
                self.depth_format,
                self.msaa_samples,
                self.dithering,
            )
            .await?;
            self.render_state.get_or_insert(render_state)
        };
        let alpha_mode = if self.support_transparent_backbuffer {
            let supported_alpha_modes = surface.get_capabilities(&render_state.adapter).alpha_modes;

            // Prefer pre multiplied over post multiplied!
            if supported_alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
                wgpu::CompositeAlphaMode::PreMultiplied
            } else if supported_alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
                wgpu::CompositeAlphaMode::PostMultiplied
            } else {
                log::warn!("Transparent window was requested, but the active wgpu surface does not support a `CompositeAlphaMode` with transparency.");
                wgpu::CompositeAlphaMode::Auto
            }
        } else {
            wgpu::CompositeAlphaMode::Auto
        };
        self.install_surface(
            viewport_id,
            SurfaceState {
                surface,
                width: size.width,
                height: size.height,
                alpha_mode,
                #[cfg(all(windows, feature = "direct-composition"))]
                dcomp: None,
            },
        )
    }

    fn install_surface(
        &mut self,
        viewport_id: ViewportId,
        surface: SurfaceState,
    ) -> Result<(), crate::WgpuError> {
        let width = NonZeroU32::new(surface.width);
        let height = NonZeroU32::new(surface.height);
        self.surfaces.insert(viewport_id, surface);
        let Some(width) = width else {
            log::debug!("The window width was zero; skipping generate textures");
            return Ok(());
        };
        let Some(height) = height else {
            log::debug!("The window height was zero; skipping generate textures");
            return Ok(());
        };
        if let Err(error) =
            self.resize_and_generate_depth_texture_view_and_msaa_view(viewport_id, width, height)
        {
            self.surfaces.remove(&viewport_id);
            return Err(error);
        }
        Ok(())
    }

    /// Returns the maximum texture dimension supported if known
    ///
    /// This API will only return a known dimension after `set_window()` has been called
    /// at least once, since the underlying device and render state are initialized lazily
    /// once we have a window (that may determine the choice of adapter/device).
    pub fn max_texture_side(&self) -> Option<usize> {
        self.render_state
            .as_ref()
            .map(|rs| rs.device.limits().max_texture_dimension_2d as usize)
    }

    fn resize_and_generate_depth_texture_view_and_msaa_view(
        &mut self,
        viewport_id: ViewportId,
        width_in_pixels: NonZeroU32,
        height_in_pixels: NonZeroU32,
    ) -> Result<(), crate::WgpuError> {
        profiling::function_scope!();

        let width = width_in_pixels.get();
        let height = height_in_pixels.get();

        let render_state = self.render_state.as_ref().unwrap();
        let surface_state = self.surfaces.get_mut(&viewport_id).unwrap();

        surface_state.width = width;
        surface_state.height = height;

        Self::configure_surface(surface_state, render_state, &self.configuration)?;

        if let Some(depth_format) = self.depth_format {
            self.depth_texture_view.insert(
                viewport_id,
                render_state
                    .device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("egui_depth_texture"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: self.msaa_samples,
                        dimension: wgpu::TextureDimension::D2,
                        format: depth_format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[depth_format],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default()),
            );
        }

        if let Some(render_state) = (self.msaa_samples > 1)
            .then_some(self.render_state.as_ref())
            .flatten()
        {
            let texture_format = render_state.target_format;
            self.msaa_texture_view.insert(
                viewport_id,
                render_state
                    .device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("egui_msaa_texture"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: self.msaa_samples,
                        dimension: wgpu::TextureDimension::D2,
                        format: texture_format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[texture_format],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default()),
            );
        };
        Ok(())
    }

    pub fn on_window_resized(
        &mut self,
        viewport_id: ViewportId,
        width_in_pixels: NonZeroU32,
        height_in_pixels: NonZeroU32,
    ) {
        profiling::function_scope!();

        if self.surfaces.contains_key(&viewport_id) {
            if let Err(error) = self.resize_and_generate_depth_texture_view_and_msaa_view(
                viewport_id,
                width_in_pixels,
                height_in_pixels,
            ) {
                #[cfg(not(all(windows, feature = "direct-composition")))]
                log::warn!("{error}");
                #[cfg(all(windows, feature = "direct-composition"))]
                {
                    self.direct_composition_error = Some(error);
                }
            }
        } else {
            log::warn!("Ignoring window resize notification with no surface created via Painter::set_window()");
        }
    }

    /// Returns two things:
    ///
    /// The approximate number of seconds spent on vsync-waiting (if any),
    /// and the captures captured screenshot if it was requested.
    ///
    /// If `capture_data` isn't empty, a screenshot will be captured.
    pub fn paint_and_update_textures(
        &mut self,
        viewport_id: ViewportId,
        pixels_per_point: f32,
        clear_color: [f32; 4],
        clipped_primitives: &[epaint::ClippedPrimitive],
        textures_delta: &epaint::textures::TexturesDelta,
        capture_data: Vec<UserData>,
    ) -> f32 {
        profiling::function_scope!();

        let capture = !capture_data.is_empty();
        let mut vsync_sec = 0.0;

        let Some(render_state) = self.render_state.as_mut() else {
            return vsync_sec;
        };
        let Some(surface_state) = self.surfaces.get(&viewport_id) else {
            return vsync_sec;
        };

        let mut encoder =
            render_state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("encoder"),
                });

        // Upload all resources for the GPU.
        let screen_descriptor = renderer::ScreenDescriptor {
            size_in_pixels: [surface_state.width, surface_state.height],
            pixels_per_point,
        };

        let user_cmd_bufs = {
            let mut renderer = render_state.renderer.write();
            for (id, image_delta) in &textures_delta.set {
                renderer.update_texture(
                    &render_state.device,
                    &render_state.queue,
                    *id,
                    image_delta,
                );
            }

            renderer.update_buffers(
                &render_state.device,
                &render_state.queue,
                &mut encoder,
                clipped_primitives,
                &screen_descriptor,
            )
        };

        let output_frame = {
            profiling::scope!("get_current_texture");
            // This is what vsync-waiting happens on my Mac.
            let start = web_time::Instant::now();
            let output_frame = surface_state.surface.get_current_texture();
            vsync_sec += start.elapsed().as_secs_f32();
            output_frame
        };

        let output_frame = match output_frame {
            Ok(frame) => frame,
            #[cfg(all(windows, feature = "direct-composition"))]
            Err(err) if surface_state.dcomp.is_some() => {
                self.direct_composition_error = match err {
                    wgpu::SurfaceError::Timeout => None,
                    wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                        Self::configure_surface(surface_state, render_state, &self.configuration)
                            .err()
                    }
                    error => Some(crate::WgpuError::DirectComposition(error.to_string())),
                };
                return vsync_sec;
            }
            Err(err) => match (*self.configuration.on_surface_error)(err) {
                SurfaceErrorAction::RecreateSurface => {
                    if let Err(error) =
                        Self::configure_surface(surface_state, render_state, &self.configuration)
                    {
                        log::warn!("{error}");
                    }
                    return vsync_sec;
                }
                SurfaceErrorAction::SkipFrame => {
                    return vsync_sec;
                }
            },
        };

        let mut capture_buffer = None;
        {
            let renderer = render_state.renderer.read();

            let target_texture = if capture {
                let capture_state = self.screen_capture_state.get_or_insert_with(|| {
                    CaptureState::new(&render_state.device, &output_frame.texture)
                });
                capture_state.update(&render_state.device, &output_frame.texture);

                &capture_state.texture
            } else {
                &output_frame.texture
            };
            let target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());

            let (view, resolve_target) = (self.msaa_samples > 1)
                .then_some(self.msaa_texture_view.get(&viewport_id))
                .flatten()
                .map_or((&target_view, None), |texture_view| {
                    (texture_view, Some(&target_view))
                });

            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui_render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear_color[0] as f64,
                            g: clear_color[1] as f64,
                            b: clear_color[2] as f64,
                            a: clear_color[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: self.depth_texture_view.get(&viewport_id).map(|view| {
                    wgpu::RenderPassDepthStencilAttachment {
                        view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            // It is very unlikely that the depth buffer is needed after egui finished rendering
                            // so no need to store it. (this can improve performance on tiling GPUs like mobile chips or Apple Silicon)
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Forgetting the pass' lifetime means that we are no longer compile-time protected from
            // runtime errors caused by accessing the parent encoder before the render pass is dropped.
            // Since we don't pass it on to the renderer, we should be perfectly safe against this mistake here!
            renderer.render(
                &mut render_pass.forget_lifetime(),
                clipped_primitives,
                &screen_descriptor,
            );

            if capture {
                if let Some(capture_state) = &mut self.screen_capture_state {
                    capture_buffer = Some(capture_state.copy_textures(
                        &render_state.device,
                        &output_frame,
                        &mut encoder,
                    ));
                }
            }
        }

        let encoded = {
            profiling::scope!("CommandEncoder::finish");
            encoder.finish()
        };

        // Submit the commands: both the main buffer and user-defined ones.
        {
            profiling::scope!("Queue::submit");
            // wgpu doesn't document where vsync can happen. Maybe here?
            let start = web_time::Instant::now();
            render_state
                .queue
                .submit(user_cmd_bufs.into_iter().chain([encoded]));
            vsync_sec += start.elapsed().as_secs_f32();
        };

        // Free textures marked for destruction **after** queue submit since they might still be used in the current frame.
        // Calling `wgpu::Texture::destroy` on a texture that is still in use would invalidate the command buffer(s) it is used in.
        // However, once we called `wgpu::Queue::submit`, it is up for wgpu to determine how long the underlying gpu resource has to live.
        {
            let mut renderer = render_state.renderer.write();
            for id in &textures_delta.free {
                renderer.free_texture(id);
            }
        }

        if let Some(capture_buffer) = capture_buffer {
            if let Some(screen_capture_state) = &mut self.screen_capture_state {
                screen_capture_state.read_screen_rgba(
                    self.context.clone(),
                    capture_buffer,
                    capture_data,
                    self.capture_tx.clone(),
                    viewport_id,
                );
            }
        }

        {
            profiling::scope!("present");
            // wgpu doesn't document where vsync can happen. Maybe here?
            let start = web_time::Instant::now();
            output_frame.present();
            vsync_sec += start.elapsed().as_secs_f32();
        }

        vsync_sec
    }

    /// Call this at the beginning of each frame to receive the requested screenshots.
    pub fn handle_screenshots(&self, events: &mut Vec<Event>) {
        for (viewport_id, user_data, screenshot) in self.capture_rx.try_iter() {
            let screenshot = Arc::new(screenshot);
            for data in user_data {
                events.push(Event::Screenshot {
                    viewport_id,
                    user_data: data,
                    image: screenshot.clone(),
                });
            }
        }
    }

    pub fn gc_viewports(&mut self, active_viewports: &ViewportIdSet) {
        self.surfaces.retain(|id, _| active_viewports.contains(id));
        self.depth_texture_view
            .retain(|id, _| active_viewports.contains(id));
        self.msaa_texture_view
            .retain(|id, _| active_viewports.contains(id));
    }

    #[allow(clippy::needless_pass_by_ref_mut, clippy::unused_self)]
    pub fn destroy(&mut self) {
        // TODO(emilk): something here?
    }
}
