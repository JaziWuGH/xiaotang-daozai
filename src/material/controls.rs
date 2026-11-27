//! 按实际绘制位置登记主岛控件，将交互和弹簧状态打包为材质输入。

use crate::anim::Spring;
use eframe::egui::{Context, Id, IdMap, Pos2, Rect, Vec2};
use std::cell::RefCell;

const MAX_CONTROLS: usize = 16;
const PRESS_SCALE: f32 = 0.14;

/// `Frame.ctl_spring.w` 中的角色编号。新角色只在末尾追加。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlRole {
    Unclassified = 0,
    Gear = 1,
    Lyric = 2,
    Prev = 3,
    Play = 4,
    Next = 5,
    Volume = 6,
    IslandLyrics = 7,
    NavL = 8,
    NavR = 9,
    Seek = 10,
    Focus = 11,
    Rest = 12,
    QuotaAdd = 13,
    CalAdd = 14,
    CalAllDay = 15,
    CalReminder = 16,
    CalDelete = 17,
    CalCancel = 18,
    CalSave = 19,
    MailSetup = 20,
}

impl ControlRole {
    fn priority(self) -> bool {
        matches!(
            self,
            Self::Gear
                | Self::Prev
                | Self::Play
                | Self::Next
                | Self::Volume
                | Self::NavL
                | Self::NavR
                | Self::Seek
        )
    }
}

#[derive(Clone, Copy)]
pub(super) struct Registration {
    id: Id,
    rect: Rect,
    radius: f32,
    role: ControlRole,
}

thread_local! {
    static REGISTRATIONS: RefCell<Option<Vec<Registration>>> = const { RefCell::new(None) };
}

/// 在主岛绘制时登记控件。无材质或在其他窗口绘制时没有活动收集器。
pub(crate) fn control(id: Id, rect: Rect, radius: f32, role: ControlRole) {
    REGISTRATIONS.with_borrow_mut(|registrations| {
        let Some(registrations) = registrations.as_mut() else {
            return;
        };
        if !rect.is_finite() || !rect.is_positive() {
            return;
        }
        let entry = Registration {
            id,
            rect,
            radius,
            role,
        };
        if let Some(existing) = registrations.iter_mut().find(|entry| entry.id == id) {
            *existing = entry;
        } else {
            registrations.push(entry);
        }
    });
}

pub(super) fn collect<R>(draw: impl FnOnce() -> R) -> (R, Vec<Registration>) {
    struct Restore(Option<Vec<Registration>>);

    impl Drop for Restore {
        fn drop(&mut self) {
            REGISTRATIONS.with_borrow_mut(|registrations| *registrations = self.0.take());
        }
    }

    let _restore = Restore(REGISTRATIONS.with_borrow_mut(|slot| slot.replace(Vec::new())));
    let output = draw();
    let registrations = REGISTRATIONS.with_borrow_mut(|slot| slot.take().unwrap_or_default());
    (output, registrations)
}

#[derive(Default)]
pub(super) struct ControlFrame {
    pub(super) count: u32,
    pub(super) rects: [[f32; 4]; MAX_CONTROLS],
    pub(super) shapes: [[f32; 4]; MAX_CONTROLS],
    pub(super) springs: [[f32; 4]; MAX_CONTROLS],
    pub(super) animating: bool,
}

#[derive(Clone, Default)]
struct ControlScales {
    pass: u64,
    entries: [Option<(Id, f32)>; MAX_CONTROLS],
}

/// 读取最近一帧的缩放，让前景图标跟随同一个弹簧。
pub(crate) fn control_scale(ctx: &Context, id: Id) -> Option<f32> {
    let pass = ctx.cumulative_pass_nr();
    let scales =
        ctx.data(|data| data.get_temp::<ControlScales>(Id::new("material-control-scales")))?;
    if pass.saturating_sub(scales.pass) > 1 {
        return None;
    }
    scales
        .entries
        .into_iter()
        .flatten()
        .find_map(|(entry_id, scale)| (entry_id == id).then_some(scale))
}

struct Motion {
    springs: [Spring; 4],
    seek_drag: Option<Spring>,
}

impl Motion {
    fn advance(
        &mut self,
        offset: Vec2,
        hovered: bool,
        pressed: bool,
        dt: f32,
        stiffness: f32,
        damping: f32,
    ) -> bool {
        let targets = [
            offset.x,
            offset.y,
            u8::from(hovered) as f32,
            u8::from(pressed) as f32,
        ];
        let mut animating = false;
        for (spring, target) in self.springs.iter_mut().zip(targets) {
            spring.target = target;
            spring.tune(stiffness, damping);
            spring.step(dt);
            if spring.settled(1.0) {
                spring.snap(target);
            } else {
                animating = true;
            }
        }
        animating
    }
}

