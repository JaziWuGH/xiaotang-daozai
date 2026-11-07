//! QQ 音乐播放模式联动：读底栏模式按钮名 + 从岛上直接切到下一档。
//!
//! 实机排查过的通道（PID 118452，QQ 音乐收在托盘时 47 个顶层窗口全隐藏）：
//! - SMTC 不上报 `AutoRepeatMode` / `IsShuffleActive`，反向设置返回 true 但不生效；
//! - 全局热键原子只有 `Tencent - QQMusicPlayer - 0xB0~B3` 与 Player_Status 等，没有模式命令；
//! - UIA `InvokePattern::Invoke` / `LegacyIAccessible::DoDefaultAction` 都抛 0x80131509，
//!   即使把窗口显示出来（offscreen=false）也一样；
//! - 按客户区坐标投递 WM_LBUTTONDOWN/UP 对自绘 UI 无效（最小化后 UIA 是逻辑布局坐标，
//!   与客户区 160×28 对不上）；
//! - `QQMusicSvr.QQMusicControl` COM 组件实例化会挂起；配置文件里没有播放模式字段。
//!
//! 能走通的是 **SendInput 真实鼠标点击**：点底栏按钮会弹出四档模式菜单
//! （UIA `ct=50020` 文本项：随机播放 / 顺序播放 / 单曲循环 / 列表循环），再点其中一项
//! 才会真正改模式——这就是「点了没反应」的原因：旧实现只把窗口调到前台，
//! 而且从临时线程调 `SetForegroundWindow` 会被 Windows 前台锁挡住。
//!
//! 因此点击岛上按钮时：短暂显示主窗口并抢前台（AttachThreadInput + Alt 解锁），
//! 点开菜单、按网易云同一套循环顺序点下一档，读回新模式，最后把光标、窗口可见性
//! 与最小化状态全部复原。全程约 1 秒，不播放不受影响；读取仍走 UIA，托盘态也能读。

use super::PlayMode;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, AtomicU8, Ordering};

/// QQ 音乐底栏模式按钮 / 菜单项的中文名。按钮名就是当前模式，随切换实时变化。
pub(super) const MODE_NAMES: [&str; 7] = [
    "列表循环", "单曲循环", "随机播放", "顺序播放", "循环播放", "心动模式", "智能模式",
];

/// 名称 → 岛仔播放模式。「循环播放」是部分版本对列表循环的叫法；
/// 「智能模式」与网易云的心动模式同属推荐流，画同一颗心。
pub(super) fn mode_from_button(name: &str) -> Option<PlayMode> {
    match name.trim() {
        "列表循环" | "循环播放" => Some(PlayMode::Cycle),
        "单曲循环" => Some(PlayMode::OneCycle),
        "随机播放" => Some(PlayMode::Random),
        "顺序播放" => Some(PlayMode::Order),
        "心动模式" | "智能模式" => Some(PlayMode::Heart),
        _ => None,
    }
}

/// 与网易云 `netease_playback::next_mode` 同一套循环顺序，跳过 QQ 音乐没有的心动模式：
/// 随机 → 顺序 → 列表循环 → 单曲循环 → 随机。
pub(super) fn next_mode(mode: PlayMode) -> PlayMode {
    match mode {
        PlayMode::Random => PlayMode::Order,
        PlayMode::Order | PlayMode::Heart => PlayMode::Cycle,
        PlayMode::Cycle => PlayMode::OneCycle,
        PlayMode::OneCycle | PlayMode::Unknown => PlayMode::Random,
    }
}

/// 菜单项：名称 + 屏幕矩形 `(left, top, right, bottom)`。
type MenuItem = (String, (i32, i32, i32, i32));

/// 在弹出菜单里挑「下一档」的索引。优先按循环顺序精确找；客户端改版缺档时
/// 退回按菜单视觉顺序（从上到下）取当前项的下一项，保证始终有反应。
pub(super) fn pick_next_menu_item(items: &[MenuItem], current: PlayMode) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let target = next_mode(current);
    if let Some(index) = items.iter().position(|(name, _)| mode_from_button(name) == Some(target)) {
        return Some(index);
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| items[i].1 .1);
    let position = order
        .iter()
        .position(|&i| mode_from_button(&items[i].0) == Some(current))
        .unwrap_or(0);
    Some(order[(position + 1) % order.len()])
}

