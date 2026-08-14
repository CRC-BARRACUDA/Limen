//! The About page.

use crate::app::*;

pub(crate) fn about_view(ui: &mut egui::Ui, reveal_at: f64) -> bool {
    let muted = ui::color::TEXT_MUTED;
    let mut license_clicked = false;
    let now = ui.input(|i| i.time);
    let animate = ui::animations_enabled();

    // Scroll so nothing (footer, button) is clipped on short windows / high scale.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Push the block toward the vertical middle.
            let top = (ui.available_height() * 0.10).clamp(20.0, 120.0);
            ui.add_space(top);

            ui.vertical_centered(|ui| {
                ui.set_max_width(480.0);

                reveal_item(ui, 0, reveal_at, now, animate, |ui| {
                    // The Limen mark paired with the Barracuda logo — one product.
                    // Allocate the whole row as one block so `vertical_centered`
                    // centers it.
                    let mark = 84.0_f32;
                    let gap = 12.0_f32;
                    let row_w = mark + gap + 1.0 + gap + mark;
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(row_w, mark), egui::Sense::hover());

                    let r1 = egui::Rect::from_min_size(rect.left_top(), egui::vec2(mark, mark));
                    draw_brand(ui.painter(), r1, 1.0, false);

                    let sep_x = rect.left() + mark + gap + 0.5;
                    let half = 26.0_f32;
                    ui.painter().vline(
                        sep_x,
                        (rect.center().y - half)..=(rect.center().y + half),
                        egui::Stroke::new(1.0_f32, ui::with_alpha(ui::color::ACCENT, 0.35)),
                    );

                    let r2 = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - mark, rect.top()),
                        egui::vec2(mark, mark),
                    );
                    draw_barracuda(ui.painter(), r2, ui::color::ACCENT_BRIGHT);
                });

                ui.add_space(14.0);
                reveal_item(ui, 1, reveal_at, now, animate, |ui| {
                    ui.label(egui::RichText::new("LIMEN").size(30.0).strong());
                });
                reveal_item(ui, 2, reveal_at, now, animate, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "v{} · {} {}",
                            env!("CARGO_PKG_VERSION"),
                            env!("LIMEN_GIT_BRANCH"),
                            i18n::t("about.branch"),
                        ))
                        .monospace()
                        .color(muted),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            i18n::t("about.commit"),
                            env!("LIMEN_GIT_COMMIT")
                        ))
                        .monospace()
                        .small()
                        .color(muted),
                    );
                });

                ui.add_space(18.0);
                reveal_item(ui, 3, reveal_at, now, animate, |ui| {
                    ui.label(egui::RichText::new(i18n::t("about.tagline")).size(15.0));
                });

                ui.add_space(18.0);
                reveal_item(ui, 4, reveal_at, now, animate, |ui| {
                    ui.separator();
                    ui.add_space(14.0);
                    // Both buttons share the wider label's width so they match,
                    // stacked (License on top, GitHub below).
                    let btn_font = egui::TextStyle::Button.resolve(ui.style());
                    let lic = i18n::t("about.view_license");
                    let gh = i18n::t("about.github");
                    let max_text = [&lic, &gh]
                        .iter()
                        .map(|l| {
                            ui.fonts(|f| {
                                f.layout_no_wrap(
                                    l.to_uppercase(),
                                    btn_font.clone(),
                                    egui::Color32::WHITE,
                                )
                                .size()
                                .x
                            })
                        })
                        .fold(0.0_f32, f32::max);
                    let bw = egui::vec2(max_text + 32.0, ui.spacing().interact_size.y);
                    if ui::outline_button(ui, &lic, bw).clicked() {
                        license_clicked = true;
                    }
                    ui.add_space(8.0);
                    if ui::outline_button(ui, &gh, bw).clicked() {
                        let url = format!("https://github.com/{}", limen_core::update::APP_REPO);
                        ui.output_mut(|o| o.open_url = Some(egui::OpenUrl::new_tab(url)));
                    }
                });

                ui.add_space(16.0);
                reveal_item(ui, 5, reveal_at, now, animate, |ui| {
                    ui.label(
                        egui::RichText::new(i18n::t("about.created_by"))
                            .small()
                            .color(muted),
                    );
                });
                ui.add_space(12.0);
            });
        });

    license_clicked
}
