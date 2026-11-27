//! 只在材质请求时读取岛体背后的桌面像素。

use std::time::{Duration, Instant};

use eframe::egui::Rect;
use windows::core::{factory, Error, Interface};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{
    BOOL, E_FAIL, HMODULE, HWND, LPARAM, POINT, RECT, RPC_E_CHANGED_MODE, S_OK,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{
    RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED,
};

use super::log;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ScreenRect {
    fn from_edges(left: i32, top: i32, right: i32, bottom: i32) -> Option<Self> {
        let width = right.checked_sub(left)?;
        let height = bottom.checked_sub(top)?;
        (width > 0 && height > 0).then_some(Self {
            x: left,
            y: top,
            width,
            height,
        })
    }

    fn source_box(self, monitor: Self) -> Option<D3D11_BOX> {
        let left = self.x.checked_sub(monitor.x)?;
        let top = self.y.checked_sub(monitor.y)?;
        let right = left.checked_add(self.width)?;
        let bottom = top.checked_add(self.height)?;
        if left < 0 || top < 0 || right > monitor.width || bottom > monitor.height {
            return None;
        }
        Some(D3D11_BOX {
            left: left as u32,
            top: top as u32,
            right: right as u32,
            bottom: bottom as u32,
            front: 0,
            back: 1,
        })
    }

    fn row_bytes(self) -> usize {
        self.width as usize * 4
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// 抓取矩形在桌面上的物理像素坐标，允许负坐标。
    pub screen: ScreenRect,
    /// 岛体在抓取矩形内的 [x, y, w, h]，单位为物理像素。
    pub body: [f32; 4],
    pub pixels_per_point: f32,
}

impl Geometry {
    pub(super) fn for_window(
        hwnd: HWND,
        body: Rect,
        capture: Rect,
        ppp: f32,
    ) -> Result<Option<(Self, Rect)>, String> {
        use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

        let mut client = RECT::default();
        unsafe { GetClientRect(hwnd, &mut client) }
            .map_err(|err| format!("读取岛体客户区：{err}"))?;
        let mut at = POINT::default();
        if !unsafe { ClientToScreen(hwnd, &mut at) }.as_bool() {
            return Err(format!("读取岛体屏幕原点：{}", Error::from_win32()));
        }
        let origin = (at.x, at.y);
        let Some(body_geometry) = Self::from_rects(body, body, origin, ppp) else {
            return Ok(None);
        };
        let Some(monitor) = Monitor::for_rect(body_geometry.screen) else {
            return Err("无法定位岛体所在显示器".into());
        };
        let bounds = monitor.bounds;
        let clip = Rect::from_min_max(
            eframe::egui::pos2(
                (bounds.x - at.x).max(client.left) as f32 / ppp,
                (bounds.y - at.y).max(client.top) as f32 / ppp,
            ),
            eframe::egui::pos2(
                (bounds.x + bounds.width - at.x).min(client.right) as f32 / ppp,
                (bounds.y + bounds.height - at.y).min(client.bottom) as f32 / ppp,
            ),
        );
        let clipped = capture.intersect(clip);
        Ok(Self::from_rects(body, clipped, origin, ppp).map(|geometry| (geometry, clipped)))
    }

    /// `capture` 是已裁剪到窗口和当前显示器交集的外扩矩形，`origin` 是客户区的屏幕物理坐标。
    pub fn from_rects(body: Rect, capture: Rect, origin: (i32, i32), ppp: f32) -> Option<Self> {
        if !capture.is_finite() || !body.is_finite() || !ppp.is_finite() || ppp <= 0.0 {
            return None;
        }
        // 与 egui 的 callback viewport 一致，两条边分别取整。岛体保留亚像素精度。
        let left = (capture.min.x * ppp).round();
        let top = (capture.min.y * ppp).round();
        let screen = ScreenRect::from_edges(
            origin.0.checked_add(left as i32)?,
            origin.1.checked_add(top as i32)?,
            origin.0.checked_add((capture.max.x * ppp).round() as i32)?,
            origin.1.checked_add((capture.max.y * ppp).round() as i32)?,
        )?;
        Some(Self {
            screen,
            body: [
                body.min.x * ppp - left,
                body.min.y * ppp - top,
                body.width() * ppp,
                body.height() * ppp,
            ],
            pixels_per_point: ppp,
        })
    }

    fn island_bounds(self) -> [i32; 4] {
        let [x, y, w, h] = self.body;
        [
            ((x - 10.0) as i32).clamp(0, self.screen.width),
            ((y - 10.0) as i32).clamp(0, self.screen.height),
            ((x + w + 10.0) as i32).clamp(0, self.screen.width),
            ((y + h + 10.0) as i32).clamp(0, self.screen.height),
        ]
    }

    fn theme_bounds(self) -> [i32; 4] {
        // 采样框锚在岛体上缘，切换页面高度时保持背景亮度稳定。
        let [x, y, w, _] = self.body;
        let ppp = self.pixels_per_point;
        let width = ((520.0 * ppp).round() as i32).clamp(1, self.screen.width);
        let height = ((120.0 * ppp).round() as i32).clamp(1, self.screen.height);
        let left = (((x + w * 0.5).round() as i32) - width / 2).clamp(0, self.screen.width - width);
        let top = ((y - 6.0 * ppp).round() as i32).clamp(0, self.screen.height - height);
        [left, top, left + width, top + height]
    }
}

/// 从上到下排列、无行填充的 BGRA 底片，亮度范围为 0 到 1。
pub struct CleanPlate {
    pub geometry: Geometry,
    pub bgra: Vec<u8>,
    pub brightness: f32,
    pub(crate) revision: u64,
}

/// 主岛专用抓屏会话。创建时无副作用，在主岛线程首次调用 `capture` 时打开。
#[derive(Default)]
pub struct Backdrop {
    source: Option<Wgc>,
    topology: Option<Vec<(isize, ScreenRect)>>,
    next_topology_check: Option<Instant>,
    settle_until: Option<Instant>,
    waiting_since: Option<Instant>,
    plate: Option<CleanPlate>,
    revision: u64,
    failed: Option<String>,
}

impl Backdrop {
    pub(super) fn current(&self, geometry: Geometry) -> Option<&CleanPlate> {
        self.plate
            .as_ref()
            .filter(|plate| plate.geometry.screen == geometry.screen)
    }

    /// 原生窗口确认截图排除后，返回与当前几何对齐的底片。
    /// 失败会关闭本次会话，重新启用材质时创建新的 `Backdrop`。
    pub(super) fn capture(&mut self, geometry: Geometry) -> Result<Option<&CleanPlate>, &str> {
        if self.failed.is_none() {
            if let Err(error) = self.capture_frame(geometry) {
                log(&format!("抓屏失败，退回基础皮肤：{error}"));
                self.fail(error);
            }
        }
        if let Some(error) = &self.failed {
            return Err(error);
        }
        Ok(self.plate.as_ref())
    }

    fn fail(&mut self, error: String) {
        self.source = None;
        self.plate = None;
        self.failed = Some(error);
    }

    fn capture_frame(&mut self, geometry: Geometry) -> Result<(), String> {
        if self
            .plate
            .as_ref()
            .is_some_and(|plate| plate.geometry.screen != geometry.screen)
        {
            self.plate = None;
        }
        let rect = geometry.screen;
        if rect.width <= 0 || rect.height <= 0 {
            return Ok(());
        }
        let now = Instant::now();
        if self.next_topology_check.map_or(true, |next| now >= next) {
            self.next_topology_check = Some(now + Duration::from_millis(100));
            let topology = desktop_topology();
            if self
                .topology
                .as_ref()
                .is_some_and(|previous| *previous != topology)
            {
                self.source = None;
                self.plate = None;
                self.settle_until = Some(now + Duration::from_millis(1000));
                log("显示器拓扑变化，等待桌面稳定后重建抓屏会话");
            }
            self.topology = Some(topology);
        }
        if self.settle_until.is_some_and(|until| now < until)
            || self.topology.as_ref().is_some_and(Vec::is_empty)
        {
            return Ok(());
        }
        let Some(monitor) = Monitor::for_rect(rect) else {
            return Err("无法定位岛体所在显示器".into());
        };
        let Some(source_box) = rect.source_box(monitor.bounds) else {
            // 窗口在切屏时有可能还没移到新屏，先用基础皮肤等待下一帧。
            self.plate = None;
            self.waiting_since = None;
            return Ok(());
        };

        if self
            .source
            .as_ref()
            .is_some_and(|source| source.monitor != monitor)
        {
            self.source = None;
            self.plate = None;
        }
        if self.source.is_none() {
            self.source = Some(Wgc::new(monitor).map_err(|err| format!("启动 WGC：{err}"))?);
            self.waiting_since = None;
        }
        let source = self.source.as_mut().unwrap();
        let raw = source
            .read(rect, &source_box, self.plate.is_none())
            .map_err(|err| format!("读取 WGC 帧：{err}"))?;
        if let Some(raw) = raw {
            let island = geometry.island_bounds();
            let (inner, outer) = split_luma(&raw, rect, island);
            // 排除属性生效前可能有黑岛残留，均匀的深色桌面仍然可以作为底片。
            if inner < 45.0 && outer - inner > 80.0 {
                self.waiting_since.get_or_insert(now);
                if let Some(plate) = &mut self.plate {
                    if let Some(raw) = grab_raw(rect) {
                        compose_with_plate(plate, &raw, island);
                        self.revision += 1;
                        plate.revision = self.revision;
                    }
                }
            } else {
                self.waiting_since = None;
                let (brightness, _) = split_luma(&raw, rect, geometry.theme_bounds());
                self.revision += 1;
                self.plate = Some(CleanPlate {
                    geometry,
                    bgra: raw,
                    brightness: brightness / 255.0,
                    revision: self.revision,
                });
            }
        }
        if self.plate.is_none() {
            self.waiting_since.get_or_insert(now);
        }
        // 静止桌面暂无新帧时可沿用对齐的底片。等待首帧或持续出现污染帧时才超时。
        if self
            .waiting_since
            .is_some_and(|since| now - since >= Duration::from_secs(2))
        {
            return Err("WGC 未能提供可用的桌面帧".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Monitor {
    handle: HMONITOR,
    bounds: ScreenRect,
}

impl Monitor {
    fn from_handle(handle: HMONITOR) -> Option<Self> {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() {
            return None;
        }
        let rect = info.rcMonitor;
        let bounds = ScreenRect::from_edges(rect.left, rect.top, rect.right, rect.bottom)?;
        Some(Self { handle, bounds })
    }

    fn for_rect(rect: ScreenRect) -> Option<Self> {
        let center = POINT {
            x: rect.x + rect.width / 2,
            y: rect.y + rect.height / 2,
        };
        Self::from_handle(unsafe { MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST) })
    }
}

fn desktop_topology() -> Vec<(isize, ScreenRect)> {
    unsafe extern "system" fn collect(
        handle: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        if let Some(monitor) = Monitor::from_handle(handle) {
            let monitors = unsafe { &mut *(data.0 as *mut Vec<(isize, ScreenRect)>) };
            monitors.push((handle.0 as isize, monitor.bounds));
        }
        BOOL(1)
    }

    let mut monitors = Vec::new();
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut monitors as *mut _ as isize),
        )
    };
    monitors.sort_unstable_by_key(|(handle, _)| *handle);
    monitors
}

struct Apartment;

impl Apartment {
    fn enter() -> windows::core::Result<Self> {
        // winit 的 OLE 拖放会先在 UI 线程建立 STA。
        match unsafe { RoInitialize(RO_INIT_SINGLETHREADED) } {
            Err(err) if err.code() == RPC_E_CHANGED_MODE => unsafe {
                RoInitialize(RO_INIT_MULTITHREADED)
            },
            result => result,
        }?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

struct Wgc {
    monitor: Monitor,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    _item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    latest: Option<Direct3D11CaptureFrame>,
    staging: Option<((i32, i32), ID3D11Texture2D)>,
    _apartment: Apartment,
}

impl Wgc {
    fn new(monitor: Monitor) -> windows::core::Result<Self> {
        let apartment = Apartment::enter()?;
        unsafe {
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device = device.ok_or(E_FAIL)?;
            let context = context.ok_or(E_FAIL)?;
            let dxgi: IDXGIDevice = device.cast()?;
            let interop_device: IDirect3DDevice =
                CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = interop.CreateForMonitor(monitor.handle)?;
            let size = SizeInt32 {
                Width: monitor.bounds.width,
                Height: monitor.bounds.height,
            };
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &interop_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )?;
            let session = pool.CreateCaptureSession(&item)?;
            let source = Self {
                monitor,
                device,
                context,
                _item: item,
                pool,
                session,
                latest: None,
                staging: None,
                _apartment: apartment,
            };
            let _ = source.session.SetIsCursorCaptureEnabled(false);
            if let Err(err) = source.session.SetIsBorderRequired(false) {
                log(&format!("保留系统捕获边框：{err}"));
            }
            source.session.StartCapture()?;
            log(&format!("抓屏会话已启动：{:?}", monitor.bounds));
            Ok(source)
        }
    }

    fn read(
        &mut self,
        rect: ScreenRect,
        src_box: &D3D11_BOX,
        recrop: bool,
    ) -> windows::core::Result<Option<Vec<u8>>> {
        let mut fresh = false;
        for _ in 0..2 {
            match self.pool.TryGetNextFrame() {
                Ok(frame) => {
                    fresh = true;
                    if let Some(old) = self.latest.replace(frame) {
                        let _ = old.Close();
                    }
                }
                // windows-rs 0.58 将成功返回的空接口投影为 Error::empty()，其错误码为 S_OK。
                Err(err) if err.code() == S_OK => break,
                Err(err) => return Err(err),
            }
        }
        if !fresh && !recrop {
            return Ok(None);
        }
        let Some(frame) = &self.latest else {
            return Ok(None);
        };
        let size = frame.ContentSize()?;
        if size.Width != self.monitor.bounds.width || size.Height != self.monitor.bounds.height {
            return Ok(None);
        }
        let size = (rect.width, rect.height);
        if self
            .staging
            .as_ref()
            .map_or(true, |(current, _)| *current != size)
        {
            // 保留最近一帧的全屏源，尺寸变化时仍从源重新裁剪，不沿用旧尺寸的纹理。
            self.staging = Some((size, new_staging(&self.device, size)?));
        }
        let (_, staging) = self.staging.as_ref().unwrap();
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        let mut pixels = vec![0; rect.row_bytes() * rect.height as usize];
        unsafe {
            self.context
                .CopySubresourceRegion(staging, 0, 0, 0, 0, &texture, 0, Some(src_box));
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let pitch = mapped.RowPitch as usize;
            let valid = !mapped.pData.is_null() && pitch >= rect.row_bytes();
            if valid {
                for (row, dest) in pixels.chunks_exact_mut(rect.row_bytes()).enumerate() {
                    let from = (mapped.pData as *const u8).add(row * pitch);
                    dest.copy_from_slice(std::slice::from_raw_parts(from, rect.row_bytes()));
                }
            }
            self.context.Unmap(staging, 0);
            if !valid {
                return Err(Error::from(E_FAIL));
            }
        }
        Ok(Some(pixels))
    }
}

impl Drop for Wgc {
    fn drop(&mut self) {
        if let Some(frame) = self.latest.take() {
            let _ = frame.Close();
        }
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

fn new_staging(
    device: &ID3D11Device,
    (width, height): (i32, i32),
) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width as u32,
        Height: height as u32,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        ..Default::default()
    };
    let mut staging = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging))? };
    Ok(staging.ok_or(E_FAIL)?)
}

