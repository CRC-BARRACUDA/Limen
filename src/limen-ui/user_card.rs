//! The user card: a Steam-style preview beside a user's avatar on hover, as the
//! platform's UI kit draws it (`UserChip`'s card).
//!
//! It opens 350ms after the pointer comes to rest on the chip, where the
//! pointer is — to its right, or to its left when there is no room — and
//! closes 150ms after the pointer has left both the chip and the card, sliding
//! back towards the user it belongs to. Coming back while it leaves keeps it.
//! Escape closes it.

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

use crate::avatar::{self, AvatarLook};

const OPEN_S: f64 = 0.35;
const CLOSE_S: f64 = 0.15;
const MOVE_S: f64 = 0.14;
const GAP: f32 = 12.0;
const MAX_W: f32 = 460.0;
/// The cut corners' size (the kit's clip-path, 20px).
const CUT: f32 = 20.0;

/// What the card says about the user.
pub struct CardInfo<'a> {
    pub look: &'a AvatarLook,
    pub name: &'a str,
    /// Without the `@`.
    pub username: &'a str,
    /// Extra lines, shown as given: an email, a role.
    pub details: &'a [String],
    /// A short tag: "You" on your own card.
    pub tag: Option<&'a str>,
    /// A link at the foot: (text, URL), opened in the browser.
    pub link: Option<(&'a str, &'a str)>,
    /// A sign-out at the very foot, with this text: your own card only.
    pub sign_out: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Idle,
    /// The pointer rests on the chip since `since`.
    Waiting { since: f64 },
    /// Shown since `since`; the pointer left both at `left` (None: still over).
    Open { since: f64, left: Option<f64> },
    /// Going since `since`.
    Leaving { since: f64 },
}

#[derive(Clone, Copy, Debug)]
struct CardState {
    phase: Phase,
    /// Where the pointer was as it opened.
    origin: Pos2,
    /// The card's size last frame, to place it before it is drawn.
    size: Vec2,
    /// On screen last frame, to know whether the pointer is over it.
    rect: Rect,
}

