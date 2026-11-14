//! A user's avatar with its decoration, as the platform's UI kit draws it.
//!
//! * The face: the user's picture, cropped square — or their initials on the
//!   amber tile while there is none.
//! * The frame: a decoration's image over the face, 120% of it, the face in
//!   the middle (frames are authored that way). Animated or not.
//! * The effect: one of the code effects in [`effects`], drawn on a canvas
//!   twice the avatar's size around it, from 40px up; it reacts to the
//!   pointer and to a click when the avatar is interactive.
//!
//! With animations off (Limen's setting, as the web's reduced motion), no
//! effect runs and animated pictures show their first frame.

pub mod canvas;
pub mod effects;
pub mod picture;

use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

use canvas::{Canvas, Face, FaceShape};
pub use effects::EffectKind;
pub use picture::{decode, Decoded, Picture, MAX_SIDE};

/// Below this size the frame is drawn but the effect is not.
pub const EFFECT_MIN_SIZE: f32 = 40.0;
/// A step never advances more than this, so a stalled window doesn't jump.
const MAX_STEP_MS: f64 = 50.0;

/// The decoration's effect, from a checked spec.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectSpec {
    pub kind: EffectKind,
    /// Checked hex colours; empty = the effect's own.
    pub colors: Vec<String>,
    /// 0..1.
    pub intensity: f32,
}

/// Everything an avatar is drawn from.
#[derive(Clone, Default)]
pub struct AvatarLook {
    /// The user's id: keys the effect's state, and its randomness.
    pub user_id: String,
    /// Display name: the initials, and the picture's alt.
    pub name: String,
    pub picture: Option<Arc<Picture>>,
    pub frame: Option<Arc<Picture>>,
    pub effect: Option<EffectSpec>,
}

/// "Ada Lovelace" → "AL"; one word → its first letter; nothing → "?".
pub fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    let letters: Vec<&str> = if parts.len() > 1 { vec![parts[0], parts[parts.len() - 1]] } else { parts };
    let s: String = letters.iter().filter_map(|p| p.chars().next()).collect::<String>().to_uppercase();
    if s.is_empty() { "?".into() } else { s }
}

/// An effect's state between frames.
#[derive(Clone)]
struct FxState {
    /// What it was built for; a change rebuilds it.
    key: String,
    fx: Arc<Mutex<Box<dyn effects::Effect>>>,
    last: Option<f64>,
}

fn seed_of(s: &str) -> u64 {
    // FNV-1a, then the clock: every avatar its own randomness.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    h ^ now
}

