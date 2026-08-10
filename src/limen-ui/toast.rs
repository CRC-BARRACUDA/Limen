//! Passing notices: what just happened, said in the corner and then gone.
//!
//! The four the platform ui-kit defines — info, ok, warning, error — with its
//! colours, its 1px border and 2px accent edge, and a word for which of the four
//! this one is.
//!
//! Two things differ from the kit, and both because this is an application
//! rather than a page. It sits under the window controls at the top right, out
//! of the tab strip's way. And it carries its own dark backing: the kit's 5%
//! wash is legible over a page that is solid behind it, and illegible over a
//! table of results.
//!
//! A notice is not a dialog. It never blocks, it never asks, and it leaves on
//! its own. Anything that needs an answer wants [`crate::confirm_dialog`].

use crate::*;

/// How long a notice stays before it starts leaving, and how long leaving takes.
///
/// Public because the countdown bar draws it and a test checks the bar: a test
/// that hard-codes the number instead would keep passing after the number
/// changed, which is the one thing it exists to catch.
pub const DWELL: f64 = 3.0;
const FADE: f64 = 0.22;

/// The width of the stack, the gap between notices, and how far below the top
/// they start — clear of the window controls.
const WIDTH: f32 = 264.0;
const GAP: f32 = 8.0;
const TOP: f32 = 44.0;

/// What kind of thing happened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// Something worth knowing.
    Info,
    /// Something finished, and finished well.
    Ok,
    /// Something is not right, but nothing failed.
    Warning,
    /// Something failed.
    Error,
}

impl Level {
    /// The kit's own colours: a text colour, a 0.6-ish accent for the leading
    /// edge, a 0.35 border, and a 0.05 wash behind it.
    fn ink(self) -> egui::Color32 {
        match self {
            Level::Info => egui::Color32::from_rgb(0xf0, 0xa8, 0x50),
            Level::Ok => egui::Color32::from_rgb(0x4a, 0xde, 0x80),
            Level::Warning => egui::Color32::from_rgb(0xfb, 0xbf, 0x24),
            Level::Error => egui::Color32::from_rgb(0xf8, 0x71, 0x71),
        }
    }

    /// What sits behind the words: the panel colour with the level's colour
    /// washed faintly through it.
    ///
    /// One colour rather than a second rectangle over the first. A wash painted
    /// as its own rect has to be given one, and the rect to hand inside a frame
    /// is the space *available*, not the space used — so it spills past the
    /// border on whichever sides the content did not fill.
    fn backing(self) -> egui::Color32 {
        let (bg, ink) = (color::BG_ELEVATED, self.ink());
        let mix = |b: u8, i: u8| (b as f32 * 0.94 + i as f32 * 0.06) as u8;
        egui::Color32::from_rgb(
            mix(bg.r(), ink.r()),
            mix(bg.g(), ink.g()),
            mix(bg.b(), ink.b()),
        )
    }

    /// What this level is called, in the reader's language.
    ///
    /// Asked fresh every frame rather than stored: that is what lets a change of
    /// language reach a notice that is already on screen.
    pub fn name(self) -> String {
        match NAMER.get() {
            Some(namer) => namer(self),
            None => self.untranslated().to_owned(),
        }
    }

    /// The name with no host to ask. The toolkit is usable on its own — the kit
    /// gallery and the tests draw notices with nothing installed — and a notice
    /// labelled in English beats one labelled `toast.info`.
    fn untranslated(self) -> &'static str {
        match self {
            Level::Info => "Info",
            Level::Ok => "Done",
            Level::Warning => "Warning",
            Level::Error => "Error",
        }
    }
}

/// How the four levels say their own names.
///
/// `ui` has no catalog of its own and is not going to grow one — a toolkit that
/// translates is a toolkit every caller has to agree with about languages. The
/// host installs this at startup instead, and until it does the English names
/// stand.
static NAMER: std::sync::OnceLock<fn(Level) -> String> = std::sync::OnceLock::new();

/// Teach the levels their names. Called once, by whoever owns the catalog.
pub fn set_namer(namer: fn(Level) -> String) {
    let _ = NAMER.set(namer);
}

