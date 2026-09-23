//! The Categories page: the categories the user has made, and what is in them.

use crate::app::*;

/// What the page reads and the choices it reports back.
///
/// Nothing is applied here. Every choice is reported upward and acted on once
/// drawing is done, because applying it needs `&mut self` and the draw is
/// holding a borrow of it.
pub(crate) struct CategoriesPage<'a> {
    /// Every category and its members.
    pub all: &'a BTreeMap<String, BTreeSet<String>>,
    /// The installed modules, for the "add a module" menu and for telling a
    /// member that is present apart from one that is only remembered.
    pub modules: &'a [ModuleSpec],
    /// What is typed in the "new category" box.
    pub new_name: &'a mut String,
    /// A category to create.
    pub create: &'a mut Option<String>,
    /// (module, category) to put in or take out.
    pub toggle: &'a mut Option<(String, String)>,
    /// A category to delete outright.
    pub delete: &'a mut Option<String>,
    /// When this page was last shown, and the frame's time — the entrance is
    /// staggered off the gap between them, so the page assembles itself rather
    /// than appearing whole.
    pub reveal_at: f64,
    pub now: f64,
    /// Off when the user has turned animations off in settings, in which case
    /// everything is drawn at once at full opacity.
    pub animate: bool,
}

pub(crate) fn categories_page(ui: &mut egui::Ui, p: &mut CategoriesPage) {
    let (reveal_at, now, animate) = (p.reveal_at, p.now, p.animate);
    ui.add_space(4.0);
    // The header arrives first and the blocks follow, so the page reads as being
    // assembled top-down rather than switched on.
    reveal_item(ui, 0, reveal_at, now, animate, |ui| {
        ui.heading(i18n::t("categories.title"));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(i18n::t("categories.hint")).color(ui::color::TEXT_MUTED));
    });
    ui.add_space(10.0);

    // Making one comes first: on an empty page it is the only thing to do, and
    // once there are categories it is still what the page is for.
    reveal_item(ui, 1, reveal_at, now, animate, |ui| {
        ui.horizontal(|ui| {
            ui::text_field(ui, p.new_name, &i18n::t("categories.new_hint"), 220.0, false);
            let named = !p.new_name.trim().is_empty();
            if ui::outline_button(ui, &i18n::t("categories.add"), egui::Vec2::ZERO).clicked()
                && named
            {
                *p.create = Some(p.new_name.trim().to_string());
            }
        });
        ui.add_space(8.0);
        ui.separator();
    });

    if p.all.is_empty() {
        ui.add_space(24.0);
        reveal_item(ui, 2, reveal_at, now, animate, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(i18n::t("categories.none")).color(ui::color::TEXT_MUTED),
                );
            });
        });
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            // Keyed by name, so creating or deleting one category does not make
            // every other block restart its entrance.
            for (k, (name, members)) in p.all.iter().enumerate() {
                let id = egui::Id::new(("catblock", name.as_str()));
                // `k + 2` continues the stagger the header and the form began,
                // rather than the first block landing alongside the title.
                reveal_card(ui, id, k + 2, reveal_at, now, animate, 0.0, |ui| {
                    category_block(ui, name, members, p);
                });
            }
            ui.add_space(12.0);
        });
}

