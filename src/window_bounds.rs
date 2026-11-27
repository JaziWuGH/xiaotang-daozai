use std::time::{Duration, Instant};
use windows::Win32::Foundation::RECT;

const SETTLE: Duration = Duration::from_millis(160);

#[derive(Default)]
pub(crate) struct WindowBounds {
    stable: Option<(RECT, Instant)>,
}

impl WindowBounds {
    /// 裁剪框换算到同一画布的物理像素。超出现有外框时一次恢复完整画布。
    pub(crate) fn target(
        &mut self,
        current: RECT,
        tight: RECT,
        full: RECT,
        transitioning: bool,
        now: Instant,
    ) -> (RECT, Option<Duration>) {
        if tight.left >= tight.right || tight.top >= tight.bottom || !contains(full, tight) {
            self.stable = None;
            return (full, None);
        }
        let needs_space = !contains(current, tight);
        if transitioning {
            self.stable = None;
            return (if needs_space { full } else { current }, None);
        }
        if needs_space {
            self.stable = Some((tight, now));
            return (full, Some(SETTLE));
        }
        if current == tight {
            self.stable = None;
            return (current, None);
        }
        let since = match self.stable {
            Some((last, since)) if last == tight => since,
            _ => {
                self.stable = Some((tight, now));
                now
            }
        };
        let wait = SETTLE.saturating_sub(now.saturating_duration_since(since));
        if wait.is_zero() {
            (tight, None)
        } else {
            (current, Some(wait))
        }
    }
}

fn contains(outer: RECT, inner: RECT) -> bool {
    outer.left <= inner.left && outer.top <= inner.top
        && inner.right <= outer.right && inner.bottom <= outer.bottom
}

/// 新旧客户区取屏幕坐标交集，同一矩形作为保留像素的源和目的区域。
pub(crate) fn preserve_overlap(old: RECT, new: RECT) -> Option<RECT> {
    let overlap = RECT {
        left: old.left.max(new.left),
        top: old.top.max(new.top),
        right: old.right.min(new.right),
        bottom: old.bottom.min(new.bottom),
    };
    (overlap.left < overlap.right && overlap.top < overlap.bottom).then_some(overlap)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: RECT = rect(0, 0, 1260, 354);
    const COMPACT: RECT = rect(290, 0, 640, 53);
    const EXPANDED: RECT = rect(166, 0, 765, 245);

    const fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    #[test]
    fn opening_and_closing_keep_the_expanded_outer_frame_through_each_transition() {
        let start = Instant::now();
        let mut bounds = WindowBounds::default();
        assert_eq!(bounds.target(COMPACT, COMPACT, FULL, true, start), (COMPACT, None));
        let early = rect(275, 0, 655, 67);
        assert_eq!(bounds.target(COMPACT, early, FULL, true, start), (FULL, None));
        let opened = start + Duration::from_millis(420);
        assert_eq!(bounds.target(FULL, EXPANDED, FULL, true, opened), (FULL, None));
        assert_eq!(bounds.target(FULL, EXPANDED, FULL, false, opened), (FULL, Some(SETTLE)));
        assert_eq!(bounds.target(FULL, EXPANDED, FULL, false, opened + SETTLE), (EXPANDED, None));

        let closed = opened + Duration::from_millis(700);
        assert_eq!(bounds.target(EXPANDED, early, FULL, true, closed), (EXPANDED, None));
        assert_eq!(bounds.target(EXPANDED, COMPACT, FULL, true, closed), (EXPANDED, None));
        assert_eq!(bounds.target(EXPANDED, COMPACT, FULL, false, closed), (EXPANDED, Some(SETTLE)));
        assert_eq!(bounds.target(EXPANDED, COMPACT, FULL, false, closed + SETTLE), (COMPACT, None));
    }

    #[test]
    fn taller_pages_and_notifications_acquire_the_whole_reserve_on_first_growth() {
        let start = Instant::now();
        let mut bounds = WindowBounds::default();
        let taller_page = rect(166, 0, 765, 273);
        let notice = rect(166, 0, 958, 273);
        assert_eq!(bounds.target(EXPANDED, taller_page, FULL, true, start), (FULL, None));
        assert_eq!(bounds.target(FULL, notice, FULL, true, start), (FULL, None));
        assert_eq!(bounds.target(FULL, notice, FULL, false, start), (FULL, Some(SETTLE)));
        assert_eq!(bounds.target(FULL, notice, FULL, false, start + SETTLE), (notice, None));
    }

    #[test]
    fn continuously_changing_bounds_never_schedule_an_early_shrink() {
        let start = Instant::now();
        let mut bounds = WindowBounds::default();
        for offset in 0..=8 {
            let tight = rect(COMPACT.left - offset, 0, COMPACT.right + offset, COMPACT.bottom);
            let now = start + Duration::from_millis(offset as u64 * 85);
            assert_eq!(bounds.target(FULL, tight, FULL, false, now), (FULL, Some(SETTLE)));
        }
        let latest = rect(COMPACT.left - 8, 0, COMPACT.right + 8, COMPACT.bottom);
        let now = start + Duration::from_millis(8 * 85);
        let before = now + SETTLE - Duration::from_millis(1);
        assert_eq!(bounds.target(FULL, latest, FULL, false, before), (FULL, Some(Duration::from_millis(1))));
        assert_eq!(bounds.target(FULL, latest, FULL, false, now + SETTLE), (latest, None));
    }

    #[test]
    fn a_late_tooltip_resets_the_deadline_and_can_expand_an_already_compact_window() {
        let start = Instant::now();
        let mut bounds = WindowBounds::default();
        let tooltip = rect(166, 0, 891, 251);
        assert_eq!(bounds.target(FULL, EXPANDED, FULL, false, start), (FULL, Some(SETTLE)));
        let late = start + SETTLE - Duration::from_millis(1);
        assert_eq!(bounds.target(FULL, tooltip, FULL, false, late), (FULL, Some(SETTLE)));
        assert_eq!(bounds.target(FULL, tooltip, FULL, false, late + SETTLE), (tooltip, None));
        let later = late + SETTLE + Duration::from_secs(1);
        assert_eq!(bounds.target(EXPANDED, tooltip, FULL, false, later), (FULL, Some(SETTLE)));
        assert_eq!(bounds.target(FULL, tooltip, FULL, false, later + SETTLE), (tooltip, None));
    }

    #[test]
    fn overlap_keeps_the_original_screen_coordinates_even_on_a_negative_monitor() {
        let old = rect(-801, -220, -414, -170);
        let new = rect(-935, -220, -296, -22);
        assert_eq!(preserve_overlap(old, new), Some(old));
        assert_eq!(preserve_overlap(new, old), Some(old));
        assert_eq!(preserve_overlap(old, rect(-900, -200, -600, -160)), Some(rect(-801, -200, -600, -170)));
        assert_eq!(preserve_overlap(old, rect(-414, -220, -310, -170)), None);
        assert_eq!(preserve_overlap(old, rect(-800, -170, -524, -140)), None);
    }

    #[test]
    fn missing_or_degenerate_paint_bounds_keep_the_full_canvas() {
        let mut bounds = WindowBounds::default();
        let now = Instant::now();
        let empty=rect(COMPACT.left,COMPACT.top,COMPACT.left,COMPACT.bottom);
        assert_eq!(bounds.target(COMPACT,empty,FULL,false,now),(FULL,None));
        assert_eq!(bounds.target(FULL,rect(-1,0,50,40),FULL,false,now),(FULL,None));
    }
}
