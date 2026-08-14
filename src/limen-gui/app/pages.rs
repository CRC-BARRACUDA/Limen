//! The pages a tab can show, and the pieces they share.

use super::*;

mod about;
mod developer;
mod module;
mod modules;
mod settings;
mod update;

pub(crate) use about::*;
pub(crate) use developer::*;
pub(crate) use module::*;
pub(crate) use modules::*;
pub(crate) use settings::*;
pub(crate) use update::*;

/// The Modules-page installed/available filter.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ModuleFilter {
    All,
    Installed,
    Available,
}

/// An installed module's card description, translated for the active UI language
/// from its `locales/<lang>.toml`, else the manifest default. The file read is
/// cached in egui memory per (module, language), so language switches update the
/// cards with no engine reload and no per-frame disk I/O.
pub(crate) fn localized_desc(ui: &egui::Ui, m: &ModuleSpec) -> Option<String> {
    let lang = i18n::locale();
    let id = egui::Id::new(("moddesc", m.name.as_str(), lang.code()));
    let resolved = match ui.data(|d| d.get_temp::<Option<String>>(id)) {
        Some(v) => v,
        None => {
            let r = limen_proto::manifest::localized_description(&m.cwd, lang.code());
            ui.data_mut(|d| d.insert_temp(id, r.clone()));
            r
        }
    };
    resolved.or_else(|| m.description.clone())
}

/// An installed module's display title, translated for the active UI language
/// from its `locales/<lang>.toml` `[module] title`, else the manifest
/// `display_name`, else the module's identifier `name`. Cached like `localized_desc`.
pub(crate) fn localized_name(ctx: &egui::Context, m: &ModuleSpec) -> String {
    let lang = i18n::locale();
    let id = egui::Id::new(("modtitle", m.name.as_str(), lang.code()));
    let resolved = match ctx.data(|d| d.get_temp::<Option<String>>(id)) {
        Some(v) => v,
        None => {
            let r = limen_proto::manifest::localized_title(&m.cwd, lang.code());
            ctx.data_mut(|d| d.insert_temp(id, r.clone()));
            r
        }
    };
    resolved
        .or_else(|| m.display_name.clone())
        .unwrap_or_else(|| m.name.clone())
}

/// A single installed-module card, in its own rounded box. `from_git` shows the
/// GitHub action only for modules installed from a repo (manual ones get just
/// Open + Remove).
/// Wrap a list card in its animations: a staggered slide-in from the left on first
/// reveal (delayed by `k`), multiplied by `filter_t` — the card's fade as it
/// enters/leaves the list on a filter change. `filter_t` is 1 while shown, eases
/// to 0 as it leaves; persistent cards sit at 1 and don't re-animate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reveal_card(
    ui: &mut egui::Ui,
    id: egui::Id,
    k: usize,
    reveal_at: f64,
    now: f64,
    animate: bool,
    remove_t: f32,
    draw: impl FnOnce(&mut egui::Ui),
) {
    // Trailing gap between cards (also collapses during a removal).
    const GAP: f32 = 10.0;
    if !animate {
        draw(ui);
        ui.add_space(GAP);
        return;
    }
    // Staggered entrance (replays when the reveal timer resets: tab shown or
    // filter changed) and a smoothstep exit so the fade, slide, and — most
    // importantly — the height collapse all progress steadily.
    let enter = ui::reveal_t(ui, k, reveal_at, now, 0.03, 0.18);
    let exit = ui::smoothstep(remove_t);

    let dx = (1.0 - enter) * 28.0 + exit * 28.0;
    if remove_t > 0.0 && remove_t < 1.0 {
        ui.ctx().request_repaint(); // keep the exit animating until settled
    }

    if exit <= 0.0 {
        // Normal render — and remember the height so a later removal can collapse
        // it smoothly (measured on a non-collapsing frame).
        let inner = ui.scope(|ui| {
            ui.set_opacity(enter);
            ui.horizontal(|ui| {
                ui.add_space(dx);
                draw(ui);
            });
        });
        ui.data_mut(|d| d.insert_temp(id, inner.response.rect.height()));
        ui.add_space(GAP);
        return;
    }

    // Removing: shrink the vertical space (card height + gap) so the cards below
    // slide up to fill it, while the card fades and slides out to the right.
    let full_h: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(90.0);
    let width = ui.available_width();
    let h = full_h * (1.0 - exit);
    let (outer, _) = ui.allocate_exact_size(egui::vec2(width, h), egui::Sense::hover());
    let mut child = ui.child_ui_with_id_source(
        egui::Rect::from_min_size(outer.min, egui::vec2(width, full_h)),
        egui::Layout::top_down(egui::Align::Min),
        id,
        None,
    );
    child.set_clip_rect(child.clip_rect().intersect(outer));
    child.set_opacity(enter * (1.0 - exit));
    child.horizontal(|ui| {
        ui.add_space(dx);
        draw(ui);
    });
    ui.add_space(GAP * (1.0 - exit));
}

