use std::sync::OnceLock;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

#[link(name = "ntdll")]
extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}

fn supported_version(major: u32, build: u32) -> bool {
    major > 10 || major == 10 && build >= 19041
}

pub(super) fn supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let mut version = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        let status = unsafe { RtlGetVersion(&mut version) };
        status >= 0 && supported_version(version.dwMajorVersion, version.dwBuildNumber)
    })
}

#[derive(Clone, Copy)]
pub(super) struct Target {
    pub hwnd: HWND,
    pub name: &'static str,
}

pub(super) type Targets = [Option<Target>; 4];

#[derive(Default)]
pub(super) struct Exclusion {
    active: bool,
    error: Option<String>,
}

fn apply_system(target: Target, enabled: bool) -> Result<(), String> {
    let affinity = if enabled {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    unsafe { SetWindowDisplayAffinity(target.hwnd, affinity) }.map_err(|error| {
        let action = if enabled { "启用" } else { "取消" };
        format!("{}无法{action}截图排除（{}）", target.name, error.code())
    })
}

fn clear(
    targets: impl IntoIterator<Item = Target>,
    mut apply: impl FnMut(Target, bool) -> Result<(), String>,
) -> Option<String> {
    let mut first_error = None;
    for target in targets {
        if let Err(error) = apply(target, false) {
            first_error.get_or_insert(error);
        }
    }
    first_error
}

impl Exclusion {
    pub(super) fn set(&mut self, desired: bool, targets: Targets) {
        self.set_with(desired, targets, supported, apply_system);
    }

    #[cfg(feature = "sdk-material")]
    pub(super) fn restore(&mut self, desired: bool, targets: Targets) {
        self.apply_to_targets(desired, targets, supported, apply_system);
    }

    #[cfg(feature = "sdk-material")]
    pub(super) fn ensure(&mut self, targets: Targets) -> Result<bool, String> {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowDisplayAffinity;

        self.ensure_with(targets, supported, |target| {
            let mut affinity = 0;
            unsafe { GetWindowDisplayAffinity(target.hwnd, &mut affinity) }
                .map_err(|error| format!("{}无法读取截图排除（{}）", target.name, error.code()))?;
            Ok(affinity == WDA_EXCLUDEFROMCAPTURE.0)
        }, apply_system)
    }

    #[cfg(feature = "sdk-material")]
    fn ensure_with(
        &mut self,
        targets: Targets,
        system_supported: impl FnOnce() -> bool,
        mut is_applied: impl FnMut(Target) -> Result<bool, String>,
        mut apply: impl FnMut(Target, bool) -> Result<(), String>,
    ) -> Result<bool, String> {
        if let Some(error) = self.take_error() {
            return Err(error);
        }
        if !self.active {
            self.set_with(true, targets, system_supported, apply);
            return self.take_error().map_or(Ok(true), Err);
        }
        let mut repaired = false;
        for target in targets.into_iter().flatten() {
            let result = is_applied(target).and_then(|applied| {
                if !applied {
                    apply(target, true)?;
                    repaired = true;
                }
                Ok(())
            });
            if let Err(error) = result {
                self.rollback(error, targets.into_iter().flatten(), apply);
                return Err(self.take_error().unwrap());
            }
        }
        Ok(repaired)
    }

    fn set_with(
        &mut self,
        desired: bool,
        targets: Targets,
        system_supported: impl FnOnce() -> bool,
        apply: impl FnMut(Target, bool) -> Result<(), String>,
    ) {
        if self.active == desired {
            return;
        }
        self.apply_to_targets(desired, targets, system_supported, apply);
    }

    fn apply_to_targets(
        &mut self,
        desired: bool,
        targets: Targets,
        system_supported: impl FnOnce() -> bool,
        mut apply: impl FnMut(Target, bool) -> Result<(), String>,
    ) {
        self.error = None;
        if !desired {
            self.active = false;
            self.error = clear(targets.into_iter().flatten(), apply);
            return;
        }
        if !system_supported() {
            self.error = Some("隐藏截图和录屏中的岛仔需要 Windows 10 2004 或更新版本".into());
            return;
        }
        for target in targets.into_iter().flatten() {
            if let Err(error) = apply(target, true) {
                self.rollback(error, targets.into_iter().flatten(), apply);
                return;
            }
        }
        self.active = true;
    }

    pub(super) fn add(&mut self, target: Target, existing: Targets) {
        self.add_with(target, existing, apply_system);
    }

    fn add_with(
        &mut self,
        target: Target,
        existing: Targets,
        mut apply: impl FnMut(Target, bool) -> Result<(), String>,
    ) {
        if self.active {
            if let Err(error) = apply(target, true) {
                self.rollback(
                    error,
                    existing
                        .into_iter()
                        .flatten()
                        .chain(std::iter::once(target)),
                    apply,
                );
            }
        }
    }

    pub(super) fn render<T>(
        &mut self,
        targets: Targets,
        new_target: Option<Target>,
        render: impl FnMut() -> windows::core::Result<T>,
    ) -> windows::core::Result<T> {
        self.render_with(targets, new_target, render, apply_system)
    }

    fn render_with<T>(
        &mut self,
        targets: Targets,
        new_target: Option<Target>,
        mut render: impl FnMut() -> windows::core::Result<T>,
        apply: impl FnMut(Target, bool) -> Result<(), String>,
    ) -> windows::core::Result<T> {
        let result = render();
        if let Err(error) = &result {
            if self.active {
                let message = format!("截图排除期间岛仔窗口更新失败（{}）", error.code());
                self.rollback(
                    message,
                    targets.into_iter().flatten().chain(new_target),
                    apply,
                );
                return render();
            }
        }
        result
    }

    fn rollback(
        &mut self,
        error: String,
        targets: impl IntoIterator<Item = Target>,
        apply: impl FnMut(Target, bool) -> Result<(), String>,
    ) {
        self.active = false;
        self.error = Some(match clear(targets, apply) {
            Some(rollback_error) => format!("{error}。{rollback_error}"),
            None => error,
        });
    }

    pub(super) fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: usize) -> Target {
        Target {
            hwnd: HWND(id as *mut _),
            name: "测试窗口",
        }
    }

    #[cfg(feature = "sdk-material")]
    #[test]
    fn material_capture_repairs_and_restores_the_saved_preference() {
        use std::cell::Cell;

        let targets = [Some(target(1)), None, None, None];
        for preference in [false, true] {
            let mut state = Exclusion::default();
            let applied = Cell::new(false);
            let write = |_: Target, enabled| { applied.set(enabled); Ok(()) };
            state.set_with(preference, targets, || true, write);
            assert_eq!(state.ensure_with(targets, || true, |_| Ok(applied.get()), write), Ok(!preference));
            assert!(applied.get());
            applied.set(false);
            assert_eq!(state.ensure_with(targets, || true, |_| Ok(applied.get()), write), Ok(true));
            assert_eq!(state.ensure_with(targets, || true, |_| Ok(applied.get()), write), Ok(false));
            state.rollback("窗口状态改变".into(), targets.into_iter().flatten(), |_, _| Err("清除失败".into()));
            assert!(applied.get());
            state.apply_to_targets(preference, targets, || true, write);
            assert_eq!(applied.get(), preference);
            assert_eq!(state.take_error(), None);
        }
    }

    #[test]
    fn capture_exclusion_requires_windows_10_build_19041_or_newer() {
        assert!(!supported_version(6, 99999));
        assert!(!supported_version(10, 19040));
        assert!(supported_version(10, 19041));
        assert!(supported_version(10, 26100));
        assert!(supported_version(11, 1));
    }

    #[test]
    fn capture_exclusion_only_changes_windows_when_requested() {
        let mut state = Exclusion::default();
        let targets = [Some(target(1)), Some(target(2)), None, None];
        let mut changes = Vec::new();
        let mut apply = |window: Target, enabled| {
            changes.push((window.hwnd.0 as usize, enabled));
            Ok(())
        };
        state.set_with(
            false,
            targets,
            || panic!("默认关闭时无需检查系统"),
            &mut apply,
        );
        state.add_with(target(3), targets, &mut apply);
        state.set_with(true, targets, || true, &mut apply);
        state.set_with(
            true,
            targets,
            || panic!("状态未变时无需检查系统"),
            &mut apply,
        );
        state.add_with(target(3), targets, &mut apply);
        state.set_with(
            false,
            [targets[0], targets[1], Some(target(3)), None],
            || true,
            &mut apply,
        );
        state.set_with(
            false,
            targets,
            || panic!("状态未变时无需检查系统"),
            &mut apply,
        );
        assert_eq!(
            changes,
            [
                (1, true),
                (2, true),
                (3, true),
                (1, false),
                (2, false),
                (3, false)
            ]
        );
        assert!(state.take_error().is_none());
    }

    #[test]
    fn capture_exclusion_never_calls_legacy_windows_affinity() {
        let mut state = Exclusion::default();
        let targets = [Some(target(1)), None, None, None];
        let apply = |_: Target, _| panic!("旧版系统不能应用捕获属性");
        state.set_with(true, targets, || false, apply);
        assert!(state
            .take_error()
            .is_some_and(|error| error.contains("Windows 10 2004")));
        state.add_with(target(2), targets, apply);
        state.set_with(false, targets, || false, apply);
        assert!(state.take_error().is_none());
    }

    #[test]
    fn capture_exclusion_rolls_back_all_windows_if_an_existing_layer_refuses() {
        let mut state = Exclusion::default();
        let targets = [Some(target(1)), Some(target(2)), Some(target(3)), None];
        let mut changes = Vec::new();
        let mut apply = |window: Target, enabled| {
            let id = window.hwnd.0 as usize;
            changes.push((id, enabled));
            if id == 2 && enabled {
                Err("分层窗口失败".into())
            } else {
                Ok(())
            }
        };
        state.set_with(true, targets, || true, &mut apply);
        state.add_with(target(4), targets, &mut apply);
        assert_eq!(
            changes,
            [(1, true), (2, true), (1, false), (2, false), (3, false)]
        );
        assert_eq!(state.take_error().as_deref(), Some("分层窗口失败"));
        assert!(state.take_error().is_none());
        let mut retried = Vec::new();
        state.set_with(
            true,
            targets,
            || true,
            |window, enabled| {
                assert!(enabled);
                retried.push(window.hwnd.0 as usize);
                Ok(())
            },
        );
        assert_eq!(retried, [1, 2, 3]);
    }

    #[test]
    fn capture_exclusion_reports_a_late_layer_failure_and_releases_previous_windows() {
        let mut state = Exclusion::default();
        let targets = [Some(target(1)), Some(target(2)), None, None];
        let mut changes = Vec::new();
        let mut apply = |window: Target, enabled| {
            let id = window.hwnd.0 as usize;
            changes.push((id, enabled));
            match (id, enabled) {
                (3, true) => Err("新底板失败".into()),
                (2, false) => Err("旧分层窗口恢复失败".into()),
                _ => Ok(()),
            }
        };
        state.set_with(true, targets, || true, &mut apply);
        state.add_with(target(3), targets, &mut apply);
        state.add_with(target(4), targets, &mut apply);
        assert_eq!(
            changes,
            [
                (1, true),
                (2, true),
                (3, true),
                (1, false),
                (2, false),
                (3, false)
            ]
        );
        let error = state.take_error().unwrap();
        assert!(error.contains("新底板失败"));
        assert!(error.contains("旧分层窗口恢复失败"));
    }

    #[test]
    fn capture_exclusion_retries_a_render_failure_once_after_clearing_new_and_existing_windows() {
        use windows::Win32::Foundation::E_FAIL;
        let mut state = Exclusion::default();
        let targets = [Some(target(1)), None, None, None];
        let changes = std::cell::RefCell::new(Vec::new());
        state.set_with(true, targets, || true, |_, _| Ok(()));
        state.add_with(target(2), targets, |_, _| Ok(()));
        let result = state.render_with(
            targets,
            Some(target(2)),
            || {
                let mut changes = changes.borrow_mut();
                changes.push(0);
                if changes.len() == 1 {
                    Err(E_FAIL.into())
                } else {
                    Ok(())
                }
            },
            |window, enabled| {
                assert!(!enabled);
                changes.borrow_mut().push(window.hwnd.0 as usize);
                Ok(())
            },
        );
        assert!(result.is_ok());
        assert_eq!(changes.into_inner(), [0, 1, 2, 0]);
        assert!(state
            .take_error()
            .is_some_and(|error| error.contains("窗口更新失败")));
        let mut attempts = 0;
        assert!(state
            .render_with(
                targets,
                None,
                || {
                    attempts += 1;
                    Err::<(), _>(E_FAIL.into())
                },
                |_, _| panic!("排除已清除")
            )
            .is_err());
        assert_eq!(attempts, 1);
    }
}
