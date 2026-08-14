//! Pop-up windows: the veil, the frame, and the dialogs built on them.

use crate::*;

/// What a pop-up window looks like, independent of what is inside it.
pub struct OverlayOpts {
    /// Requested content width in points; clamped to the window.
    pub width: f32,
    /// How tall the content may grow before it scrolls.
    pub max_height: f32,
    /// A title bar with the window controls on the right. `None` leaves the
    /// header to the content — the host's own dialogs centre their titles.
    pub title: Option<String>,
    /// Show the back arrow beside the close control.
    pub back: bool,
    /// Show the close control.
    pub close: bool,
    /// The area this pop-up belongs to — dimmed, blocked, and centred within.
    ///
    /// A tab's content rather than the whole window: the title bar, the tab
    /// strip and every other tab stay usable while it is open, so a pop-up
    /// suspends the thing that raised it and nothing else. `None` falls back to
    /// the whole window.
    pub bounds: Option<egui::Rect>,
}

impl Default for OverlayOpts {
    fn default() -> Self {
        Self {
            width: 460.0,
            max_height: f32::INFINITY,
            title: None,
            back: false,
            close: false,
            bounds: None,
        }
    }
}

/// What the chrome around a pop-up reported this frame.
#[derive(Default)]
pub struct Overlay {
    /// Esc was pressed or the back arrow clicked — go back one step.
    pub back: bool,
    /// The close control was clicked.
    pub close: bool,
    /// It has finished animating away and the caller may let go of its content.
    pub closed: bool,
}

/// A pop-up window: the veil, the frame, the entrance, and nothing about what
/// is inside.
///
/// Every overlay in the app is this plus its own content — a module's view, an
/// "are you sure?", a permission prompt. They were three near-identical copies
/// of the same thirty lines, which is how one of them ended up without the
/// entrance animation the other two had.
///
/// Call it every frame with `open`: an overlay that only exists while it is open
/// cannot animate shut.
pub fn overlay(
    ctx: &egui::Context,
    id: egui::Id,
    open: bool,
    opts: &OverlayOpts,
    add: impl FnOnce(&mut egui::Ui),
) -> Overlay {
    let mut out = Overlay::default();
    let t = if animations_enabled() {
        ctx.animate_bool_with_time(id, open, 0.13)
    } else {
        open as u8 as f32
    };
    if t <= 0.002 {
        out.closed = !open;
        return out;
    }

    // The veil dims what the pop-up belongs to and, sensing clicks across all of
    // it at a higher order than the panels, swallows them — so that area is
    // inert while the pop-up stands. Bounded rather than full-screen: everything
    // outside it, the title bar and the tab strip included, stays live.
    let screen = ctx.screen_rect();
    let bounds = opts.bounds.unwrap_or(screen);
    egui::Area::new(id.with("veil"))
        .order(egui::Order::Middle)
        .fixed_pos(bounds.min)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(bounds.size(), egui::Sense::click_and_drag());
            ui.painter()
                .rect_filled(rect, egui::Rounding::ZERO, with_alpha(color::BG, 0.72 * t));
        });

    // Esc always gets you out — a pop-up you cannot leave is a trap, and a
    // module should not be able to build one.
    if open && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        out.back = true;
    }

    let area = egui::Area::new(id.with("box"))
        .order(egui::Order::Foreground)
        // Rises the last few pixels as it arrives, and sinks as it leaves.
        // Centred on its own area, not on the window: anchoring is relative to
        // the screen, so the offset is how far that area's middle sits from it.
        .anchor(
            egui::Align2::CENTER_CENTER,
            [
                bounds.center().x - screen.center().x,
                bounds.center().y - screen.center().y + (1.0 - t) * 12.0,
            ],
        )
        .show(ctx, |ui| {
            ui.set_opacity(t);
            egui::Frame::none()
                .fill(color::BG_ELEVATED)
                .stroke(egui::Stroke::new(1.0_f32, color::BORDER))
                .inner_margin(egui::Margin::symmetric(22.0, 18.0))
                .show(ui, |ui| {
                    // Pinned width, always: module text fields ask for
                    // `f32::INFINITY`, harmless in a panel because a panel is
                    // already bounded, but an Area is not — an unpinned pop-up
                    // grows straight past both edges of the window.
                    let room = (bounds.width() - 80.0).max(320.0);
                    let target_w = opts.width.clamp(320.0, 1290.0).min(room);
                    let max_h = opts.max_height.min(bounds.height() - 100.0).max(200.0);
                    let w = animate_back(ctx, id.with("w"), target_w, 0.22);
                    ui.set_width(w);

                    // The controls are not the title's to carry: a pop-up whose
                    // content names itself still needs a way out of it.
                    if opts.title.is_some() || opts.close || opts.back {
                        ui.horizontal(|ui| {
                            if let Some(title) = &opts.title {
                                ui.label(egui::RichText::new(title).size(16.0).strong());
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    // Right-to-left, so close is placed first and
                                    // ends up outermost — the corner it occupies
                                    // on the window itself. Back sits just inside
                                    // it, with the window controls rather than
                                    // adrift on the other side of the title.
                                    if opts.close
                                        && window_button(ui, WinBtn::Close)
                                            .on_hover_text("Esc")
                                            .clicked()
                                    {
                                        out.close = true;
                                    }
                                    if opts.back && window_button(ui, WinBtn::Back).clicked() {
                                        out.back = true;
                                    }
                                },
                            );
                        });
                        ui.add_space(10.0);
                    }

                    // Height follows the content, measured on the previous frame
                    // and animated toward. Content that changes makes the box
                    // grow rather than appear at a new size — the two read as one
                    // thing changing.
                    let hkey = id.with("content_h");
                    let measured: f32 = ui.data(|d| d.get_temp(hkey)).unwrap_or(0.0);
                    let mut area = egui::ScrollArea::vertical();
                    if measured > 0.0 {
                        let h = animate_back(ctx, id.with("h"), measured.min(max_h), 0.22);
                        area = area.max_height(h).min_scrolled_height(h);
                    } else {
                        // First frame for this content: nothing measured yet, so
                        // let it size itself and record what it came to.
                        area = area.max_height(max_h);
                    }
                    area
                        // Width is the pop-up's decision, so it must not shrink
                        // to content; height is handled above.
                        .auto_shrink([false, measured <= 0.0])
                        .show(ui, |ui| {
                            ui.set_width(w);
                            add(ui);
                            let h = ui.min_rect().height();
                            ui.data_mut(|d| d.insert_temp(hkey, h));
                        });
                });
        });
    pop_layer(ctx, area.response.layer_id, area.response.rect, t);
    out
}