/// Turn an `owner/repo` (or full URL) into a browsable GitHub URL.
pub(crate) fn repo_url(repo: &str) -> String {
    if repo.contains("://") {
        repo.to_string()
    } else {
        format!("https://github.com/{repo}")
    }
}

/// A dependency row: `Requires: programs`, naming the *modules* that provide the
/// capabilities rather than the capability strings — `programs`, not
/// `programs.local`.
///
/// A name is clickable when something installed provides it; clicking searches
/// for that module so its card comes into view. When nothing provides it, the
/// capability string is shown as plain text instead — still visible, but there
/// is nothing to jump to.
pub(crate) fn dep_row<'a>(
    ui: &mut egui::Ui,
    prefix: &str,
    prefix_color: egui::Color32,
    caps: impl Iterator<Item = &'a String>,
    providers: &HashMap<&str, &ModuleSpec>,
    find: &mut Option<String>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(egui::RichText::new(prefix).small().color(prefix_color));
        for (i, cap) in caps.enumerate() {
            if i > 0 {
                ui.label(
                    egui::RichText::new("·")
                        .small()
                        .color(ui::color::TEXT_MUTED),
                );
            }
            match providers.get(cap.as_str()) {
                Some(p) => {
                    let clicked = ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(&p.name)
                                    .small()
                                    .underline()
                                    .color(ui::color::ACCENT),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked();
                    if clicked {
                        *find = Some(p.name.clone());
                    }
                }
                None => {
                    ui.label(
                        egui::RichText::new(cap)
                            .small()
                            .color(ui::color::TEXT_MUTED),
                    );
                }
            }
        }
    });
}

/// A small rounded pill, like Zed's category tags.
/// A clickable tag chip, drawn as `#tag`.
///
/// Distinct from [`badge`] (capabilities), which is inert: a tag is the user's
/// handle for filtering, so it reacts to hover and clicking one puts it in the
/// search box. Laid out by measuring the galley first, so hover is correct on
/// the same frame rather than a frame late.
pub(crate) fn tag_chip(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        format!("#{text}"),
        egui::FontId::proportional(11.0),
        ui::color::ACCENT,
    );
    let pad = egui::vec2(7.0, 2.5);
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
                if hot {
                    ui::color::ACCENT
                } else {
                    ui::color::BORDER
                },
            ),
        );
        p.galley(
            rect.min + pad,
            galley,
            if hot {
                ui::color::ACCENT_BRIGHT
            } else {
                ui::color::ACCENT
            },
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(crate) fn badge(ui: &mut egui::Ui, text: &str) {
    egui::Frame::none()
        .fill(ui::color::BG_WIDGET)
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(11.0)
                    .color(ui::color::TEXT_MUTED),
            );
        });
}

/// The About page. Returns `true` if the "License" button was clicked.
/// Fade a single About-screen element in, staggered by `k`, so the block
/// assembles itself one line at a time when the tab is shown.
pub(crate) fn reveal_item(
    ui: &mut egui::Ui,
    k: usize,
    reveal_at: f64,
    now: f64,
    animate: bool,
    draw: impl FnOnce(&mut egui::Ui),
) {
    if !animate {
        draw(ui);
        return;
    }
    let t = ui::reveal_t(ui, k, reveal_at, now, 0.02, 0.12);
    ui.scope(|ui| {
        ui.set_opacity(t);
        draw(ui);
    });
}