/// QQ 音乐单独一份状态，避免与网易云的 `PLAY_MODE` 互相串值。
static QQ_MODE: AtomicU8 = AtomicU8::new(0);
static LAST_READ_MS: AtomicU64 = AtomicU64::new(0);
static READING: AtomicBool = AtomicBool::new(false);
static CYCLING: AtomicBool = AtomicBool::new(false);
/// 上次找到按钮的顶层窗口，正常路径只读一个窗口，避免每次遍历 40+ 个顶层窗口。
static CACHED_HWND: AtomicIsize = AtomicIsize::new(0);

pub(super) fn current() -> PlayMode {
    PlayMode::from_u8(QQ_MODE.load(Ordering::Relaxed))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn log_transition(previous: PlayMode, mode: PlayMode) {
    if previous == mode || cfg!(test) {
        return;
    }
    let line = format!("QQ音乐播放模式 {} → {}\n", previous.label(), mode.label());
    crate::resources::append_log(
        &crate::model::config_dir().join("diagnostics"),
        "playmode.log",
        &line,
    );
}

/// 由轮询线程调用：1.2 秒节流，读取放到独立线程，绝不阻塞 250ms 的媒体轮询。
/// QQ 音乐收在托盘时 UIA 树依然完整，不需要为了读模式去显示窗口。
#[cfg(windows)]
pub(super) fn poll() {
    let now = now_ms();
    if now.saturating_sub(LAST_READ_MS.load(Ordering::Relaxed)) < 1200 {
        return;
    }
    // 先查 CYCLING 再占 READING：写成 `READING.swap(..) || CYCLING.load(..)` 时，
    // 短路会让 READING 停在 true 且没有配对的释放，之后轮询再也不读模式。
    if CYCLING.load(Ordering::Acquire) || READING.swap(true, Ordering::AcqRel) {
        return;
    }
    LAST_READ_MS.store(now, Ordering::Relaxed);
    std::thread::spawn(|| {
        if let Some(mode) = read_mode() {
            let previous = current();
            log_transition(previous, mode);
            QQ_MODE.store(mode as u8, Ordering::Relaxed);
        }
        READING.store(false, Ordering::Release);
    });
}

#[cfg(not(windows))]
pub(super) fn poll() {}

/// 点岛上模式按钮：走真实鼠标点击，把 QQ 音乐切到下一档模式。
/// 由 `Cmd::CyclePlayMode` 的后台线程调用，内部含约 1 秒的等待，不阻塞 UI。
#[cfg(windows)]
pub(super) fn cycle() -> bool {
    if CYCLING.swap(true, Ordering::AcqRel) {
        return false;
    }
    let result = cycle_impl();
    CYCLING.store(false, Ordering::Release);
    match result {
        Some(mode) => {
            let previous = current();
            log_transition(previous, mode);
            QQ_MODE.store(mode as u8, Ordering::Relaxed);
            // 刚点完立刻刷新，别让节流期间的旧值把图标闪回去。
            LAST_READ_MS.store(now_ms(), Ordering::Relaxed);
            true
        }
        // 失败细节由 cycle_impl 记进 playmode.log，这里不重复写一行。
        None => false
    }
}

#[cfg(not(windows))]
pub(super) fn cycle() -> bool {
    false
}

#[cfg(windows)]
unsafe extern "system" fn collect_windows(
    hwnd: windows::Win32::Foundation::HWND,
    data: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::BOOL {
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
    let mut class = [0u16; 64];
    let n = GetClassNameW(hwnd, &mut class);
    if String::from_utf16_lossy(&class[..n.max(0) as usize]) == "TXGuiFoundation" {
        let out = &mut *(data.0 as *mut Vec<windows::Win32::Foundation::HWND>);
        out.push(hwnd);
    }
    windows::Win32::Foundation::BOOL(1)
}

/// 在顶层窗口里找底栏模式按钮（只认 Button=50000 且名字是已知模式）。
/// 返回按钮当前名称与屏幕矩形；托盘隐藏态下矩形是负坐标，显示后会重新取。
#[cfg(windows)]
fn mode_button(
    automation: &windows::Win32::UI::Accessibility::IUIAutomation,
    condition: &windows::Win32::UI::Accessibility::IUIAutomationCondition,
    hwnd: windows::Win32::Foundation::HWND,
) -> Option<(String, windows::Win32::Foundation::RECT)> {
    use windows::Win32::UI::Accessibility::TreeScope_Descendants;
    unsafe {
        let root = automation.ElementFromHandle(hwnd).ok()?;
        let elements = root.FindAll(TreeScope_Descendants, condition).ok()?;
        for i in 0..elements.Length().unwrap_or(0) {
            let Ok(element) = elements.GetElement(i) else { continue };
            if element.CurrentControlType().map(|v| v.0).unwrap_or(0) != 50000 {
                continue;
            }
            let name = element.CurrentName().map(|v| v.to_string()).unwrap_or_default();
            if mode_from_button(&name).is_none() {
                continue;
            }
            let rect = element.CurrentBoundingRectangle().unwrap_or_default();
            if rect.right <= rect.left || rect.bottom <= rect.top {
                continue;
            }
            return Some((name, rect));
        }
    }
    None
}

/// 枚举 QQ 音乐所有主窗口类名的顶层窗口（含弹出的模式菜单窗）。
#[cfg(windows)]
fn all_qq_windows() -> Vec<windows::Win32::Foundation::HWND> {
    use windows::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::EnumWindows,
    };
    let mut out: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect_windows), LPARAM(&mut out as *mut Vec<HWND> as isize));
    }
    out
}

