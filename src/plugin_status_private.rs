//! 状态页敏感值的临时显隐。

use crate::{app::Env, paint::Card};
use eframe::egui::{self, CursorIcon, Id};
use std::{hash::Hash, time::Duration};

pub(crate) const MASKED_VALUE: &str = "••••";

const REVEAL_SECONDS: f64 = 30.0;
type Reveal = (Id, f64);

pub(crate) fn sensitive_values_blocked(exclude_from_capture: bool, env: &Env) -> bool {
    exclude_from_capture || env.screen_locked || env.recording || env.fullscreen_hidden
}

fn reveal_key() -> Id {
    Id::new("plugin-status-revealed-value")
}

pub(crate) fn clear_sensitive_reveal(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Reveal>(reveal_key()));
}

pub(super) struct SensitiveValues {
    plugin: Id,
    blocked: bool,
}

impl SensitiveValues {
    pub(super) fn new(plugin: &str, blocked: bool) -> Self {
        Self {
            plugin: Id::new(("plugin-sensitive", plugin)),
            blocked,
        }
    }

    pub(super) fn revealed(
        &self,
        c: &Card,
        sensitive: bool,
        identity: impl Hash,
        rect: [f32; 4],
    ) -> bool {
        self.reveal(c, sensitive, identity, rect, true).0
    }

    pub(super) fn revealed_for_actions(
        &self,
        c: &Card,
        sensitive: bool,
        identity: impl Hash,
        rect: [f32; 4],
    ) -> (bool, bool) {
        self.reveal(c, sensitive, identity, rect, false)
    }

    fn reveal(
        &self,
        c: &Card,
        sensitive: bool,
        identity: impl Hash,
        [x, y, width, height]: [f32; 4],
        allow_rehide: bool,
    ) -> (bool, bool) {
        if !sensitive {
            return (true, true);
        }
        if self.blocked
            || !c.live
            || !c
                .rect(x, y, width, height)
                .intersects(c.painter.clip_rect())
        {
            return (false, false);
        }
        let id = self.plugin.with(identity);
        let now = c.ui.input(|input| input.time);
        let ctx = c.ui.ctx();
        let current = ctx.data(|data| data.get_temp::<Reveal>(reveal_key()));
        let until =
            current.and_then(|(value, until)| (value == id && now < until).then_some(until));
        if let Some(until) = until {
            ctx.request_repaint_after(Duration::from_secs_f64(until - now));
            // 已展开的动作行把点击交给正文与行尾按钮。
            if !allow_rehide {
                return (true, true);
            }
        }
        let response = c
            .hit(x, y, width, height, id)
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() {
            if until.is_some() {
                clear_sensitive_reveal(ctx);
                return (false, false);
            }
            ctx.data_mut(|data| data.insert_temp(reveal_key(), (id, now + REVEAL_SECONDS)));
            ctx.request_repaint_after(Duration::from_secs_f64(REVEAL_SECONDS));
            if !allow_rehide {
                ctx.request_repaint();
            }
            return (true, false);
        }
        (until.is_some(), until.is_some())
    }
}
