//! Text that arrives a letter or a word at a time.

use crate::*;

/// How a run of text arrives on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reveal {
    /// One character at a time.
    Letters,
    /// A whole word at a time — legible at speeds where letters are a blur.
    ///
    /// Nothing on screen asks for it yet; it is here because a long paragraph
    /// wants it, and because a reveal that can only count characters is half an
    /// answer. Covered by the tests either way.
    #[allow(dead_code)]
    Words,
}

/// How much of `text` has arrived after `elapsed` seconds at `per_sec` units.
///
/// Returns a byte length, and always one that falls on a character boundary:
/// slicing a string by a count of characters is the same as slicing it by bytes
/// only in ASCII, and this application is bilingual — the first Cyrillic letter
/// in a Ukrainian heading would panic.
pub fn revealed_len(text: &str, mode: Reveal, elapsed: f64, per_sec: f64) -> usize {
    if per_sec <= 0.0 || elapsed < 0.0 {
        return 0;
    }
    let want = (elapsed * per_sec).floor();
    if want >= text.chars().count() as f64 {
        return text.len();
    }
    let want = want as usize;
    match mode {
        Reveal::Letters => text.char_indices().nth(want).map_or(text.len(), |(i, _)| i),
        Reveal::Words => {
            // A word ends where a run of non-whitespace does. Revealing up to
            // the *end* of the n-th word rather than the start of the next one
            // keeps the trailing space with the word that follows it, so the
            // line does not twitch as each space lands on its own.
            let mut seen = 0;
            let mut in_word = false;
            for (i, c) in text.char_indices() {
                if c.is_whitespace() {
                    if in_word {
                        seen += 1;
                        if seen >= want {
                            return i;
                        }
                        in_word = false;
                    }
                } else {
                    in_word = true;
                }
            }
            text.len()
        }
    }
}

/// The prefix of `text` that has arrived. See [`revealed_len`].
///
/// The widget works in byte lengths and has no use for this; it is the readable
/// form of the same answer, and what the tests are written against.
#[allow(dead_code)]
pub fn revealed(text: &str, mode: Reveal, elapsed: f64, per_sec: f64) -> &str {
    &text[..revealed_len(text, mode, elapsed, per_sec)]
}

/// How a [`typed_label`] looks and how fast it arrives.
///
/// A struct rather than five more arguments, and it has a `Default` so a call
/// site names only what it is changing.
pub struct Typed {
    pub font: egui::FontId,
    pub color: egui::Color32,
    /// Where the text sits in the width it is given.
    pub align: egui::Align,
    pub mode: Reveal,
    /// Letters or words per second.
    pub per_sec: f64,
}

impl Default for Typed {
    fn default() -> Self {
        Typed {
            font: egui::FontId::proportional(13.0),
            color: color::TEXT,
            align: egui::Align::Min,
            mode: Reveal::Letters,
            per_sec: 90.0,
        }
    }
}

/// A label that types itself out. Returns `true` once all of it is showing.
///
/// The text is laid out once, in full, and revealed by clipping — never by
/// laying out a growing prefix. That matters three times over:
///
/// * nothing moves. Every glyph is already in its final position, so a centred
///   line does not slide left as it grows and the page below does not reflow.
/// * the layout is cached, because the same text and width are asked for every
///   frame. Re-laying out a prefix would be new work each frame, on the one
///   thread that must never do any.
/// * it therefore costs the same whether the text is a heading or the whole of
///   the GPL.
///
/// The clock starts when the text is first seen and restarts when it changes —
/// so switching language mid-animation retypes the new string rather than
/// revealing the tail of one that is no longer there.
pub fn typed_label(ui: &mut egui::Ui, id: egui::Id, text: &str, style: &Typed) -> bool {
    let Typed {
        ref font,
        color,
        align,
        mode,
        per_sec,
    } = *style;
    let wrap = ui.available_width();
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font.clone(), color, wrap);
    job.halign = align;
    let galley = ui.fonts(|f| f.layout_job(job));

    // The full width is claimed, not just the text's, so `align` has something
    // to align within — otherwise a short heading is centred inside its own
    // narrow box and still sits on the left of the window.
    let (rect, _) = ui.allocate_exact_size(egui::vec2(wrap, galley.size().y), egui::Sense::hover());
    let anchor = match align {
        egui::Align::Min => rect.left_top(),
        egui::Align::Center => egui::pos2(rect.center().x, rect.top()),
        egui::Align::Max => rect.right_top(),
    };

    let paint_all = |ui: &egui::Ui| ui.painter().galley(anchor, galley.clone(), color);
    if !animations_enabled() {
        paint_all(ui);
        return true;
    }

    // The clock, before the data lock: reading input while holding it deadlocks.
    let now = ui.input(|i| i.time);
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        h.finish()
    };
    let start = ui.data_mut(|d| match d.get_temp::<(u64, f64)>(id) {
        Some((k, t)) if k == key => t,
        _ => {
            d.insert_temp(id, (key, now));
            now
        }
    });

    let shown = revealed_len(text, mode, now - start, per_sec);
    if shown == text.len() {
        paint_all(ui);
        return true;
    }
    ui.ctx().request_repaint();
    if shown == 0 {
        return false;
    }

    // Where the reveal has reached, in the finished layout.
    let chars = text[..shown].chars().count();
    let caret = galley.pos_from_cursor(&galley.from_ccursor(egui::text::CCursor::new(chars)));
    let (top, bottom) = (anchor.y + caret.top(), anchor.y + caret.bottom());
    let x = anchor.x + caret.left();

    // Two draws of the one galley: every line above the caret in full, and the
    // line it is on up to the caret. A single rectangle cannot express that.
    let clip = |r: egui::Rect| r.intersect(ui.clip_rect());
    let full_rows = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top()), egui::pos2(rect.right(), top));
    let part_row = egui::Rect::from_min_max(egui::pos2(rect.left(), top), egui::pos2(x, bottom));
    for r in [full_rows, part_row] {
        if r.is_positive() {
            ui.painter().with_clip_rect(clip(r)).galley(anchor, galley.clone(), color);
        }
    }

    // A caret, so a pause reads as typing rather than as a page that stopped
    // loading.
    let bar = egui::Rect::from_min_size(egui::pos2(x + 1.0, top + 2.0), egui::vec2(2.0, (bottom - top - 4.0).max(2.0)));
    ui.painter().rect_filled(bar, 0.0, color);
    false
}

// ---- widgets --------------------------------------------------------------- //
