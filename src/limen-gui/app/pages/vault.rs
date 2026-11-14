//! The Vault page: Limen's vault, its own place beside Categories.

use crate::app::*;

/// The Vault tab: what the vault is, then its state and contents.
/// `installed`: the modules there are, to tell a removed one's secrets apart.
pub(crate) fn vault_page(ui: &mut egui::Ui, vault: &mut crate::app::vault::VaultUi, installed: &[String], reveal_at: f64) {
    let now = ui.input(|i| i.time);
    let animate = ui::animations_enabled();
    egui::ScrollArea::vertical()
        .id_source("vault_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            reveal_item(ui, 0, reveal_at, now, animate, |ui| {
                ui.heading(i18n::t("vault.title"));
                ui.separator();
                ui.label(egui::RichText::new(i18n::t("vault.hint")).small().color(ui::color::TEXT_MUTED));
            });
            ui.add_space(10.0);
            reveal_item(ui, 1, reveal_at, now, animate, |ui| vault.section(ui, installed));
        });
}