/// Whether notices are shown at all. On by default.
static SHOWING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turn notices on or off. Off, nothing is raised and nothing is queued: a
/// notice suppressed is a notice that never happened, not one waiting.
pub fn set_enabled(on: bool) {
    SHOWING.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn enabled() -> bool {
    SHOWING.load(std::sync::atomic::Ordering::Relaxed)
}

/// How much of a notice's time is left, as a fraction: 1 the moment it appears,
/// 0 when it is due to go.
///
/// Pulled out so the bar that draws it can be checked without a window.
pub fn remaining(born: f64, now: f64) -> f32 {
    (1.0 - ((now - born) / DWELL)).clamp(0.0, 1.0) as f32
}

/// One notice on screen.
#[derive(Clone)]
struct Notice {
    level: Level,
    text: String,
    /// When it appeared, and when it was told to go — by a click, or by time.
    born: f64,
    leaving: Option<f64>,
}

fn queue_id() -> egui::Id {
    egui::Id::new("limen_toasts")
}

/// Raise a notice. It shows immediately and leaves by itself.
pub fn notify(ctx: &egui::Context, level: Level, text: impl Into<String>) {
    if !enabled() {
        return;
    }
    let now = ctx.input(|i| i.time);
    let notice = Notice {
        level,
        text: text.into(),
        born: now,
        leaving: None,
    };
    ctx.data_mut(|d| {
        let q: &mut Vec<Notice> = d.get_temp_mut_or_insert_with(queue_id(), Vec::new);
        q.push(notice);
    });
    ctx.request_repaint();
}

/// How many notices are on screen. For tests, and for anything that needs to
/// know whether the corner is busy.
pub fn showing(ctx: &egui::Context) -> usize {
    ctx.data(|d| d.get_temp::<Vec<Notice>>(queue_id()).map_or(0, |q| q.len()))
}

/// Draw whatever is showing, and forget whatever has finished leaving.
///
/// Call once a frame, after everything else: a notice sits above the pop-ups,
/// because it is telling you something about the thing the pop-up is doing.
pub fn draw(ctx: &egui::Context) {
    // The clock before the data lock: reading input while holding it deadlocks.
    let now = ctx.input(|i| i.time);
    let mut queue: Vec<Notice> = ctx.data(|d| d.get_temp(queue_id())).unwrap_or_default();
    if queue.is_empty() {
        return;
    }

    // Time is up: start it leaving. Its own clock, so a new notice never makes
    // an older one wait.
    for n in queue.iter_mut() {
        if n.leaving.is_none() && now - n.born > DWELL {
            n.leaving = Some(now);
        }
    }

    // Top right, below the window controls.
    let screen = ctx.screen_rect();
    let corner = egui::pos2(screen.right() - WIDTH - GAP, screen.top() + TOP);
    let mut y = corner.y;
    let mut dismissed = Vec::new();
    for (i, n) in queue.iter().enumerate() {
        let t = match n.leaving {
            None => ease_out((((now - n.born) / FADE).clamp(0.0, 1.0)) as f32),
            Some(at) => 1.0 - ease_out((((now - at) / FADE).clamp(0.0, 1.0)) as f32),
        };
        let t = if animations_enabled() { t } else { n.leaving.is_none() as u8 as f32 };
        // Faded out *and* on its way out. An arriving notice is also at zero
        // on its first frame, and taking that for "finished leaving" removes it
        // before it has ever been drawn — which is a notice that never appears.
        if t <= 0.002 && n.leaving.is_some() {
            dismissed.push(i);
            continue;
        }
        let height = draw_one(ctx, n, egui::pos2(corner.x, y), t, now, i, &mut dismissed);
        y += height + GAP;
    }

    // Anything that finished leaving, or was clicked, goes.
    if !dismissed.is_empty() {
        ctx.data_mut(|d| {
            let q: &mut Vec<Notice> = d.get_temp_mut_or_insert_with(queue_id(), Vec::new);
            let mut i = 0;
            q.retain(|_| {
                let keep = !dismissed.contains(&i);
                i += 1;
                keep
            });
        });
    } else {
        ctx.data_mut(|d| d.insert_temp(queue_id(), queue));
    }
    ctx.request_repaint();
}

/// One notice, returning how tall it came out.
#[allow(clippy::too_many_arguments)]
fn draw_one(
    ctx: &egui::Context,
    n: &Notice,
    at: egui::Pos2,
    t: f32,
    now: f64,
    index: usize,
    dismissed: &mut Vec<usize>,
) -> f32 {
    let ink = n.level.ink();
    let a = |k: f32| with_alpha(ink, k * t);
    // It arrives from the edge it lives on, so the movement says where it came
    // from rather than merely that something appeared.
    let dx = (1.0 - t) * 24.0;

    let area = egui::Area::new(egui::Id::new(("limen_toast", index)))
        // Above the pop-ups: a notice about what a dialog is doing is no use
        // underneath it.
        .order(egui::Order::Tooltip)
        .fixed_pos(at + egui::vec2(dx, 0.0))
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_opacity(t);
            ui.set_width(WIDTH);
            egui::Frame::none()
                // Its own backing, mostly opaque. Without this the page shows
                // through the words.
                .fill(with_alpha(n.level.backing(), 0.94))
                .stroke(egui::Stroke::new(1.0_f32, a(0.45)))
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    // The frame is as wide as its content unless told otherwise,
                    // so without this each notice is the width of its own
                    // longest line and the stack has a ragged edge.
                    ui.set_min_width(WIDTH - 20.0);
                    ui.horizontal_top(|ui| {
                        let (icon, _) =
                            ui.allocate_exact_size(egui::vec2(13.0, 13.0), egui::Sense::hover());
                        paint_icon(ui.painter(), icon, n.level, a(1.0));
                        ui.add_space(7.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(
                                egui::RichText::new(n.level.name())
                                    .monospace()
                                    .size(9.5)
                                    .color(a(0.9)),
                            );
                            ui.label(
                                egui::RichText::new(&n.text)
                                    .size(11.5)
                                    .color(color::TEXT),
                            );
                        });
                    });
                })
                .response
        });

    // The frame's rect, not the area's: the area is as wide as it was allowed to
    // be, the frame as wide as it drew. Everything below is an edge of the box,
    // so it has to be measured from the box.
    let rect = area.inner.rect;
    // And inside the border rather than over it. A bar laid along the bottom
    // edge itself replaces the border for as far as it runs, which reads as a
    // box whose outline stops half way.
    let inner = rect.shrink(1.0);
    // How long it has left, along the bottom edge. A notice that leaves on its
    // own should say when — otherwise it disappears mid-sentence and the only
    // way to read the rest is to make it happen again.
    let left = remaining(n.born, now);
    if left > 0.0 && n.leaving.is_none() {
        let painter = ctx.layer_painter(area.response.layer_id);
        let track = egui::Rect::from_min_size(
            egui::pos2(inner.left(), inner.bottom() - 2.0),
            egui::vec2(inner.width(), 2.0),
        );
        painter.rect_filled(track, egui::Rounding::ZERO, a(0.12));
        painter.rect_filled(
            egui::Rect::from_min_size(track.min, egui::vec2(track.width() * left, 2.0)),
            egui::Rounding::ZERO,
            a(0.55),
        );
    }
    // The 2px leading edge the kit draws down the left of every alert.
    ctx.layer_painter(area.response.layer_id).rect_filled(
        egui::Rect::from_min_size(inner.min, egui::vec2(2.0, inner.height())),
        egui::Rounding::ZERO,
        a(0.6),
    );
    // Clicked anywhere: go now rather than wait out the dwell.
    if area.response.interact(egui::Sense::click()).clicked() {
        dismissed.push(index);
    }
    rect.height()
}