#[derive(Default)]
pub(super) struct Controls {
    motions: IdMap<Motion>,
}

impl Controls {
    pub(super) fn frame(
        &mut self,
        ctx: &Context,
        mut registrations: Vec<Registration>,
        ppp: f32,
        capture_origin_px: Pos2,
        dt: f32,
        stiffness: f32,
        damping: f32,
    ) -> ControlFrame {
        let by_area = |a: &Registration, b: &Registration| b.rect.area().total_cmp(&a.rect.area());
        if registrations.len() > MAX_CONTROLS {
            registrations.sort_by(|a, b| {
                b.role
                    .priority()
                    .cmp(&a.role.priority())
                    .then_with(|| by_area(a, b))
            });
            registrations.truncate(MAX_CONTROLS);
        }
        // 小控件最后交给着色器，重叠时位于较大的底板上层。
        registrations.sort_by(by_area);
        self.motions
            .retain(|id, _| registrations.iter().any(|entry| entry.id == *id));
        let mut frame = ControlFrame::default();
        let mut scales = ControlScales {
            pass: ctx.cumulative_pass_nr(),
            ..Default::default()
        };
        let stiffness_factor = stiffness.clamp(0.2, 3.0);
        let stiffness = 300.0 * stiffness_factor;
        let damping = damping.clamp(0.1, 1.0);

        for entry in registrations {
            let Some(response) = ctx.read_response(entry.id) else {
                continue;
            };
            let pressed = response.enabled() && response.is_pointer_button_down_on();
            let hovered = response.enabled() && response.hovered();
            let offset = response
                .interact_pointer_pos()
                .filter(|_| pressed && entry.role != ControlRole::Seek)
                .map_or(Vec2::ZERO, |pointer| {
                    ((pointer - entry.rect.center()) * 0.14)
                        .clamp(Vec2::splat(-3.0), Vec2::splat(3.0))
                });
            let motion = self.motions.entry(entry.id).or_insert_with(|| Motion {
                springs: [Spring::new(0.0, stiffness, damping); 4],
                seek_drag: None,
            });
            frame.animating |= motion.advance(offset, hovered, pressed, dt, stiffness, damping);
            let [x, y, hover, press] = motion.springs.map(|spring| spring.pos);
            let (drag_scale, press_state, press_scale) = if entry.role == ControlRole::Seek {
                let drag_stiffness = 1000.0 * stiffness_factor;
                let drag = motion
                    .seek_drag
                    .get_or_insert_with(|| Spring::new(0.0, drag_stiffness, 1.0));
                drag.tune(drag_stiffness, 1.0);
                drag.target = u8::from(
                    response.enabled()
                        && (pressed || response.clicked() || response.drag_stopped()),
                ) as f32;
                drag.step(dt);
                if drag.settled(1.0) {
                    drag.snap(drag.target);
                } else {
                    frame.animating = true;
                }
                let drag = drag.pos.clamp(0.0, 1.0);
                (1.0 + 0.9 * drag, drag, PRESS_SCALE * 0.5)
            } else {
                (1.0, press.clamp(0.0, 1.0), PRESS_SCALE)
            };
            let index = frame.count as usize;
            let rect = Rect::from_center_size(entry.rect.center(), entry.rect.size() * drag_scale);
            let min = rect.min.to_vec2() * ppp - capture_origin_px.to_vec2();
            let size = rect.size() * ppp;
            frame.rects[index] = [min.x, min.y, size.x, size.y];
            frame.shapes[index] = [
                entry
                    .radius
                    .clamp(0.0, entry.rect.width().min(entry.rect.height()) * 0.5)
                    * ppp
                    * drag_scale,
                u8::from(response.enabled()) as f32,
                hover.clamp(0.0, 1.0),
                press_state,
            ];
            frame.springs[index] = [
                x * ppp,
                y * ppp,
                1.0 + press_scale * stiffness_factor * press.clamp(0.0, 1.3),
                entry.role as u8 as f32,
            ];
            scales.entries[index] = Some((entry.id, frame.springs[index][2]));
            frame.count += 1;
        }
        ctx.data_mut(|data| {
            *data.get_temp_mut_or_default(Id::new("material-control-scales")) = scales
        });

        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2, CentralPanel, RawInput, Sense};