impl Default for CardState {
    fn default() -> Self {
        Self { phase: Phase::Idle, origin: Pos2::ZERO, size: Vec2::new(320.0, 200.0), rect: Rect::NOTHING }
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(2)
}

/// The card for the chip `anchor`. Call every frame after drawing the chip.
/// True when its sign-out was clicked.
pub fn hover_card(ui: &egui::Ui, id: egui::Id, anchor: &egui::Response, info: &CardInfo) -> bool {
    let ctx = ui.ctx().clone();
    let now = ui.input(|i| i.time);
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let mut st: CardState = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let over_chip = anchor.hovered();
    let over_card = pointer.is_some_and(|p| st.rect.contains(p));
    let over = over_chip || over_card;
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    // Until it opens, the card belongs where the hand is now, not where it
    // entered the chip.
    if matches!(st.phase, Phase::Idle | Phase::Waiting { .. })
        && over_chip
        && let Some(p) = pointer
    {
        st.origin = p;
    }

    st.phase = match st.phase {
        Phase::Idle if over_chip => Phase::Waiting { since: now },
        Phase::Idle => Phase::Idle,
        Phase::Waiting { .. } if !over_chip => Phase::Idle,
        Phase::Waiting { since } if now - since >= OPEN_S => Phase::Open { since: now, left: None },
        Phase::Waiting { since } => Phase::Waiting { since },
        Phase::Open { .. } if escape => Phase::Leaving { since: now },
        Phase::Open { since, .. } if over => Phase::Open { since, left: None },
        Phase::Open { since, left: None } => Phase::Open { since, left: Some(now) },
        Phase::Open { left: Some(at), .. } if now - at >= CLOSE_S => Phase::Leaving { since: now },
        Phase::Open { since, left } => Phase::Open { since, left },
        // Coming back while it leaves cancels the exit: the card stays.
        Phase::Leaving { .. } if over => Phase::Open { since: now - MOVE_S, left: None },
        Phase::Leaving { since } if now - since >= MOVE_S => Phase::Idle,
        Phase::Leaving { since } => Phase::Leaving { since },
    };
    // How far in: 0 hidden, 1 shown — and the motion.
    let animate = crate::animations_enabled();
    let shown = match st.phase {
        Phase::Idle | Phase::Waiting { .. } => None,
        Phase::Open { since, .. } => Some(if animate { ease_out(((now - since) / MOVE_S).min(1.0) as f32) } else { 1.0 }),
        Phase::Leaving { since } => {
            let t = ((now - since) / MOVE_S).min(1.0) as f32;
            Some(if animate { 1.0 - t * t } else { 0.0 })
        }
    };
    if !matches!(st.phase, Phase::Idle) {
        ctx.request_repaint();
    }
    let Some(t) = shown else {
        st.rect = Rect::NOTHING;
        ctx.data_mut(|d| d.insert_temp(id, st));
        return false;
    };

    // Beside the pointer, to the right or, without room, to the left; just
    // above it, and never off an edge.
    let screen = ctx.screen_rect();
    let (w, h) = (st.size.x, st.size.y);
    let right = st.origin.x + GAP + w <= screen.right() - 8.0;
    let left_x = if right {
        st.origin.x + GAP
    } else {
        (st.origin.x - GAP - w).min(screen.right() - w - 8.0).max(8.0)
    };
    let top = (st.origin.y - 24.0).min((screen.bottom() - h - 8.0).max(8.0)).max(8.0);
    let slide = (1.0 - t) * 6.0 * if right { -1.0 } else { 1.0 };
    let pos = Pos2::new(left_x + slide, top);

    let area = egui::Area::new(id.with("card"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .interactable(true)
        .show(&ctx, |ui| {
            ui.set_opacity(t);
            card_body(ui, id, info)
        });
    st.size = area.response.rect.size();
    st.rect = area.response.rect;
    let signed_out = area.inner;
    if signed_out {
        // Gone at once: the account it was about is gone.
        st.phase = Phase::Idle;
        st.rect = Rect::NOTHING;
    }
    ctx.data_mut(|d| d.insert_temp(id, st));
    signed_out
}

fn card_body(ui: &mut egui::Ui, id: egui::Id, info: &CardInfo) -> bool {
    let mut signed_out = false;
    let bg = ui.painter().add(egui::Shape::Noop);
    let margin = 20.0;
    let inner = egui::Frame::none().inner_margin(egui::Margin::same(margin)).show(ui, |ui| {
        ui.set_max_width(MAX_W - margin * 2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            avatar::avatar(ui, id.with("card_avatar"), info.look, 72.0, true, None);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(egui::RichText::new(info.name).size(22.0).strong().color(Color32::WHITE));
                if !info.username.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("@{}", info.username))
                            .monospace()
                            .size(15.0)
                            .color(Color32::from_rgba_unmultiplied(200, 120, 48, 140)),
                    );
                }
                if let Some(tag) = info.tag {
                    tag_box(ui, tag);
                }
            });
        });
        if !info.details.is_empty() || info.link.is_some() || info.sign_out.is_some() {
            ui.add_space(14.0);
            let r = ui.available_rect_before_wrap();
            ui.painter().hline(
                r.x_range(),
                r.top(),
                egui::Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(200, 120, 48, 31)),
            );
            ui.add_space(14.0);
            for d in info.details {
                ui.label(egui::RichText::new(d).monospace().size(14.0).color(Color32::from_rgba_unmultiplied(196, 180, 152, 153)));
            }
            if let Some((text, url)) = info.link {
                ui.add_space(4.0);
                let resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("↗ {}", text.to_uppercase()))
                            .monospace()
                            .size(13.0)
                            .color(crate::color::ACCENT_BRIGHT),
                    )
                    .sense(egui::Sense::click()),
                );
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            }
            if let Some(text) = info.sign_out {
                ui.add_space(10.0);
                signed_out = sign_out_row(ui, text).clicked();
            }
        }
    });
    let rect = inner.response.rect;
    ui.painter().set(bg, card_shape(rect));
    signed_out
}