/// What a frame of the pop-up layer produced.
#[derive(Default)]
pub struct ModalOutcome {
    /// A button inside the pop-up was clicked.
    pub invoke: Option<Invoke>,
    /// Go back one step — Esc, the back arrow, or a `dismiss` button. At the
    /// first step that closes the pop-up; deeper in it returns to what raised it.
    pub dismissed: bool,
    /// Close the whole pop-up, however deep it went. The cross means "I am done
    /// here", which at three steps in should not mean "take me back two".
    pub close_all: bool,
    /// The closing animation has finished, so the caller can drop what it kept
    /// only so the pop-up had something to draw on the way out.
    pub closed: bool,
}

/// Draw the module pop-up layer: a module view shown over the screen it came
/// from.
///
/// The chrome is [`overlay`]; this adds what is particular to a module's own
/// window — its requested size, its title bar with back and close, and the
/// widgets it sent. `view` is the one on top, or, once the stack is empty, the
/// one still animating away, which is why the caller keeps it a moment longer
/// than the stack does.
///
/// `depth` shows the back arrow once there is somewhere to go back to.
#[allow(clippy::too_many_arguments)]
pub fn modal_layer(
    ctx: &egui::Context,
    view: Option<&View>,
    open: bool,
    depth: usize,
    bounds: Option<egui::Rect>,
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
) -> ModalOutcome {
    let id = egui::Id::new("limen_modal");
    let mut out = ModalOutcome::default();
    let Some(view) = view else {
        // Still has to be driven so the layer can finish closing.
        out.closed = overlay(
            ctx,
            id,
            false,
            &OverlayOpts {
                bounds,
                ..Default::default()
            },
            |_| {},
        )
        .closed;
        return out;
    };

    let opts = OverlayOpts {
        width: view.modal_width.unwrap_or(560.0),
        max_height: view.modal_height.unwrap_or(f32::INFINITY),
        title: Some(view.title.clone()),
        // Back appears only once there is somewhere to go back to; at the first
        // step the cross is the only way out.
        back: depth > 1,
        close: true,
        bounds,
    };

    let chrome = overlay(ctx, id, open, &opts, |ui| {
        // Depth keys the entrance so a second pop-up over the first animates in
        // as its own arrival. Read the clock before taking the data lock —
        // `input` and `data_mut` lock the same Context, and nesting them
        // deadlocks the moment a pop-up opens.
        let now = ui.input(|i| i.time);
        let key = id.with("reveal").with(depth);
        let reveal = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || now));
        if let Some(inv) = render_view(ui, view, inputs, busy, reveal) {
            out.invoke = Some(inv);
        }
    });
    out.dismissed = chrome.back;
    // The cross means "I am done here", which at three steps in should not mean
    // "take me back two".
    out.close_all = chrome.close;
    out.closed = chrome.closed;
    out
}