/// 收集弹出菜单里的模式项：排除底栏按钮本身、离屏元素与无效矩形，并按矩形去重。
/// 菜单浮层同时挂在主窗口和独立弹出窗的 UIA 树上，所以要把所有窗口都翻一遍。
#[cfg(windows)]
fn mode_menu_items(
    automation: &windows::Win32::UI::Accessibility::IUIAutomation,
    condition: &windows::Win32::UI::Accessibility::IUIAutomationCondition,
    windows: &[windows::Win32::Foundation::HWND],
) -> Vec<MenuItem> {
    use windows::Win32::UI::Accessibility::TreeScope_Descendants;
    let mut out: Vec<MenuItem> = Vec::new();
    unsafe {
        for hwnd in windows {
            let Ok(root) = automation.ElementFromHandle(*hwnd) else { continue };
            let Ok(elements) = root.FindAll(TreeScope_Descendants, condition) else { continue };
            for i in 0..elements.Length().unwrap_or(0) {
                let Ok(element) = elements.GetElement(i) else { continue };
                // 菜单项是文本（ct=50020），底栏模式按钮是 Button（ct=50000），不能混。
                if element.CurrentControlType().map(|v| v.0).unwrap_or(0) == 50000 {
                    continue;
                }
                if element.CurrentIsOffscreen().map(|v| v.as_bool()).unwrap_or(true) {
                    continue;
                }
                let name = element.CurrentName().map(|v| v.to_string()).unwrap_or_default();
                if mode_from_button(&name).is_none() {
                    continue;
                }
                let r = element.CurrentBoundingRectangle().unwrap_or_default();
                if r.right <= r.left || r.bottom <= r.top || r.left < 0 || r.top < 0 {
                    continue;
                }
                let item: MenuItem = (name, (r.left, r.top, r.right, r.bottom));
                if !out.contains(&item) {
                    out.push(item);
                }
            }
        }
    }
    out
}

#[cfg(windows)]
fn find_mode_window(
    automation: &windows::Win32::UI::Accessibility::IUIAutomation,
    condition: &windows::Win32::UI::Accessibility::IUIAutomationCondition,
) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::EnumWindows,
    };
    let cached_raw = CACHED_HWND.load(Ordering::Relaxed);
    if cached_raw != 0 {
        let cached = HWND(cached_raw as *mut core::ffi::c_void);
        if mode_button(automation, condition, cached).is_some() {
            return Some(cached);
        }
        CACHED_HWND.store(0, Ordering::Relaxed);
    }
    let mut windows: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect_windows), LPARAM(&mut windows as *mut Vec<HWND> as isize));
    }
    for hwnd in windows {
        if mode_button(automation, condition, hwnd).is_some() {
            CACHED_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
            return Some(hwnd);
        }
    }
    None
}