/// How often to repaint a running effect: every frame while the window has
/// the focus, about ten a second when it does not, never while it is
/// minimised — the kit stops its loop while the tab is hidden.
fn request_frame(ctx: &egui::Context) {
    let (focused, minimized) =
        ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.viewport().minimized.unwrap_or(false)));
    if minimized {
        return;
    }
    if focused {
        ctx.request_repaint();
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Draw an avatar of `size` px. `interactive`: the effect answers the pointer
/// and a click. `overflow`: a layer to draw the effect on when it must spill
/// past the panel the avatar sits in (the header); None draws it in place.
pub fn avatar(
    ui: &mut egui::Ui,
    id: egui::Id,
    look: &AvatarLook,
    size: f32,
    interactive: bool,
    overflow: Option<egui::Order>,
) -> egui::Response {
    avatar_from(ui, id, look, size, interactive, overflow, EFFECT_MIN_SIZE)
}

/// [`avatar`], with the effect drawn from `effect_floor` px up rather than
/// from [`EFFECT_MIN_SIZE`]: for the one place someone meets their own
/// decoration small — the title bar — where it spills onto a layer of its own
/// and the bar keeps its height.
pub fn avatar_from(
    ui: &mut egui::Ui,
    id: egui::Id,
    look: &AvatarLook,
    size: f32,
    interactive: bool,
    overflow: Option<egui::Order>,
    effect_floor: f32,
) -> egui::Response {
    let sense = if interactive { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), sense);
    if !ui.is_rect_visible(rect.expand(size)) {
        return response;
    }
    let ctx = ui.ctx().clone();
    let animate = crate::animations_enabled();
    let now = ui.input(|i| i.time);
    let shape = look.effect.as_ref().map(|e| e.kind.face_shape()).unwrap_or(FaceShape::Square);
    let painter = ui.painter();

    // The face: the tile, the picture (or initials) inside its 1px border.
    let rounding = match shape {
        FaceShape::Square => 0.0,
        FaceShape::Rounded => size * 0.18,
        FaceShape::Circle => size / 2.0,
    };
    painter.rect_filled(rect, rounding, Color32::from_rgba_unmultiplied(200, 120, 48, 31));
    let inner = rect.shrink(1.0);
    let mut next_ms: Option<f64> = None;
    match &look.picture {
        Some(pic) => {
            let (tex, next) = pic.frame_at(now * 1000.0, animate);
            next_ms = next;
            // object-fit: cover — the middle of the picture, square.
            let a = pic.aspect;
            let uv = if a >= 1.0 {
                let w = 1.0 / a;
                Rect::from_min_max(Pos2::new((1.0 - w) / 2.0, 0.0), Pos2::new((1.0 + w) / 2.0, 1.0))
            } else {
                Rect::from_min_max(Pos2::new(0.0, (1.0 - a) / 2.0), Pos2::new(1.0, (1.0 + a) / 2.0))
            };
            let mut img = egui::Image::new((tex.id(), inner.size())).uv(uv);
            if rounding > 0.0 {
                img = img.rounding(rounding);
            }
            img.paint_at(ui, inner);
        }
        None => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                initials(&look.name),
                egui::FontId::monospace((size * 0.38).max(8.0)),
                crate::color::ACCENT_BRIGHT,
            );
        }
    }
    painter.rect_stroke(rect, rounding, egui::Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(200, 120, 48, 77)));

    // The frame: 120% of the face, centred on it.
    if let Some(frame) = &look.frame {
        let (tex, next) = frame.frame_at(now * 1000.0, animate);
        next_ms = match (next_ms, next) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let fr = rect.expand(size * 0.1);
        let painter = match overflow {
            Some(order) => ctx.layer_painter(egui::LayerId::new(order, id.with("frame"))),
            None => {
                let mut p = ui.painter().clone();
                p.set_clip_rect(ui.clip_rect().union(fr));
                p
            }
        };
        painter.image(tex.id(), fr, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }
    if let Some(ms) = next_ms {
        ctx.request_repaint_after(std::time::Duration::from_millis(ms.max(10.0) as u64));
    }

    // The effect, on a canvas twice the avatar's size around it.
    let Some(spec) = look.effect.as_ref().filter(|_| animate && size >= effect_floor) else {
        return response;
    };
    let env = effects::Env::new(spec.kind, size, &spec.colors, spec.intensity);
    let canvas_rect = Rect::from_center_size(rect.center(), Vec2::splat(env.extent));
    let key = format!("{:?}|{:?}|{}|{}", spec.kind, spec.colors, spec.intensity, size);
    let state_id = id.with("avatar_fx");
    let mut state: FxState = ctx.data_mut(|d| d.get_temp(state_id)).filter(|s: &FxState| s.key == key).unwrap_or_else(|| {
        FxState {
            key: key.clone(),
            fx: Arc::new(Mutex::new(effects::create(spec.kind, &env, seed_of(&look.user_id)))),
            last: None,
        }
    });
    let dt = state.last.map(|l| ((now - l) * 1000.0).clamp(0.0, MAX_STEP_MS)).unwrap_or(0.0) as f32;
    state.last = Some(now);
    // The pointer, in canvas px, while it is over the canvas.
    let pointer = if interactive {
        ui.input(|i| i.pointer.hover_pos())
            .filter(|p| canvas_rect.contains(*p) && ui.rect_contains_pointer(canvas_rect))
            .map(|p| (p - canvas_rect.min).to_pos2())
    } else {
        None
    };
    {
        let mut fx = state.fx.lock().unwrap_or_else(|e| e.into_inner());
        if interactive
            && response.clicked()
            && let Some(at) = ui.input(|i| i.pointer.interact_pos())
        {
            fx.press((at - canvas_rect.min).to_pos2());
        }
        fx.update(dt, pointer);
        let face = match &look.picture {
            Some(p) => Face::Picture { still: p.still.id(), inverted: p.inverted.id() },
            None => Face::Initials(initials(&look.name)),
        };
        let fx_painter = match overflow {
            Some(order) => ctx.layer_painter(egui::LayerId::new(order, id.with("effect"))),
            None => {
                let mut p = ui.painter().clone();
                p.set_clip_rect(ui.clip_rect().union(canvas_rect));
                p
            }
        };
        let pad = (env.extent - size) / 2.0;
        let face_rect = Rect::from_min_size(Pos2::new(pad, pad), Vec2::splat(size));
        fx.draw(&Canvas::new(fx_painter, canvas_rect.min, face, shape, face_rect));
    }
    ctx.data_mut(|d| d.insert_temp(state_id, state));
    request_frame(&ctx);
    response
}