/// GDI 可能读到岛体自身，只能为已存在的干净底片提供岛体外像素。
fn grab_raw(rect: ScreenRect) -> Option<Vec<u8>> {
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let memory = CreateCompatibleDC(screen);
        let result = (|| {
            if memory.is_invalid() {
                return None;
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: rect.width,
                    biHeight: -rect.height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap =
                CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let bitmap = HGDIOBJ(bitmap.0);
            let previous = SelectObject(memory, bitmap);
            let pixels = if !previous.is_invalid() {
                let ok = BitBlt(
                    memory,
                    0,
                    0,
                    rect.width,
                    rect.height,
                    screen,
                    rect.x,
                    rect.y,
                    SRCCOPY,
                )
                .is_ok();
                let _ = SelectObject(memory, previous);
                if ok && !bits.is_null() {
                    let _ = GdiFlush();
                    Some(
                        std::slice::from_raw_parts(
                            bits.cast::<u8>(),
                            rect.row_bytes() * rect.height as usize,
                        )
                        .to_vec(),
                    )
                } else {
                    None
                }
            } else {
                None
            };
            let _ = DeleteObject(bitmap);
            pixels
        })();
        if !memory.is_invalid() {
            let _ = DeleteDC(memory);
        }
        let _ = ReleaseDC(None, screen);
        result
    }
}