#[cfg(windows)]
unsafe fn send_key(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };
    let size = std::mem::size_of::<INPUT>() as i32;
    let down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, ..Default::default() } },
    };
    let up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: KEYEVENTF_KEYUP, ..Default::default() } },
    };
    SendInput(&[down, up], size);
}

/// 抢前台：AttachThreadInput 到当前前台窗口的线程，再用一次 Alt 键解锁
/// Windows 的前台锁（`SetForegroundWindow` 从后台线程直接调会被静默拒绝）。
#[cfg(windows)]
unsafe fn grab_foreground(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_MENU;
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };
    let foreground = GetForegroundWindow();
    let thread = GetWindowThreadProcessId(foreground, None);
    let current = GetCurrentThreadId();
    if thread != 0 && thread != current {
        let _ = AttachThreadInput(current, thread, true);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = AttachThreadInput(current, thread, false);
    }
    send_key(VK_MENU);
    let _ = SetForegroundWindow(hwnd);
    let _ = BringWindowToTop(hwnd);
}

#[cfg(windows)]
unsafe fn click(x: i32, y: i32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;
    let _ = SetCursorPos(x, y);
    std::thread::sleep(std::time::Duration::from_millis(80));
    let size = std::mem::size_of::<INPUT>() as i32;
    let down = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: MOUSEEVENTF_LEFTDOWN, ..Default::default() } },
    };
    let up = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: MOUSEEVENTF_LEFTUP, ..Default::default() } },
    };
    SendInput(&[down], size);
    std::thread::sleep(std::time::Duration::from_millis(50));
    SendInput(&[up], size);
}

/// 无论切换成功与否，都把菜单、光标、置顶与窗口可见性复原。
#[cfg(windows)]
struct Restore {
    hwnd: windows::Win32::Foundation::HWND,
    cursor: windows::Win32::Foundation::POINT,
    was_visible: bool,
    was_iconic: bool,
}