/// The website's sign-out: its mark — a door, an arrow leaving it — and the
/// words, grey, turning red under the pointer.
fn sign_out_row(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let font = egui::FontId::monospace(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_uppercase(), font, Color32::WHITE);
    let icon = 13.0;
    let size = Vec2::new(icon + 8.0 + galley.size().x, galley.size().y.max(icon));
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hover = crate::anim_bool(ui, resp.id.with("hover"), resp.hovered(), 0.15);
    let ink = crate::lerp_color(Color32::from_rgb(0x64, 0x74, 0x8b), Color32::from_rgb(0xf8, 0x71, 0x71), hover);
    let p = ui.painter();
    let r = Rect::from_center_size(Pos2::new(rect.left() + icon / 2.0, rect.center().y), Vec2::splat(icon));
    let s = egui::Stroke::new(1.5_f32, ink);
    let (w, mid) = (r.width(), r.center().y);
    p.line_segment([Pos2::new(r.left() + w * 0.42, r.top()), r.left_top()], s);
    p.line_segment([r.left_top(), r.left_bottom()], s);
    p.line_segment([r.left_bottom(), Pos2::new(r.left() + w * 0.42, r.bottom())], s);
    p.line_segment([Pos2::new(r.left() + w * 0.3, mid), Pos2::new(r.right(), mid)], s);
    p.line_segment([Pos2::new(r.right(), mid), Pos2::new(r.right() - w * 0.22, mid - w * 0.22)], s);
    p.line_segment([Pos2::new(r.right(), mid), Pos2::new(r.right() - w * 0.22, mid + w * 0.22)], s);
    p.galley(Pos2::new(r.right() + 8.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn tag_box(ui: &mut egui::Ui, tag: &str) {
    let text = egui::RichText::new(tag.to_uppercase()).monospace().size(12.0).color(Color32::from_rgb(0xfb, 0xb9, 0x5e));
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Monospace);
    let size = galley.size() + Vec2::new(16.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(251, 185, 94, 15));
    p.rect_stroke(rect, 0.0, egui::Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(251, 185, 94, 77)));
    p.galley(rect.min + Vec2::new(8.0, 2.0), galley, Color32::WHITE);
}

/// The card's background: cut at the top-right and bottom-left corners, a
/// dark amber gradient, a thin border and a brighter edge on the left, over a
/// soft shadow.
fn card_shape(r: Rect) -> egui::Shape {
    let pts = [
        r.left_top(),
        Pos2::new(r.right() - CUT, r.top()),
        Pos2::new(r.right(), r.top() + CUT),
        r.right_bottom(),
        Pos2::new(r.left() + CUT, r.bottom()),
        Pos2::new(r.left(), r.bottom() - CUT),
    ];
    let shadow = egui::epaint::Shadow {
        offset: Vec2::new(0.0, 16.0),
        blur: 48.0,
        spread: 0.0,
        color: Color32::from_black_alpha(204),
    }
    .as_shape(r, 0.0);
    // 160°: lighter at the top-left, darkest at the bottom-right.
    let (top, bottom) = (Color32::from_rgb(0x14, 0x0d, 0x07), Color32::from_rgb(0x08, 0x06, 0x04));
    let shade = |p: Pos2| {
        let t = (((p.x - r.left()) / r.width().max(1.0)) * 0.34 + ((p.y - r.top()) / r.height().max(1.0)) * 0.94) / 1.28;
        crate::lerp_color(top, bottom, t.clamp(0.0, 1.0))
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.center(), shade(r.center()));
    for p in pts {
        mesh.colored_vertex(p, shade(p));
    }
    for i in 0..pts.len() as u32 {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % pts.len() as u32);
    }
    let border = egui::Shape::closed_line(
        pts.to_vec(),
        egui::Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(200, 120, 48, 77)),
    );
    let edge = egui::Shape::line_segment(
        [r.left_top() + Vec2::new(1.0, 0.0), Pos2::new(r.left() + 1.0, r.bottom() - CUT)],
        egui::Stroke::new(2.0_f32, Color32::from_rgba_unmultiplied(251, 185, 94, 153)),
    );
    egui::Shape::Vec(vec![egui::Shape::from(shadow), egui::Shape::mesh(mesh), border, edge])
}
