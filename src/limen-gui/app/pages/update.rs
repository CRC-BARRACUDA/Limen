//! The Update page.

use crate::app::*;

/// The Update tab: current/latest versions + an Update button.
pub(crate) fn update_view(
    ui: &mut egui::Ui,
    info: Option<&limen_core::UpdateInfo>,
    updating: bool,
    // Held back as not signed by the team: by an unknown key, or by none.
    unsigned: Option<&Option<String>>,
    do_update: &mut bool,
    do_update_unsigned: &mut bool,
    show_changes: &mut bool,
) {
    ui.add_space(4.0);
    ui.heading(i18n::t("update.title"));
    ui.separator();
    ui.add_space(8.0);

    let Some(info) = info else {
        ui.label(egui::RichText::new(i18n::t("update.up_to_date")).color(ui::color::TEXT_MUTED));
        return;
    };

    ui.label(
        egui::RichText::new(format!(
            "{}: v{} -> v{}",
            i18n::t("update.available"),
            info.current,
            info.latest
        ))
        .size(15.0),
    );
    if !info.url.is_empty() {
        ui.add_space(2.0);
        ui.hyperlink_to(i18n::t("update.release_notes"), &info.url);
    }
    ui.add_space(14.0);
    if let Some(unknown_key) = unsigned {
        // Not the team's, as far as Limen can tell: say so, and let the user
        // decide — the one thing here that is theirs to.
        let what = match unknown_key {
            Some(key) => i18n::t("update.unknown_signer").replace("{key}", key),
            None => i18n::t("update.unsigned"),
        };
        ui.label(egui::RichText::new(what).color(ui::color::WARNING));
        ui.add_space(2.0);
        ui.label(egui::RichText::new(i18n::t("update.unsigned_hint")).small().color(ui::color::TEXT_MUTED));
        ui.add_space(8.0);
        if ui::outline_button(ui, &i18n::t("update.install_anyway"), egui::Vec2::ZERO).clicked() {
            *do_update_unsigned = true;
        }
        ui.add_space(10.0);
    }
    ui.horizontal(|ui| {
        let clicked = ui
            .add_enabled_ui(!updating, |ui| {
                ui::primary_button(ui, &i18n::t("update.button"), egui::Vec2::ZERO)
            })
            .inner
            .clicked();
        if clicked {
            *do_update = true;
        }
        // The changelog is a thing you consult and dismiss, not a thing the
        // page has to carry. Inline it was a small scrolling box under the
        // heading, which is the worst of both: too short to read, tall enough
        // to push the button that matters off the fold.
        if !info.notes.trim().is_empty()
            && ui::outline_button(ui, &i18n::t("update.changes"), egui::Vec2::ZERO).clicked()
        {
            *show_changes = true;
        }
        if updating {
            ui.add_space(6.0);
            ui.spinner();
            ui.label(egui::RichText::new(i18n::t("update.installing")).small());
        } else if info.asset_url.is_none() {
            ui.label(
                egui::RichText::new(i18n::t("update.no_binary"))
                    .small()
                    .color(ui::color::TEXT_MUTED),
            );
        }
    });
}
