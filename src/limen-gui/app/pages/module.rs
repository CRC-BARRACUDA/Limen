//! A module's own screen, and a detail view opened from a table row.

use crate::app::*;

#[allow(clippy::too_many_arguments)]
/// Render a detail tab opened from a table row action (e.g. "About device").
/// Its view is interactive too, so buttons inside it (e.g. "Open path") dispatch.
pub(crate) fn detail_view(
    ui: &mut egui::Ui,
    id: u64,
    detail_tabs: &mut HashMap<u64, DetailTab>,
    action: &mut Option<ui::Invoke>,
) {
    let Some(tab) = detail_tabs.get_mut(&id) else {
        ui.label(i18n::t("detail.unavailable"));
        return;
    };
    let heading = if tab.title.is_empty() {
        i18n::t("tab.details")
    } else {
        tab.title.clone()
    };
    if let Some(err) = &tab.error {
        ui.heading(&heading);
        ui.separator();
        ui.colored_label(egui::Color32::LIGHT_RED, err);
        return;
    }
    match &tab.view {
        Some(v) => {
            let heading = if v.title.is_empty() {
                heading
            } else {
                v.title.clone()
            };
            ui.heading(heading);
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Dev UI-kit preview: no entrance animation.
                    if let Some(a) = ui::render_view(ui, v, &mut tab.inputs, None, 0.0) {
                        *action = Some(a);
                    }
                });
        }
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(i18n::t("detail.loading"));
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn module_view(
    ui: &mut egui::Ui,
    name: &str,
    reveal_at: f64,
    view: &Option<ui::View>,
    view_error: &Option<String>,
    inputs: &mut HashMap<String, String>,
    output: &str,
    busy_action: Option<&ui::Action>,
    action: &mut Option<ui::Invoke>,
) {
    match view {
        Some(v) => {
            ui.heading(if v.title.is_empty() { name } else { &v.title });
            ui.separator();
            // Scroll the module content so tall views (long tables, big scale)
            // don't overflow the window.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // The in-flight button shows a spinner (via busy_action).
                    if let Some(a) = ui::render_view(ui, v, inputs, busy_action, reveal_at) {
                        *action = Some(a);
                    }

                    // Show the Result pane only when a method returned output.
                    if !output.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(i18n::t("module.result")).strong());
                        let mut text = output;
                        ui.add(
                            egui::TextEdit::multiline(&mut text)
                                .desired_width(f32::INFINITY)
                                .code_editor(),
                        );
                    }
                });
        }
        None => {
            if let Some(err) = view_error {
                ui.heading(name);
                ui.separator();
                // Rendered as Markdown so failure messages can bold the required
                // capability/module (e.g. "requires capability **crowdstrike**").
                ui::markdown(ui, err);
            } else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(i18n::t("detail.loading"));
                });
            }
        }
    }
}