    #[test]
    fn registration_is_scoped_and_reentrant() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(16.0));
        control(Id::new("outside"), rect, 2.0, ControlRole::Unclassified);
        let (_, outer) = collect(|| {
            control(Id::new("outer"), rect, 2.0, ControlRole::Gear);
            let (_, inner) = collect(|| control(Id::new("inner"), rect, 3.0, ControlRole::Lyric));
            assert_eq!(inner.len(), 1);
            assert_eq!(inner[0].id, Id::new("inner"));
            control(Id::new("outer"), rect, 4.0, ControlRole::Play);
        });
        assert_eq!(outer.len(), 1);
        assert_eq!(outer[0].id, Id::new("outer"));
        assert_eq!(outer[0].radius, 4.0);
        assert!(outer[0].role == ControlRole::Play);
        assert!(collect(|| {}).1.is_empty());
    }

    #[test]
    fn keeps_largest_controls_and_uses_physical_capture_origin() {
        let ctx = Context::default();
        let mut controls = Controls::default();
        let _ = ctx.run(RawInput::default(), |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                let (_, registrations) = collect(|| {
                    for width in 1..=20 {
                        let rect = Rect::from_min_size(pos2(20.0, 10.0), vec2(width as f32, 8.0));
                        let actual = ui.interact(rect, Id::new(width), Sense::click());
                        control(actual.id, rect, 3.0, ControlRole::Unclassified);
                    }
                });
                let frame =
                    controls.frame(ctx, registrations, 1.25, pos2(12.0, 4.0), 0.016, 1.0, 0.5);
                assert_eq!(frame.count, 16);
                assert_eq!(frame.rects[0], [13.0, 8.5, 25.0, 10.0]);
                assert_eq!(frame.rects[15][2], 6.25);
                assert_eq!(frame.shapes[0][0..2], [3.75, 1.0]);
                assert_eq!(frame.springs[0], [0.0, 0.0, 1.0, 0.0]);
            });
        });
    }

    #[test]
    fn retains_priority_roles_before_larger_ordinary_controls() {
        let ctx = Context::default();
        let mut controls = Controls::default();
        let important = [
            ControlRole::Gear,
            ControlRole::Prev,
            ControlRole::Play,
            ControlRole::Next,
            ControlRole::Volume,
            ControlRole::NavL,
            ControlRole::NavR,
            ControlRole::Seek,
        ];
        let _ = ctx.run(RawInput::default(), |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                let (_, registrations) = collect(|| {
                    for index in 0..24 {
                        let width = index as f32 + 1.0;
                        let rect = Rect::from_min_size(pos2(20.0, 10.0), vec2(width, 8.0));
                        let response = ui.interact(rect, Id::new(index), Sense::click());
                        let role = important
                            .get(index)
                            .copied()
                            .unwrap_or(ControlRole::Unclassified);
                        control(response.id, rect, 2.0, role);
                    }
                });
                let frame = controls.frame(ctx, registrations, 1.0, Pos2::ZERO, 0.016, 1.0, 0.5);
                assert_eq!(frame.count, 16);
                assert_eq!(
                    frame.rects.map(|rect| rect[2]),
                    [24., 23., 22., 21., 20., 19., 18., 17., 8., 7., 6., 5., 4., 3., 2., 1.]
                );
                assert_eq!(
                    frame.springs.map(|spring| spring[3]),
                    [0., 0., 0., 0., 0., 0., 0., 0., 10., 9., 8., 6., 5., 4., 3., 1.]
                );
                assert_eq!(control_scale(ctx, Id::new(7usize)), Some(1.0));
                assert_eq!(control_scale(ctx, Id::new(15usize)), None);
            });
        });
        let _ = ctx.run(RawInput::default(), |_| {});
        let _ = ctx.run(RawInput::default(), |_| {});
        assert_eq!(control_scale(&ctx, Id::new(7usize)), None);
    }

    #[test]
    fn a_steady_hover_stops_the_spring_repaint() {
        let mut motion = Motion {
            springs: [Spring::new(0.0, 300.0, 0.5); 4],
            seek_drag: None,
        };
        let dt = 1.0 / 60.0;
        assert!(motion.advance(vec2(2.0, -1.5), true, true, dt, 300.0, 0.5));
        assert_ne!(motion.springs[0].pos, 0.0);
        for _ in 0..120 {
            motion.advance(Vec2::ZERO, true, false, dt, 300.0, 0.5);
        }
        assert!(!motion.advance(Vec2::ZERO, true, false, dt, 300.0, 0.5));
        assert_eq!(
            motion.springs.map(|spring| spring.pos),
            [0.0, 0.0, 1.0, 0.0]
        );
    }
}