#[cfg(windows)]
impl Drop for Restore {
    fn drop(&mut self) {
        use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
        use windows::Win32::UI::WindowsAndMessaging::{
            SetCursorPos, SetWindowPos, ShowWindow, SW_HIDE, SW_MINIMIZE, SWP_NOMOVE, SWP_NOSIZE,
            HWND_NOTOPMOST,
        };
        unsafe {
            send_key(VK_ESCAPE); // 兜底关掉没点中的菜单
            let _ = SetCursorPos(self.cursor.x, self.cursor.y);
            let _ = SetWindowPos(self.hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            if !self.was_visible {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            } else if self.was_iconic {
                let _ = ShowWindow(self.hwnd, SW_MINIMIZE);
            }
        }
    }
}

/// 显示 → 点底栏按钮 → 点菜单下一档 → 读回。返回切换后的模式。
#[cfg(windows)]
fn cycle_impl() -> Option<PlayMode> {
    use std::time::Duration;
    use windows::Win32::{
        Foundation::{HWND, POINT},
        System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED},
        UI::{
            Accessibility::{CUIAutomation, IUIAutomation},
            Input::KeyboardAndMouse::VK_ESCAPE,
            WindowsAndMessaging::{
                GetCursorPos, GetForegroundWindow, IsIconic, IsWindowVisible, SetWindowPos,
                ShowWindow, SW_RESTORE, SW_SHOW, SWP_NOMOVE, SWP_NOSIZE, HWND_TOPMOST,
            },
        },
    };
    unsafe {
        let apartment = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let mut steps: Vec<String> = Vec::new();
        let result = (|| {
            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL).ok()?;
            let condition = automation.CreateTrueCondition().ok()?;
            let hwnd: HWND = find_mode_window(&automation, &condition)?;

            let mut cursor = POINT::default();
            let _ = GetCursorPos(&mut cursor);
            let was_visible = IsWindowVisible(hwnd).as_bool();
            let was_iconic = IsIconic(hwnd).as_bool();
            let _restore = Restore { hwnd, cursor, was_visible, was_iconic };

            // 托盘态窗口既隐藏又最小化，两种恢复都要走；再临时置顶保证点击落在它上面。
            if was_iconic {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            } else {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            if !IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            grab_foreground(hwnd);
            send_key(VK_ESCAPE); // 清掉上次可能残留的菜单
            // QQ 音乐从托盘恢复后要重排布局，等足它重绘再取按钮坐标。
            std::thread::sleep(Duration::from_millis(500));
            steps.push(format!("窗口 {:?} visible={} iconic={} 前台={}",
                hwnd.0, was_visible, was_iconic, GetForegroundWindow() == hwnd));

            // 显示后重新取矩形：托盘态 UIA 报的是 -32000 附近的逻辑坐标。
            let (current_name, button_rect) = mode_button(&automation, &condition, hwnd)?;
            let from = mode_from_button(&current_name).unwrap_or_else(current);
            steps.push(format!("按钮 {current_name} rect {button_rect:?} 当前 {from:?}"));
            click((button_rect.left + button_rect.right) / 2, (button_rect.top + button_rect.bottom) / 2);
            // 菜单有弹出动画，太早读 UIA 树会一个菜单项都看不到。
            std::thread::sleep(Duration::from_millis(700));

            let mut candidates = vec![hwnd];
            candidates.extend(all_qq_windows());
            candidates.dedup();
            let items = mode_menu_items(&automation, &condition, &candidates);
            steps.push(format!("菜单项 {} 个 {:?}", items.len(), items));
            let index = pick_next_menu_item(&items, from)?;
            let rect = items[index].1;
            steps.push(format!("选中 {} {:?}", items[index].0, rect));
            click((rect.0 + rect.2) / 2, (rect.1 + rect.3) / 2);
            std::thread::sleep(Duration::from_millis(500));

            let (new_name, _) = mode_button(&automation, &condition, hwnd)?;
            steps.push(format!("读回 {new_name}"));
            mode_from_button(&new_name)
        })();
        if apartment {
            CoUninitialize();
        }
        if result.is_none() {
            let detail = format!("QQ音乐播放模式 切换失败: {}\n", steps.join(" | "));
            #[cfg(test)]
            { eprintln!("{detail}"); }
            #[cfg(not(test))]
            { crate::resources::append_log(
                &crate::model::config_dir().join("diagnostics"), "playmode.log", &detail); }
        }
        result
    }
}