/// The four marks, painted rather than typed: the application ships one font,
/// and it has no check mark in it.
fn paint_icon(painter: &egui::Painter, rect: egui::Rect, level: Level, ink: egui::Color32) {
    let stroke = egui::Stroke::new(1.3_f32, ink);
    let c = rect.center();
    let r = rect.width() * 0.44;
    match level {
        Level::Ok => {
            painter.circle_stroke(c, r, stroke);
            painter.line_segment([c + egui::vec2(-2.6, 0.2), c + egui::vec2(-0.8, 2.1)], stroke);
            painter.line_segment([c + egui::vec2(-0.8, 2.1), c + egui::vec2(2.8, -2.1)], stroke);
        }
        Level::Error => {
            painter.circle_stroke(c, r, stroke);
            painter.line_segment([c + egui::vec2(-2.1, -2.1), c + egui::vec2(2.1, 2.1)], stroke);
            painter.line_segment([c + egui::vec2(2.1, -2.1), c + egui::vec2(-2.1, 2.1)], stroke);
        }
        Level::Info => {
            painter.circle_stroke(c, r, stroke);
            painter.circle_filled(c + egui::vec2(0.0, -2.5), 1.0, ink);
            painter.line_segment([c + egui::vec2(0.0, -0.6), c + egui::vec2(0.0, 2.8)], stroke);
        }
        Level::Warning => {
            // A triangle, which is what a warning is everywhere.
            let p = [
                c + egui::vec2(0.0, -r - 0.6),
                c + egui::vec2(r + 0.6, r * 0.9),
                c + egui::vec2(-r - 0.6, r * 0.9),
            ];
            painter.add(egui::Shape::closed_line(p.to_vec(), stroke));
            painter.line_segment([c + egui::vec2(0.0, -2.0), c + egui::vec2(0.0, 1.0)], stroke);
            painter.circle_filled(c + egui::vec2(0.0, 2.6), 1.0, ink);
        }
    }
}
