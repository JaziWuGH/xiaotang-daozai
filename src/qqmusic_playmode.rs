//! QQ 音乐播放模式联动：读取底栏模式按钮的 UIA 名称，让岛上图标与客户端保持一致。
//!
//! 实机排查过所有「反向切换」通道，QQ 音乐一个都不开放：
//! - SMTC 不上报 `AutoRepeatMode` / `IsShuffleActive`，反向设置返回 true 但不生效；
//! - 全局热键原子只有 `Tencent - QQMusicPlayer - 0xB0~B3` 与 Player_Status 等，没有模式命令；
//! - UIA `InvokePattern::Invoke` / `LegacyIAccessible::DoDefaultAction` 都抛 0x80131509，
//!   即使把窗口显示出来（offscreen=false）也一样；
//! - 按客户区坐标投递 WM_LBUTTONDOWN/UP 对自绘 UI 无效（最小化后 UIA 坐标是逻辑布局坐标，
//!   与客户区 160×28 更对不上）；
//! - `QQMusicSvr.QQMusicControl` COM 组件实例化会挂起；配置文件里也没有播放模式字段。
//!
//! 因此这里只做「读」：客户端里改了模式，岛仔在两秒内同步图标。点击岛上按钮不模拟点击、
//! 不移动鼠标、不闪窗口，只把 QQ 音乐主窗口调到前台，用户点一下底栏即可。

use super::PlayMode;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, AtomicU8, Ordering};

/// QQ 音乐底栏模式按钮的 UIA 名称。按钮名就是当前模式的中文名，随切换实时变化。
pub(super) const MODE_NAMES: [&str; 7] = [
    "列表循环", "单曲循环", "随机播放", "顺序播放", "循环播放", "心动模式", "智能模式",
];

/// 按钮名 → 岛仔播放模式。「循环播放」是部分版本对列表循环的叫法；
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

/// QQ 音乐单独一份状态，避免与网易云的 `PLAY_MODE` 互相串值。
static QQ_MODE: AtomicU8 = AtomicU8::new(0);
static LAST_READ_MS: AtomicU64 = AtomicU64::new(0);
static READING: AtomicBool = AtomicBool::new(false);
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

/// 由轮询线程调用：1.2 秒节流，读取放到独立线程，绝不阻塞 250ms 的媒体轮询。
/// QQ 音乐收在托盘时 UIA 树依然完整，不需要为了读模式去显示窗口。
#[cfg(windows)]
pub(super) fn poll() {
    let now = now_ms();
    if now.saturating_sub(LAST_READ_MS.load(Ordering::Relaxed)) < 1200 {
        return;
    }
    if READING.swap(true, Ordering::AcqRel) {
        return;
    }
    LAST_READ_MS.store(now, Ordering::Relaxed);
    std::thread::spawn(|| {
        if let Some(mode) = read_mode() {
            let previous = current();
            if previous != mode && !cfg!(test) {
                // 只在变化时记一行，便于确认岛上确实跟上了客户端的模式切换。
                let line = format!("QQ音乐播放模式 {} → {}\n", previous.label(), mode.label());
                crate::resources::append_log(
                    &crate::model::config_dir().join("diagnostics"),
                    "playmode.log",
                    &line,
                );
            }
            QQ_MODE.store(mode as u8, Ordering::Relaxed);
        }
        READING.store(false, Ordering::Release);
    });
}

#[cfg(not(windows))]
pub(super) fn poll() {}

/// 点击岛上模式按钮：QQ 音乐不接受任何无侵入的切换指令，把主窗口调到前台即可。
#[cfg(windows)]
pub(super) fn reveal() -> bool {
    super::reveal_app("QQMusic.exe")
}

#[cfg(not(windows))]
pub(super) fn reveal() -> bool {
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

/// 在一个顶层窗口里找模式按钮。只认 Button(50000) 且名字是已知模式，
/// 避免把「播放队列」「HQ高品质」等底栏按钮误判成模式。
#[cfg(windows)]
fn mode_in_window(
    automation: &windows::Win32::UI::Accessibility::IUIAutomation,
    condition: &windows::Win32::UI::Accessibility::IUIAutomationCondition,
    hwnd: windows::Win32::Foundation::HWND,
) -> Option<PlayMode> {
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
            if let Some(mode) = mode_from_button(&name) {
                return Some(mode);
            }
        }
    }
    None
}

#[cfg(windows)]
fn read_mode() -> Option<PlayMode> {
    use windows::Win32::{
        Foundation::{HWND, LPARAM},
        System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED},
        UI::{
            Accessibility::{CUIAutomation, IUIAutomation},
            WindowsAndMessaging::EnumWindows,
        },
    };
    unsafe {
        // UIA 是 COM，读取线程自己初始化；已初始化时不再配对释放，避免破坏调用方的套间。
        let apartment = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let result = (|| {
            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL).ok()?;
            let condition = automation.CreateTrueCondition().ok()?;

            // HWND 内层是 *mut c_void，缓存里存它的整数值，取回时再转回指针。
            let cached_raw = CACHED_HWND.load(Ordering::Relaxed);
            if cached_raw != 0 {
                let cached = HWND(cached_raw as *mut core::ffi::c_void);
                if let Some(mode) = mode_in_window(&automation, &condition, cached) {
                    return Some(mode);
                }
                CACHED_HWND.store(0, Ordering::Relaxed);
            }

            let mut windows: Vec<HWND> = Vec::new();
            let _ = EnumWindows(Some(collect_windows), LPARAM(&mut windows as *mut Vec<HWND> as isize));
            for hwnd in windows {
                if let Some(mode) = mode_in_window(&automation, &condition, hwnd) {
                    CACHED_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
                    return Some(mode);
                }
            }
            None
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
}
