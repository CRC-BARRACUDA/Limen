//! The Docs page — Limen's own handbook, carried inside the binary.
//!
//! The docs are Markdown files under `resources/docs/<lang>/`, embedded at build
//! time. Embedded rather than read from disk on purpose: Limen is a portable
//! program that runs off a USB stick on a machine it has never seen, and help
//! that is only useful when the folder is intact is help that is missing exactly
//! when it is needed. It is also why nothing here opens a browser — the answer
//! has to be readable on a machine with no network at all.

use crate::app::*;
use crate::i18n::Lang;

/// One article, in every language it exists in.
pub struct Topic {
    /// Stable name, used by tests and never shown.
    pub id: &'static str,
    en: &'static str,
    uk: &'static str,
}

impl Topic {
    /// The article in the active language, falling back to English — the same
    /// rule the string catalogs follow, for the same reason: a half-translated
    /// handbook should show the half it has rather than a blank page.
    pub fn body(&self) -> &'static str {
        match i18n::locale() {
            Lang::Uk => self.uk,
            Lang::En => self.en,
        }
    }

    /// The heading the article opens with, which is also its entry in the list.
    /// Taken from the document rather than from a catalog key, so a title and
    /// the page it names cannot drift apart.
    pub fn title(&self) -> &'static str {
        self.body()
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("# "))
            .unwrap_or(self.id)
    }

    /// Whether `needle` (already lowercase) appears in this article's text.
    fn matches(&self, needle: &str) -> bool {
        needle.is_empty() || self.body().to_lowercase().contains(needle)
    }
}

/// The handbook, in reading order: what this is, then the two things every user
/// does, then the things they do later, and troubleshooting last.
pub const TOPICS: &[Topic] = &[
    Topic {
        id: "start",
        en: include_str!("../../../../resources/docs/en/start.md"),
        uk: include_str!("../../../../resources/docs/uk/start.md"),
    },
    Topic {
        id: "modules",
        en: include_str!("../../../../resources/docs/en/modules.md"),
        uk: include_str!("../../../../resources/docs/uk/modules.md"),
    },
    Topic {
        id: "trust",
        en: include_str!("../../../../resources/docs/en/trust.md"),
        uk: include_str!("../../../../resources/docs/uk/trust.md"),
    },
    Topic {
        id: "organising",
        en: include_str!("../../../../resources/docs/en/organising.md"),
        uk: include_str!("../../../../resources/docs/uk/organising.md"),
    },
    Topic {
        id: "language",
        en: include_str!("../../../../resources/docs/en/language.md"),
        uk: include_str!("../../../../resources/docs/uk/language.md"),
    },
    Topic {
        id: "trouble",
        en: include_str!("../../../../resources/docs/en/trouble.md"),
        uk: include_str!("../../../../resources/docs/uk/trouble.md"),
    },
];

/// Which topics a query leaves, in the order they are listed.
///
/// The search is over the whole article, not its title: somebody looking for
/// help types the word that is bothering them — "digest", "offline", "python" —
/// and that word is in the body of the page that answers it.
pub fn matching(query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    (0..TOPICS.len())
        .filter(|&i| TOPICS[i].matches(&needle))
        .collect()
}

/// Everything the page needs from the app.
pub struct DocsPage<'a> {
    /// Index into [`TOPICS`] of the article being read.
    pub selected: &'a mut usize,
    pub search: &'a mut String,
    /// Whether the contents column is out. Closing it gives the article the
    /// whole width, which is what a long page wants on a small window.
    pub contents_open: &'a mut bool,
    pub reveal_at: f64,
    pub now: f64,
    pub animate: bool,
}

/// Width of the contents column. Wide enough for the longest title in either
/// language without wrapping, which is what keeps the list scannable.
const LIST_WIDTH: f32 = 230.0;

/// The toggle's box, and so the narrowest the contents strip ever gets.
const TOGGLE: f32 = 26.0;

