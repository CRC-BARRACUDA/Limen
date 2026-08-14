//! The Settings page.

use crate::app::*;

/// The Settings tab. `reveal_at` staggers the sections in when the tab is shown.
#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_view(
    ui: &mut egui::Ui,
    scale: &mut f32,
    changed: &mut bool,
    animations: &mut bool,
    anim_changed: &mut bool,
    alerts: &mut bool,
    alerts_changed: &mut bool,
    lang: &mut i18n::Lang,
    lang_changed: &mut bool,
    reveal_at: f64,
) {
    let now = ui.input(|i| i.time);
    let animate = ui::animations_enabled();
    let hint = |ui: &mut egui::Ui, key: &str| {
        ui.label(
            egui::RichText::new(i18n::t(key))
                .small()
                .color(ui::color::TEXT_MUTED),
        );
    };

    ui.add_space(4.0);
    reveal_item(ui, 0, reveal_at, now, animate, |ui| {
        ui.heading(i18n::t("settings.title"));
        ui.separator();
    });

    // Language — the endonyms stay untranslated (a picker always shows each
    // language in its own name).
    ui.add_space(6.0);
    reveal_item(ui, 1, reveal_at, now, animate, |ui| {
        ui.label(egui::RichText::new(i18n::t("settings.language")).strong());
        hint(ui, "settings.language_hint");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for l in i18n::Lang::ALL {
                if ui::chip(ui, l.label(), *lang == l).clicked() {
                    *lang = l;
                    *lang_changed = true;
                }
            }
        });
    });

    ui.add_space(16.0);
    reveal_item(ui, 2, reveal_at, now, animate, |ui| {
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(i18n::t("settings.ui_scale")).strong());
        hint(ui, "settings.ui_scale_hint");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for pct in [100.0_f32, 125.0, 150.0, 175.0, 200.0] {
                let selected = (*scale - pct).abs() < 0.5;
                if ui::chip(ui, &format!("{}%", pct as u32), selected).clicked() {
                    *scale = pct;
                    *changed = true;
                }
            }
        });
    });

    ui.add_space(16.0);
    reveal_item(ui, 3, reveal_at, now, animate, |ui| {
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(i18n::t("settings.animations")).strong());
        hint(ui, "settings.animations_hint");
        ui.add_space(6.0);
        if ui::toggle(ui, animations, &i18n::t("settings.animations_toggle")).changed() {
            *anim_changed = true;
        }
    });

    ui.add_space(16.0);
    reveal_item(ui, 4, reveal_at, now, animate, |ui| {
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(i18n::t("settings.alerts")).strong());
        hint(ui, "settings.alerts_hint");
        ui.add_space(6.0);
        if ui::toggle(ui, alerts, &i18n::t("settings.alerts_toggle")).changed() {
            *alerts_changed = true;
        }
    });
}