fn split_luma(pixels: &[u8], rect: ScreenRect, [left, top, right, bottom]: [i32; 4]) -> (f32, f32) {
    let mut sum = [0u64; 2];
    let mut count = [0u32; 2];
    for y in (0..rect.height).step_by(4) {
        for x in (0..rect.width).step_by(4) {
            let segment = usize::from(!(left <= x && x < right && top <= y && y < bottom));
            let start = y as usize * rect.row_bytes() + x as usize * 4;
            sum[segment] += u64::from(pixels[start])
                + u64::from(pixels[start + 1])
                + u64::from(pixels[start + 2]);
            count[segment] += 3;
        }
    }
    (
        sum[0] as f32 / count[0].max(1) as f32,
        sum[1] as f32 / count[1].max(1) as f32,
    )
}

fn compose_with_plate(plate: &mut CleanPlate, raw: &[u8], [left, top, right, bottom]: [i32; 4]) {
    let stride = plate.geometry.screen.row_bytes();
    for (y, (dest, src)) in plate
        .bgra
        .chunks_exact_mut(stride)
        .zip(raw.chunks_exact(stride))
        .enumerate()
    {
        if y < top as usize || y >= bottom as usize {
            dest.copy_from_slice(src);
        } else {
            let left = left as usize * 4;
            let right = right as usize * 4;
            dest[..left].copy_from_slice(&src[..left]);
            dest[right..].copy_from_slice(&src[right..]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn geometry() -> Geometry {
        Geometry::from_rects(
            Rect::from_min_max(pos2(20.0, 8.0), pos2(80.0, 32.0)),
            Rect::from_min_max(pos2(10.2, 4.7), pos2(90.4, 40.0)),
            (-1200, 200),
            1.5,
        )
        .unwrap()
    }

    fn plate(geometry: Geometry) -> CleanPlate {
        let screen = geometry.screen;
        CleanPlate {
            geometry,
            bgra: vec![77; screen.row_bytes() * screen.height as usize],
            brightness: 0.3,
            revision: 1,
        }
    }

    #[test]
    fn geometry_maps_fractional_points_to_monitor_pixels() {
        let geometry = geometry();
        assert_eq!(
            geometry.screen,
            ScreenRect {
                x: -1185,
                y: 207,
                width: 121,
                height: 53
            }
        );
        assert_eq!(geometry.body, [15.0, 5.0, 90.0, 36.0]);
        let monitor = ScreenRect {
            x: -1920,
            y: 100,
            width: 1920,
            height: 1080,
        };
        let crop = geometry.screen.source_box(monitor).unwrap();
        assert_eq!(
            [crop.left, crop.top, crop.right, crop.bottom],
            [735, 107, 856, 160]
        );
        let outside = ScreenRect {
            x: -1930,
            ..geometry.screen
        };
        assert!(outside.source_box(monitor).is_none());
    }

    #[test]
    fn theme_sampling_stays_anchored_when_body_height_changes() {
        let mut geometry = geometry();
        geometry.screen.height = 300;
        let before = geometry.theme_bounds();
        geometry.body[3] *= 3.0;
        assert_eq!(before, geometry.theme_bounds());
    }

    #[test]
    fn resizing_invalidates_previous_plate_while_waiting_for_pixels() {
        let mut geometry = geometry();
        let mut backdrop = Backdrop {
            plate: Some(plate(geometry)),
            ..Default::default()
        };
        backdrop.next_topology_check = Some(Instant::now() + Duration::from_secs(60));
        backdrop.settle_until = backdrop.next_topology_check;
        geometry.screen.width += 1;
        assert!(backdrop
            .capture(geometry)
            .unwrap()
            .is_none());
    }

    #[test]
    fn failed_capture_discards_plate_and_stays_disabled() {
        let geometry = geometry();
        let mut backdrop = Backdrop {
            plate: Some(plate(geometry)),
            ..Default::default()
        };
        backdrop.fail("测试抓屏失败".into());
        assert!(backdrop.plate.is_none());
        assert!(backdrop.source.is_none());
        assert!(matches!(
            backdrop.capture(geometry),
            Err("测试抓屏失败")
        ));
    }

    #[test]
    fn compositing_never_writes_gdi_pixels_inside_the_island() {
        let geometry = geometry();
        let mut plate = plate(geometry);
        let raw = vec![23; plate.bgra.len()];
        compose_with_plate(&mut plate, &raw, [2, 1, 8, 3]);
        let row = geometry.screen.row_bytes();
        assert_eq!(plate.bgra[0], 23);
        assert_eq!(plate.bgra[row + 4], 23);
        assert_eq!(plate.bgra[row + 8], 77);
        assert_eq!(plate.bgra[row * 2 + 7 * 4], 77);
        assert_eq!(plate.bgra[row * 2 + 8 * 4], 23);
        assert_eq!(plate.bgra[row * 3 + 8], 23);
    }
}