/// The button that puts the contents away and brings them back.
///
/// Hand-drawn rather than a glyph: the UI font is JetBrains Mono, which has no
/// chevron to set — the same reason the favourite star is drawn by hand. Drawn
/// from `open` as a number, so it turns through the same animation that moves
/// the column rather than snapping at the end of it.
fn contents_toggle(ui: &mut egui::Ui, open: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(TOGGLE, TOGGLE), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let a = ui::interact(ui, &resp);
        let color = if a.hover > 0.0 {
            ui::color::ACCENT_BRIGHT
        } else {
            ui::color::TEXT_MUTED
        };
        // The box, in the chips' idiom: an outline that brightens on hover.
        ui.painter().rect_stroke(
            rect,
            egui::Rounding::ZERO,
            egui::Stroke::new(1.0_f32, ui::with_alpha(color, 0.25 + 0.45 * a.hover)),
        );
        // A chevron pointing left when the column is out (press to put it away)
        // and right when it is away. `dir` sweeps between the two.
        let c = rect.center();
        let dir = 1.0 - 2.0 * open; // open → -1 (points left), closed → +1
        let (w, h) = (3.5_f32, 5.5_f32);
        ui.painter().add(egui::Shape::line(
            vec![
                egui::pos2(c.x - w * dir, c.y - h),
                egui::pos2(c.x + w * dir, c.y),
                egui::pos2(c.x - w * dir, c.y + h),
            ],
            egui::Stroke::new(1.6_f32, color),
        ));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn docs_page(ui: &mut egui::Ui, page: &mut DocsPage) {
    let DocsPage {
        selected,
        search,
        contents_open,
        reveal_at,
        now,
        animate,
    } = page;
    let (reveal_at, now, animate) = (*reveal_at, *now, *animate);

    // The width the column is at this instant: 1 while out, 0 while away, and
    // every value between during the sweep.
    let open = ui::anim_bool(
        ui,
        ui.id().with("docs_contents_open"),
        **contents_open,
        0.18,
    );
    reveal_item(ui, 0, reveal_at, now, animate, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(i18n::t("docs.title")).size(20.0).strong());
        });
        ui.label(
            egui::RichText::new(i18n::t("docs.subtitle"))
                .color(ui::color::TEXT_MUTED),
        );
    });
    ui.add_space(10.0);

    reveal_item(ui, 1, reveal_at, now, animate, |ui| {
        ui::text_field(ui, search, &i18n::t("docs.search_hint"), f32::INFINITY, false);
    });
    ui.add_space(10.0);

    let shown = matching(search);
    if shown.is_empty() {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(i18n::t("docs.no_match")).color(ui::color::TEXT_MUTED));
        return;
    }
    // A search that filtered out what you were reading moves you to the first
    // thing it did find, rather than leaving the pane showing a page the list no
    // longer offers.
    if !shown.contains(selected) {
        **selected = shown[0];
    }

    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        // Contents, as wide as the sweep has got. The width is what animates —
        // the article beside it is laid out from what is left, so it widens as
        // the column narrows instead of jumping when it lands.
        let list_w = LIST_WIDTH * open;
        // The strip the contents live in. It never narrows past the toggle:
        // the button that brings the column back has to be somewhere, and the
        // top of where the column was is where it went.
        let strip_w = list_w.max(TOGGLE);
        let mut toggled = false;
        ui.allocate_ui_with_layout(
            egui::vec2(strip_w, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                // Right-aligned in the column while it is out, and carried left
                // with it as it closes — so the button travels with the thing it
                // is hiding rather than jumping across the page.
                ui.horizontal(|ui| {
                    ui.add_space((strip_w - TOGGLE).max(0.0));
                    if contents_toggle(ui, open)
                        .on_hover_text(i18n::t(if **contents_open {
                            "docs.hide_contents"
                        } else {
                            "docs.show_contents"
                        }))
                        .clicked()
                    {
                        toggled = true;
                    }
                });
                ui.add_space(6.0);
                // Contents. `top_down`, not `top_down_justified`: justified
                // stretches every chip to the column's width, which centres the
                // labels and makes the list read as a column of buttons rather
                // than as contents.
                if open > 0.01 {
                    // Clipped to the width it currently has, so the titles slide
                    // out of sight rather than being re-wrapped narrower and
                    // narrower on the way.
                    let top = ui.cursor().top();
                    ui.set_clip_rect(ui.clip_rect().intersect(egui::Rect::from_min_size(
                        egui::pos2(ui.max_rect().left(), top),
                        egui::vec2(list_w, height),
                    )));
                    ui.set_opacity(open);
                    egui::ScrollArea::vertical()
                        .id_source("docs_contents")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            // The chips keep their full width while the column
                            // narrows; anything past the edge is clipped off.
                            ui.set_min_width(LIST_WIDTH);
                            for (k, &i) in shown.iter().enumerate() {
                                reveal_item(ui, k + 2, reveal_at, now, animate, |ui| {
                                    if ui::chip(ui, TOPICS[i].title(), **selected == i).clicked() {
                                        **selected = i;
                                    }
                                });
                                ui.add_space(4.0);
                            }
                        });
                }
            },
        );
        if toggled {
            **contents_open = !**contents_open;
        }
        ui.add_space(16.0);
        // A hairline between the strip and the article, so the two columns read
        // as one page rather than as two unrelated lists. It fades with the
        // column: a divider with nothing on one side of it divides nothing.
        let x = ui.cursor().left();
        ui.painter().vline(
            x,
            ui.max_rect().top()..=ui.max_rect().bottom(),
            egui::Stroke::new(1.0_f32, ui::with_alpha(ui::color::ACCENT, 0.25 * open)),
        );
        ui.add_space(16.0);

        // The article. Given its own top-down ui first: a ScrollArea inherits
        // whatever layout its parent has, and this parent is `horizontal_top` —
        // so without this every paragraph is laid out as the next *column* and
        // the page comes out as strips of vertical text.
        let width = ui.available_width();
        ui.allocate_ui_with_layout(
            egui::vec2(width, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_source("docs_article")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Long lines are unreadable across a maximised window, so
                        // the text column stops where prose stops being
                        // comfortable — and keeps that width rather than
                        // shrinking to the longest paragraph.
                        let text_w = width.min(720.0);
                        ui.set_min_width(text_w);
                        ui.set_max_width(text_w);
                        reveal_item(ui, 2, reveal_at, now, animate, |ui| {
                            ui::markdown(ui, TOPICS[**selected].body());
                        });
                        ui.add_space(24.0);
                    });
            },
        );
    });
}