/// One category: its name, what is in it, and the menu that puts more in.
fn category_block(
    ui: &mut egui::Ui,
    name: &str,
    members: &BTreeSet<String>,
    p: &mut CategoriesPage,
) {
    egui::Frame::none()
        .fill(ui::color::BG_ELEVATED)
        .stroke(egui::Stroke::new(1.0_f32, ui::color::BORDER))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            // Force a top-down layout, and do not assume one.
            //
            // `reveal_card` draws its children inside `ui.horizontal` — that is
            // how it applies the slide offset — and an `egui::Frame` inherits
            // whatever layout its parent has. So without this, every "next row"
            // below is laid out as the next *column*: the member chips landed to
            // the right of the header, hard against the window edge, one per
            // line, with the menu pushed off screen entirely.
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width());

                // Two explicit columns rather than a right-to-left child: the width
                // is split up front, so the title cannot push the button off and the
                // button cannot take the row away from everything below it.
                ui.horizontal_top(|ui| {
                    let right_w = 96.0;
                    let spacing = ui.spacing().item_spacing.x;
                    let left_w = (ui.available_width() - right_w - spacing).max(160.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_w, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            // Force the region to fill left_w so the button is pushed
                            // to the box's end (allocate_ui otherwise collapses to
                            // content width).
                            ui.set_min_width(left_w);
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(name).size(15.0).strong());
                                ui.label(
                                    egui::RichText::new(format!("({})", members.len()))
                                        .monospace()
                                        .color(ui::color::TEXT_MUTED),
                                );
                            });
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(right_w, 0.0),
                        egui::Layout::top_down(egui::Align::Max),
                        |ui| {
                            if ui
                                .button(
                                    egui::RichText::new(i18n::t("categories.delete"))
                                        .color(ui::color::ERROR),
                                )
                                .on_hover_text(i18n::t("categories.delete_hint"))
                                .clicked()
                            {
                                *p.delete = Some(name.to_string());
                            }
                        },
                    );
                });

                ui.add_space(8.0);
                if members.is_empty() {
                    ui.label(
                        egui::RichText::new(i18n::t("categories.empty")).color(ui::color::TEXT_MUTED),
                    );
                } else {
                    ui.horizontal_wrapped(|ui| {
                        for m in members {
                            // A member that is not installed is shown greyed rather
                            // than hidden: the name is still in the user's settings,
                            // and quietly dropping it from the page is how somebody
                            // concludes the category emptied itself.
                            let present = p.modules.iter().any(|s| s.name == *m);
                            if member_chip(ui, m, present).clicked() {
                                *p.toggle = Some((m.clone(), name.to_string()));
                            }
                        }
                    });
                }

                ui.add_space(8.0);
                // Only modules not already in it — a menu offering what is already
                // ticked is a menu of no-ops.
                let addable: Vec<&ModuleSpec> = p
                    .modules
                    .iter()
                    .filter(|s| !members.contains(&s.name))
                    .collect();
                ui.menu_button(i18n::t("categories.add_module"), |ui| {
                    ui.set_min_width(200.0);
                    if addable.is_empty() {
                        ui.label(
                            egui::RichText::new(i18n::t("categories.all_in"))
                                .color(ui::color::TEXT_MUTED),
                        );
                        return;
                    }
                    for s in addable {
                        if ui.button(&s.name).clicked() {
                            *p.toggle = Some((s.name.clone(), name.to_string()));
                            ui.close_menu();
                        }
                    }
                });
            });
        });
}