/// A modal "are you sure?" over the app.
///
/// The chrome is [`overlay`]; this is the question shape on top of it — a
/// centred title, the subject in its own frame, and two buttons.
///
/// Call it every frame with `open`; it animates itself in and out, so the caller
/// keeps whatever it is holding until the answer comes back. Returns
/// `Some(true)` on confirm, `Some(false)` on cancel, `None` while it is open,
/// closing, or absent.
///
/// `which` keys the dialog: the host's own questions and a module's are the same
/// dialog, but two of them sharing one id would fight over the screen.
///
/// `subject` is what the action will happen *to* — drawn in its own frame, so
/// the thing at stake is unmistakable rather than something to skim past inside
/// a sentence. Keep passing it while the dialog closes, or it will blink empty
/// on the way out.
#[allow(clippy::too_many_arguments)]
pub fn confirm_dialog(
    ctx: &egui::Context,
    which: &str,
    open: bool,
    title: &str,
    subject: Option<&str>,
    confirm_label: &str,
    cancel_label: &str,
    bounds: Option<egui::Rect>,
) -> Option<bool> {
    let mut answer = None;
    let opts = OverlayOpts {
        width: 420.0,
        bounds,
        ..Default::default()
    };
    overlay(
        ctx,
        egui::Id::new("limen_confirm").with(which),
        open,
        &opts,
        |ui| {
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(title).size(17.0).strong());
                if let Some(s) = subject {
                    ui.add_space(14.0);
                    egui::Frame::none()
                        .fill(color::BG_WIDGET)
                        .stroke(egui::Stroke::new(1.0_f32, color::BORDER))
                        .inner_margin(egui::Margin::symmetric(16.0, 9.0))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(s).strong());
                        });
                }
                ui.add_space(18.0);

                // Both buttons the same width, so the pair can be centred by
                // arithmetic rather than by hoping a layout does it.
                const BW: f32 = 150.0;
                let gap = ui.spacing().item_spacing.x;
                let pair = BW * 2.0 + gap;
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - pair) / 2.0).max(0.0));
                    if danger_button(ui, confirm_label, egui::vec2(BW, 0.0)).clicked() {
                        answer = Some(true);
                    }
                    if primary_button(ui, cancel_label, egui::vec2(BW, 0.0)).clicked() {
                        answer = Some(false);
                    }
                });
            });
        },
    );
    // Only a live dialog can be answered; a closing one is just an animation.
    open.then_some(answer).flatten()
}

/// The host's consent dialog: the same pop-up window as everything else, with
/// the permission question inside it.
///
/// Content is a closure because what needs consenting to varies — a module name,
/// the permissions it declares — while the frame, the veil and the timing should
/// not. Returns `true` once it has finished animating away, so the caller can
/// let go of whatever it was drawing.
pub fn consent_dialog(
    ctx: &egui::Context,
    open: bool,
    bounds: Option<egui::Rect>,
    contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    let opts = OverlayOpts {
        width: 460.0,
        bounds,
        ..Default::default()
    };
    overlay(
        ctx,
        egui::Id::new("limen_consent"),
        open,
        &opts,
        |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(tr("perm.title"))
                        .size(17.0)
                        .strong(),
                );
            });
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(10.0);
            contents(ui);
        },
    )
    .closed
}
