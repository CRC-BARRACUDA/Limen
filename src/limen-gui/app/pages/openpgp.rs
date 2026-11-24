//! The OpenPGP page: the keys Limen trusts.

use crate::app::*;

/// A fingerprint as it is read aloud and compared: in fours.
fn in_fours(fingerprint: &str) -> String {
    fingerprint
        .as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The OpenPGP tab: what the signatures are for, and the team's keys built
/// into this Limen — each by its fingerprint, which is what is trusted.
pub(crate) fn openpgp_page(ui: &mut egui::Ui) {
    let muted = |ui: &mut egui::Ui, t: String| {
        ui.label(egui::RichText::new(t).small().color(ui::color::TEXT_MUTED));
    };
    egui::ScrollArea::vertical().id_source("openpgp_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        ui.heading(i18n::t("openpgp.title"));
        ui.separator();
        muted(ui, i18n::t("openpgp.hint"));
        ui.add_space(12.0);
        ui.label(egui::RichText::new(i18n::t("openpgp.team_keys")).strong());
        muted(ui, i18n::t("openpgp.team_keys_hint"));
        ui.add_space(8.0);
        for key in limen_sign::Keyring::team().keys() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&key.name).strong());
                let (text, color) = match key.status {
                    limen_sign::KeyStatus::Active => (i18n::t("openpgp.active"), ui::color::ACCENT_BRIGHT),
                    limen_sign::KeyStatus::Retired => (i18n::t("openpgp.retired"), ui::color::TEXT_MUTED),
                };
                // Plain words, not a pill: a pill reads as something to press.
                ui.label(egui::RichText::new(format!("· {text}")).small().color(color));
            });
            for id in &key.user_ids {
                muted(ui, id.clone());
            }
            ui.add_space(2.0);
            let fingerprint = in_fours(&key.fingerprint);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&fingerprint).monospace());
                if ui::outline_button(ui, &i18n::t("openpgp.copy"), egui::Vec2::ZERO).clicked() {
                    ui.ctx().copy_text(fingerprint.clone());
                }
            });
            if key.status == limen_sign::KeyStatus::Retired {
                muted(ui, i18n::t("openpgp.retired_hint"));
            }
            ui.add_space(12.0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_is_shown_in_fours() {
        assert_eq!(
            in_fours("8D151FD231907502BBFA528E140ABA21244518B8"),
            "8D15 1FD2 3190 7502 BBFA 528E 140A BA21 2445 18B8"
        );
    }
}