/// A module inside a category. Clicking it takes it out.
fn member_chip(ui: &mut egui::Ui, name: &str, present: bool) -> egui::Response {
    let color = if present {
        ui::color::ACCENT
    } else {
        ui::color::TEXT_MUTED
    };
    let text = if present {
        name.to_string()
    } else {
        // Said in words rather than by styling alone: a colour difference is not
        // an explanation, and this one means "this module is not installed here".
        format!("{name} — {}", i18n::t("categories.missing"))
    };
    let galley =
        ui.painter()
            .layout_no_wrap(format!("{text}  ✕"), egui::FontId::proportional(12.0), color);
    let pad = egui::vec2(8.0, 3.0);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + pad * 2.0, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = resp.hovered();
        let p = ui.painter();
        p.rect_filled(
            rect,
            egui::Rounding::same(4.0),
            if hot {
                ui::color::BG_HOVER
            } else {
                ui::color::BG_WIDGET
            },
        );
        p.rect_stroke(
            rect,
            egui::Rounding::same(4.0),
            egui::Stroke::new(
                1.0_f32,
                if hot { ui::color::ERROR } else { ui::color::BORDER },
            ),
        );
        p.galley(rect.min + pad, galley, color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(i18n::t("categories.remove_module"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every piece of text the page painted, with where it was painted.
    fn painted(shape: &egui::Shape, out: &mut Vec<(egui::Pos2, String)>) {
        match shape {
            egui::Shape::Text(t) => out.push((t.pos, t.galley.text().to_string())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| painted(s, out)),
            _ => {}
        }
    }

    /// Draw the page headlessly at a known size and report what landed where.
    fn lay_out(width: f32) -> Vec<(egui::Pos2, String)> {
        let ctx = egui::Context::default();
        let mut all = BTreeMap::new();
        all.insert(
            "CrowdStrike".to_string(),
            ["aaa", "bbb", "ccc", "ddd", "eee"]
                .iter()
                .map(|s| s.to_string())
                .collect::<BTreeSet<String>>(),
        );
        let mut text = Vec::new();
        // Two frames: egui settles sizes it had to measure on the first.
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 900.0),
                )),
                ..Default::default()
            };
            let (mut name, mut create, mut toggle, mut delete) =
                (String::new(), None, None, None);
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    // Drawn exactly as the app draws it — on a plain top-down ui.
                    // The horizontal that caused the bug is inside the page, in
                    // `reveal_card`, so it is exercised without being staged here.
                    categories_page(
                        ui,
                        &mut CategoriesPage {
                            all: &all,
                            modules: &[],
                            new_name: &mut name,
                            create: &mut create,
                            toggle: &mut toggle,
                            delete: &mut delete,
                            reveal_at: 0.0,
                            now: 1_000.0,
                            animate: true,
                        },
                    );
                });
            });
            text.clear();
            for c in &out.shapes {
                painted(&c.shape, &mut text);
            }
        }
        text
    }

    fn find<'a>(text: &'a [(egui::Pos2, String)], needle: &str) -> &'a (egui::Pos2, String) {
        text.iter()
            .find(|(_, s)| s.contains(needle))
            .unwrap_or_else(|| panic!("never painted {needle:?}; saw {:?}",
                                      text.iter().map(|(_, s)| s).collect::<Vec<_>>()))
    }

    /// The members belong **below** the heading and **inside** the window.
    ///
    /// They once landed to the right of it instead, hard against the window edge
    /// and one per line, because `reveal_card` draws its children inside a
    /// horizontal and a `Frame` inherits its parent's layout — so each "next row"
    /// became the next column. Nothing about that is visible in the source of
    /// this page, which is why it is asserted here rather than left to the eye.
    #[test]
    fn members_sit_below_the_heading_and_on_screen() {
        const W: f32 = 1200.0;
        let text = lay_out(W);
        let (head, _) = find(&text, "CrowdStrike");

        let mut leftmost = f32::MAX;
        for m in ["aaa", "bbb", "ccc", "ddd", "eee"] {
            let (at, painted) = find(&text, m);
            assert!(!painted.is_empty());
            assert!(
                at.y > head.y,
                "member {m:?} was painted at y={}, not below the heading at y={}",
                at.y, head.y
            );
            // On screen at all. When the layout inherited a horizontal these sat
            // at the window's right edge and ran off it.
            assert!(
                at.x < W - 40.0,
                "member {m:?} was painted at x={} in a {W}px window — off the edge",
                at.x
            );
            leftmost = leftmost.min(at.x);
        }
        // And the row starts where the panel does, rather than somewhere out to
        // the right — which is the difference the bug actually made.
        assert!(
            leftmost < 100.0,
            "the members begin at x={leftmost}, not at the left of the panel"
        );
    }

    /// All five members share a line or two — not five lines of one.
    ///
    /// One per line is what a wrap width of nearly nothing produces, and it looks
    /// enough like a design choice to survive a glance.
    #[test]
    fn the_members_wrap_together_rather_than_one_per_line() {
        let text = lay_out(1200.0);
        let mut rows: Vec<f32> = ["aaa", "bbb", "ccc", "ddd", "eee"]
            .iter()
            .map(|m| find(&text, m).0.y)
            .collect();
        rows.sort_by(|a, b| a.partial_cmp(b).unwrap());
        rows.dedup_by(|a, b| (*a - *b).abs() < 2.0);
        assert!(
            rows.len() <= 2,
            "five short members took {} lines — they are not sharing a row",
            rows.len()
        );
    }
}
