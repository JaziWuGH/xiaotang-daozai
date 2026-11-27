//! 画布岛仔沿用宿主的眼睛、表情图集与配色。

use super::{geometry::Clip, CanvasTheme};
use crate::{
    paint::Card,
    pet::{Pet, PetState},
    pet_expressions::{Expression, ExpressionSheets},
    plugin_canvas::{PetMood, PetMotion},
};
use eframe::{
    egui::{self, emath::Rot2, Color32, Pos2, Rect, Vec2},
    epaint::{Mesh, Shape, Tessellator},
};
use std::{sync::Arc, time::Duration};

const FRAME: Duration = Duration::from_nanos(33_333_334);

pub(super) struct Placement {
    pub id: Option<String>,
    pub at: Pos2,
    pub size: f32,
    pub angle: f32,
    pub clip: Clip,
    pub mood: PetMood,
    pub motion: PetMotion,
    pub opacity: f32,
}

#[derive(Default)]
pub(super) struct Runner {
    native: Pet,
    id: Option<String>,
    selected: Option<(PetMood, PetMotion)>,
    previous_draw: Option<f64>,
    expected: f64,
    age: f64,
    expression: Option<Expression>,
    expression_at: f64,
    next_expression: f64,
    sequence: u64,
}

pub(super) struct DrawPets<'a> {
    pub runner: &'a mut Runner,
    pub time: f64,
    pub live: bool,
    pub sheets: Option<Arc<ExpressionSheets>>,
    pub next: Option<Duration>,
}

impl DrawPets<'_> {
    pub fn paint(
        &mut self,
        c: &Card<'_>,
        pet: &Placement,
        offset: Vec2,
        clip: Rect,
        theme: CanvasTheme,
    ) {
        let runner = &mut *self.runner;
        let selected = Some((pet.mood, pet.motion));
        if runner.selected != selected || runner.id != pet.id {
            runner.id.clone_from(&pet.id);
            runner.selected = selected;
            runner.age = 0.0;
            runner.previous_draw = None;
            runner.expression = None;
            runner.next_expression = 0.0;
            runner.native.expressions.clear();
        }
        let animate = self.live && crate::bar_layout::animations_enabled();
        let dt = if animate {
            let dt = runner
                .previous_draw
                .map_or(0.0, |before| self.time - before);
            if (0.0..=runner.expected.max(0.2) + 0.20).contains(&dt) {
                dt
            } else {
                0.0
            }
        } else {
            0.0
        };
        runner.age += dt;
        runner.previous_draw = animate.then_some(self.time);
        runner.native.expressions.set_sheets(self.sheets.clone());

        let mut next = if pet.motion != PetMotion::None && animate {
            Some(FRAME)
        } else {
            None
        };
        let seed = c.ui.make_persistent_id(("canvas-pet", &pet.id)).value();
        let expression = runner.expression(pet.mood, seed, animate, &mut next);
        if let Some((expression, age)) = expression {
            runner.native.expressions.play_at(expression, age);
        } else if runner.native.expressions.current().is_some() {
            runner.native.expressions.clear();
        }
        let state = match pet.mood {
            PetMood::Sleeping => PetState::Sleeping,
            PetMood::Happy => PetState::Happy,
            _ => PetState::Awake,
        };
        let sampled = (runner.age * 30.0).floor() / 30.0;
        runner.native.step(state, sampled, 0.65, [0.0; 2], false);

        let pose = pose(pet.motion, sampled as f32);
        let mut card = c.faded(pet.opacity);
        card.painter.set_clip_rect(clip);
        card.s = pet.size / 38.0;
        card.origin = pet.at + offset + egui::vec2(0.0, -14.5 + pose.lift) * card.s;
        let ctx = card.ui.ctx();
        let layer = card.painter.layer_id();
        let first = crate::skin_paint::shape_count(ctx, layer);
        let w = 38.0 * pose.width;
        let h = 19.0 * pose.height;
        if pose.dust > 0.0 {
            let dust = card.faded(pose.dust);
            for (x, y, r) in [(-14.0, 25.5, 1.45), (-19.0, 27.5, 1.0), (17.0, 26.0, 1.15)] {
                dust.circle(x, y, r, theme.map(crate::theme::DIM));
            }
        }
        card.fill(
            -w * 0.5,
            14.5 - h * 0.5,
            w,
            h,
            h * 0.5,
            theme.map(Color32::from_rgb(15, 15, 20)),
        );
        card.stroke(
            -w * 0.5,
            14.5 - h * 0.5,
            w,
            h,
            h * 0.5,
            1.2,
            theme.token(&crate::plugin_canvas::Token::Accent),
        );
        let face = crate::skin_paint::shape_count(ctx, layer);
        runner
            .native
            .draw_canvas_face(&card, 0.0, theme.palette, theme.map(Color32::WHITE));
        if pet.angle != 0.0 || pet.clip.has_planes() || pose.height != 1.0 {
            let pivot = pet.at + offset;
            let head = pivot + egui::vec2(0.0, pose.lift * card.s);
            let rotation = Rot2::from_angle(pet.angle);
            let pixels = ctx.pixels_per_point();
            ctx.graphics_mut(|layers| {
                let list = layers.entry(layer);
                let mut tessellator =
                    Tessellator::new(pixels, Default::default(), [1; 2], Vec::new());
                for index in first..list.next_idx().0 {
                    list.mutate_shape(egui::layers::ShapeIdx(index), |entry| {
                        let mut mesh = if let Shape::Mesh(mesh) = &entry.shape {
                            mesh.as_ref().clone()
                        } else {
                            let mut mesh = Mesh::default();
                            tessellator.tessellate_shape(entry.shape.clone(), &mut mesh);
                            mesh
                        };
                        for vertex in &mut mesh.vertices {
                            if index >= face {
                                let delta = vertex.pos - head;
                                vertex.pos =
                                    head + egui::vec2(delta.x * pose.width, delta.y * pose.height);
                            }
                            vertex.pos = pivot + rotation * (vertex.pos - pivot);
                        }
                        let _ = pet.clip.mesh(&mut mesh, |point| point - offset, None);
                        entry.shape = Shape::mesh(mesh);
                    });
                }
            });
        }
        if let Some(delay) = next {
            self.next = Some(self.next.map_or(delay, |scheduled| scheduled.min(delay)));
        }
        runner.expected = next.map_or(0.0, |delay| delay.as_secs_f64());
    }
}