/// 只读：遍历窗口找模式按钮，把名称映射成播放模式。
#[cfg(windows)]
fn read_mode() -> Option<PlayMode> {
    use windows::Win32::{
        System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED},
        UI::Accessibility::{CUIAutomation, IUIAutomation},
    };
    unsafe {
        // UIA 是 COM，读取线程自己初始化；已初始化时不再配对释放，避免破坏调用方的套间。
        let apartment = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let result = (|| {
            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL).ok()?;
            let condition = automation.CreateTrueCondition().ok()?;
            find_mode_window(&automation, &condition)
                .and_then(|hwnd| mode_button(&automation, &condition, hwnd))
                .and_then(|(name, _)| mode_from_button(&name))
        })();
        if apartment {
            CoUninitialize();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qqmusic_button_names_map_to_play_modes() {
        assert_eq!(mode_from_button("列表循环"), Some(PlayMode::Cycle));
        assert_eq!(mode_from_button("循环播放"), Some(PlayMode::Cycle));
        assert_eq!(mode_from_button("单曲循环"), Some(PlayMode::OneCycle));
        assert_eq!(mode_from_button("随机播放"), Some(PlayMode::Random));
        assert_eq!(mode_from_button("顺序播放"), Some(PlayMode::Order));
        assert_eq!(mode_from_button("心动模式"), Some(PlayMode::Heart));
        assert_eq!(mode_from_button("智能模式"), Some(PlayMode::Heart));
        assert_eq!(mode_from_button(" 列表循环 "), Some(PlayMode::Cycle));
    }

    #[test]
    fn qqmusic_only_recognizes_mode_buttons() {
        // 底栏其它按钮不能被误判；客户端改版出新名字时宁可 Unknown，也不乱画图标。
        for other in ["暂停", "下一曲", "播放队列", "打开歌词", "HQ高品质 切换品质", "主菜单", ""] {
            assert_eq!(mode_from_button(other), None, "{other} 不应被认成播放模式");
        }
    }

    #[test]
    fn every_known_button_name_has_a_mode() {
        for name in MODE_NAMES {
            assert!(mode_from_button(name).is_some(), "{name} 必须有对应模式");
        }
    }

    #[test]
    fn cycle_order_matches_netease_without_heart_mode() {
        // 网易云是 随机→顺序→心动→列表循环→单曲循环→随机；QQ 音乐没有心动档，跳过它。
        assert_eq!(next_mode(PlayMode::Random), PlayMode::Order);
        assert_eq!(next_mode(PlayMode::Order), PlayMode::Cycle);
        assert_eq!(next_mode(PlayMode::Cycle), PlayMode::OneCycle);
        assert_eq!(next_mode(PlayMode::OneCycle), PlayMode::Random);
        assert_eq!(next_mode(PlayMode::Unknown), PlayMode::Random);
        assert_eq!(next_mode(PlayMode::Heart), PlayMode::Cycle);
    }

    #[test]
    fn picks_the_next_mode_from_the_popup_menu() {
        // 实机菜单从上到下的顺序（top 坐标依次递增），与循环顺序不同，必须按模式找而不是按位置。
        let menu: Vec<MenuItem> = vec![
            ("随机播放".into(), (1074, 700, 1144, 715)),
            ("顺序播放".into(), (1074, 741, 1144, 756)),
            ("单曲循环".into(), (1074, 782, 1144, 797)),
            ("列表循环".into(), (1074, 823, 1144, 838)),
        ];
        let name = |i: usize| menu[i].0.as_str();
        assert_eq!(name(pick_next_menu_item(&menu, PlayMode::Cycle).unwrap()), "单曲循环");
        assert_eq!(name(pick_next_menu_item(&menu, PlayMode::OneCycle).unwrap()), "随机播放");
        assert_eq!(name(pick_next_menu_item(&menu, PlayMode::Random).unwrap()), "顺序播放");
        assert_eq!(name(pick_next_menu_item(&menu, PlayMode::Order).unwrap()), "列表循环");
        assert_eq!(pick_next_menu_item(&[], PlayMode::Cycle), None);
    }

    /// 实机冒烟：直接跑岛上点击会执行的 `cycle_impl`，切满一整圈回到初始模式。
    /// 手动运行：`cargo test --lib qqmusic_playmode::tests::live_cycle -- --ignored --exact --nocapture`
    #[test]
    #[cfg(windows)]
    #[ignore = "需要本机 QQ 音乐正在播放；会短暂显示窗口、移动光标，结束后模式与窗口状态自动复原"]
    fn live_cycle_switches_qqmusic_mode() {
        let before = read_mode().expect("QQ 音乐应在运行且能读到模式按钮");
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(cycle_impl().expect("点底栏模式菜单应能切换模式"));
        }
        assert_eq!(seen.first(), Some(&next_mode(before)), "第一下必须切到下一档");
        assert_eq!(seen.last(), Some(&before), "切满一圈应回到初始模式");
        assert_eq!(read_mode(), Some(before), "客户端里也应已复原");
    }

    #[test]
    fn falls_back_to_visual_order_when_client_menu_changes() {
        // 客户端改版少了「单曲循环」时，仍要按菜单顺序切一档，不能点了没反应。
        let menu: Vec<MenuItem> = vec![
            ("随机播放".into(), (1074, 700, 1144, 715)),
            ("列表循环".into(), (1074, 741, 1144, 756)),
        ];
        let name = |i: usize| menu[i].0.as_str();
        assert_eq!(name(pick_next_menu_item(&menu, PlayMode::Random).unwrap()), "列表循环");
        assert_eq!(name(pick_next_menu_item(&menu, PlayMode::Cycle).unwrap()), "随机播放");
        // 完全陌生的当前模式也至少给第一项，而不是返回 None。
        assert!(pick_next_menu_item(&menu, PlayMode::Heart).is_some());
    }
}