impl Runner {
    fn expression(
        &mut self,
        mood: PetMood,
        seed: u64,
        animate: bool,
        next: &mut Option<Duration>,
    ) -> Option<(Expression, f32)> {
        match mood {
            PetMood::Expressions => {
                if self.expression.is_none() || self.age >= self.next_expression {
                    let index = (crate::anim::hash01(seed.wrapping_add(self.sequence))
                        * Expression::ALL.len() as f32) as usize;
                    let proposed = Expression::ALL[index.min(Expression::ALL.len() - 1)];
                    self.expression = Some(if Some(proposed) == self.expression {
                        Expression::ALL[(index + 1) % Expression::ALL.len()]
                    } else {
                        proposed
                    });
                    self.expression_at = self.age;
                    self.sequence = self.sequence.wrapping_add(1);
                    self.next_expression =
                        self.age + 6.0 + 4.0 * f64::from(crate::anim::hash01(seed ^ self.sequence));
                }
                let expression = self.expression?;
                let age = (self.age - self.expression_at) as f32;
                if !animate {
                    return Some((expression, expression.duration() * 0.53));
                }
                if age < expression.duration() {
                    *next = Some(FRAME);
                    Some((expression, age))
                } else {
                    let delay =
                        Duration::from_secs_f64((self.next_expression - self.age).max(0.001));
                    *next = Some(next.map_or(delay, |frame| frame.min(delay)));
                    None
                }
            }
            PetMood::Expression(expression) => {
                let age = self.age as f32;
                if animate && age < expression.duration() {
                    *next = Some(FRAME);
                    Some((expression, age))
                } else {
                    Some((expression, expression.duration() * 0.53))
                }
            }
            _ => None,
        }
    }
}

struct Pose {
    lift: f32,
    width: f32,
    height: f32,
    dust: f32,
}

fn pose(motion: PetMotion, age: f32) -> Pose {
    let mut pose = Pose {
        lift: 0.0,
        width: 1.0,
        height: 1.0,
        dust: 0.0,
    };
    match motion {
        PetMotion::Bob => pose.lift = -2.1 * (age * std::f32::consts::TAU / 1.6).sin(),
        PetMotion::Run => {
            let phase = (age / 0.70).fract();
            if phase < 0.23 {
                let squash = (phase / 0.23) * 0.22;
                pose.width += squash * 0.75;
                pose.height -= squash;
                pose.lift = squash * 7.0;
            } else if phase < 0.74 {
                let jump = (((phase - 0.23) / 0.51) * std::f32::consts::PI).sin();
                pose.lift = -9.0 * jump;
                pose.width -= jump * 0.10;
                pose.height += jump * 0.17;
            } else {
                let decay = 1.0 - (phase - 0.74) / 0.26;
                pose.width += decay * 0.14;
                pose.height -= decay * 0.24;
                pose.lift = decay * 2.0;
                pose.dust = decay * 0.8;
            }
        }
        PetMotion::None => {}
    }
    pose
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_canvas_pet_run_has_charge_jump_and_landing_poses() {
        let charge = pose(PetMotion::Run, 0.13);
        assert!(charge.height < 1.0 && charge.width > 1.0 && charge.lift > 0.0);
        let airborne = pose(PetMotion::Run, 0.33);
        assert!(airborne.height > 1.0 && airborne.lift < -7.0);
        let landing = pose(PetMotion::Run, 0.58);
        assert!(landing.dust > 0.0);
        let stationary = pose(PetMotion::None, 1.0);
        assert_eq!(stationary.lift, 0.0);
        assert_eq!(stationary.dust, 0.0);
    }
}
