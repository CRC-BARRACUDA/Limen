//! The Developer window: the UI kit, the console, the engine's own state.

use crate::app::*;

/// Tabs inside the Developer window.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum DevTab {
    DevMode,
    UiKit,
    Console,
}

/// The Developer tab's "Dev mode" sub-tab: source app/module updates from local
/// directories instead of GitHub, for testing. Session-only (resets on restart).
/// `applied` is set when the source changes so the caller can re-run the checks.
pub(crate) fn dev_mode_view(
    ui: &mut egui::Ui,
    dev_mode_on: &mut bool,
    dev_limen_path: &mut String,
    dev_modules_path: &mut String,
    applied: &mut bool,
) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(i18n::t("dev.dev_mode")).strong());
    ui.label(
        egui::RichText::new(i18n::t("dev.dev_mode_help"))
            .small()
            .color(ui::color::TEXT_MUTED),
    );
    ui.add_space(8.0);
    egui::Grid::new("dev_mode_paths")
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            ui.label(i18n::t("dev.limen_path"));
            ui::text_field(
                ui,
                dev_limen_path,
                &i18n::t("dev.limen_path_hint"),
                320.0,
                false,
            );
            ui.end_row();
            ui.label(i18n::t("dev.modules_path"));
            ui::text_field(
                ui,
                dev_modules_path,
                &i18n::t("dev.modules_path_hint"),
                320.0,
                false,
            );
            ui.end_row();
        });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let label = if *dev_mode_on {
            i18n::t("dev.update_dev_mode")
        } else {
            i18n::t("dev.set_dev_mode")
        };
        if ui::primary_button(ui, &label, egui::Vec2::ZERO).clicked() {
            let as_dir = |s: &str| {
                let t = s.trim();
                (!t.is_empty()).then(|| std::path::PathBuf::from(t))
            };
            limen_core::set_update_dir(as_dir(dev_limen_path));
            limen_registry::set_update_modules_dir(as_dir(dev_modules_path));
            *dev_mode_on = true;
            *applied = true;
        }
        if *dev_mode_on
            && ui::outline_button(ui, &i18n::t("dev.turn_off"), egui::Vec2::ZERO).clicked()
        {
            limen_core::set_update_dir(None);
            limen_registry::set_update_modules_dir(None);
            *dev_mode_on = false;
            *applied = true;
        }
    });

    // GitHub token (persistent) — authenticates module-registry requests, lifting
    // the 60/hr anonymous limit to 5,000/hr and making conditional (304) checks
    // free. Self-contained: the edit buffer lives in egui memory; Save persists it
    // to settings.json and applies it live.
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(6.0);
    ui.label(egui::RichText::new(i18n::t("dev.token_title")).strong());
    ui.label(
        egui::RichText::new(i18n::t("dev.token_help"))
            .small()
            .color(ui::color::TEXT_MUTED),
    );
    ui.add_space(4.0);
    // The validation call hits the network, so it runs on a background thread; the
    // shared slot below carries its result back (`Ok(token)` = verified, save it;
    // `Err(msg)` = rejected). While the slot is present but empty, a spinner shows.
    type TokenProbe = std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>;
    let tok_id = egui::Id::new("dev_github_token_buf");
    let status_id = egui::Id::new("dev_github_token_status");
    let probe_id = egui::Id::new("dev_github_token_probe");
    let mut token = ui.data_mut(|d| {
        d.get_temp::<String>(tok_id).unwrap_or_else(|| {
            limen_core::Config::load()
                .ok()
                .and_then(|c| c.github_token)
                .unwrap_or_default()
        })
    });
    // (message, is_error) — result of the last Save, kept in egui memory.
    let mut status = ui.data_mut(|d| d.get_temp::<(String, bool)>(status_id).unwrap_or_default());

    // Collect the result of an in-flight validation, if any has finished.
    let probe = ui.data_mut(|d| d.get_temp::<TokenProbe>(probe_id));
    let mut testing = false;
    if let Some(p) = probe {
        match p.lock().unwrap().take() {
            Some(Ok(t)) => {
                if let Ok(mut cfg) = limen_core::Config::load() {
                    cfg.github_token = Some(t.clone());
                    let _ = cfg.save();
                }
                limen_registry::set_github_token(Some(t));
                status = (i18n::t("dev.token_saved"), false);
                *applied = true;
                ui.data_mut(|d| d.remove::<TokenProbe>(probe_id));
            }
            Some(Err(e)) => {
                status = (format!("{} {e}", i18n::t("dev.token_not_saved")), true);
                ui.data_mut(|d| d.remove::<TokenProbe>(probe_id));
            }
            None => {
                // Still running — keep the frame animating.
                testing = true;
                ui.ctx().request_repaint();
            }
        }
    }

    ui.horizontal(|ui| {
        ui::text_field(ui, &mut token, &i18n::t("dev.token_hint"), 320.0, true);
        ui.add_enabled_ui(!testing, |ui| {
            if ui::primary_button(ui, &i18n::t("dev.save_token"), egui::Vec2::ZERO).clicked() {
                let t = token.trim().to_string();
                if t.is_empty() {
                    // Clearing needs no test — revert to the unauthenticated default.
                    if let Ok(mut cfg) = limen_core::Config::load() {
                        cfg.github_token = None;
                        let _ = cfg.save();
                    }
                    limen_registry::set_github_token(None);
                    status = (i18n::t("dev.token_cleared"), false);
                    *applied = true;
                } else {
                    // Verify the token grants API access, off-thread, before saving.
                    let slot: TokenProbe = std::sync::Arc::new(std::sync::Mutex::new(None));
                    let sink = slot.clone();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let res = limen_registry::test_github_token(&t).map(|()| t);
                        *sink.lock().unwrap() = Some(res);
                        ctx.request_repaint();
                    });
                    ui.data_mut(|d| d.insert_temp(probe_id, slot));
                    status = (String::new(), false);
                    ui.ctx().request_repaint();
                }
            }
        });
        if testing {
            ui.add(egui::Spinner::new().size(16.0));
            ui.label(
                egui::RichText::new(i18n::t("dev.verifying"))
                    .small()
                    .color(ui::color::TEXT_MUTED),
            );
        }
    });
    if !status.0.is_empty() {
        ui.add_space(2.0);
        let color = if status.1 {
            ui::color::ERROR
        } else {
            ui::color::SUCCESS
        };
        ui.label(egui::RichText::new(&status.0).small().color(color));
    }
    ui.data_mut(|d| {
        d.insert_temp(tok_id, token);
        d.insert_temp(status_id, status);
    });

    if *dev_mode_on {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(i18n::t("dev.dev_mode_on"))
                .small()
                .color(egui::Color32::from_rgb(0xe6, 0x9a, 0x5c)),
        );
    }
}

/// The Developer tab: sub-tabs for docs / UI kit / log console. The content fades
/// in when the tab is shown *and* whenever the sub-tab changes (`reveal_at` /
/// `shown_dev_tab` track that, reset on the same frame as the click).
#[allow(clippy::too_many_arguments)]
pub(crate) fn developer_view(
    ui: &mut egui::Ui,
    dev_tab: &mut DevTab,
    inputs: &mut HashMap<String, String>,
    logs: &std::collections::VecDeque<String>,
    autoscroll: &mut bool,
    dev_mode_on: &mut bool,
    dev_limen_path: &mut String,
    dev_modules_path: &mut String,
    dev_applied: &mut bool,
    revealed_at: &mut Option<f64>,
    shown_dev_tab: &mut DevTab,
) {
    ui.horizontal(|ui| {
        if ui::chip(
            ui,
            &i18n::t("dev.tab_dev_mode"),
            *dev_tab == DevTab::DevMode,
        )
        .clicked()
        {
            *dev_tab = DevTab::DevMode;
        }
        if ui::chip(ui, &i18n::t("dev.tab_ui_kit"), *dev_tab == DevTab::UiKit).clicked() {
            *dev_tab = DevTab::UiKit;
        }
        if ui::chip(ui, &i18n::t("dev.tab_console"), *dev_tab == DevTab::Console).clicked() {
            *dev_tab = DevTab::Console;
        }
    });
    ui.separator();

    // Replay the content reveal when the sub-tab changes (same frame as the click).
    let now = ui.input(|i| i.time);
    if *dev_tab != *shown_dev_tab {
        *revealed_at = Some(now);
        *shown_dev_tab = *dev_tab;
    }
    let reveal_at = revealed_at.unwrap_or(now);
    let animate = ui::animations_enabled();

    reveal_item(ui, 0, reveal_at, now, animate, |ui| match dev_tab {
        DevTab::DevMode => dev_mode_view(
            ui,
            dev_mode_on,
            dev_limen_path,
            dev_modules_path,
            dev_applied,
        ),
        DevTab::UiKit => ui::render_demo_ui(ui, inputs),
        DevTab::Console => dev_console(ui, logs, autoscroll),
    });
}

/// The Developer window's "Console" tab — all host + module log lines.
pub(crate) fn dev_console(
    ui: &mut egui::Ui,
    logs: &std::collections::VecDeque<String>,
    autoscroll: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} {}", logs.len(), i18n::t("dev.lines")))
                .color(ui::color::TEXT_MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui::toggle(ui, autoscroll, &i18n::t("dev.autoscroll"));
        });
    });
    ui.separator();

    egui::Frame::none()
        .fill(ui::color::BG_ELEVATED)
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(*autoscroll)
                .show(ui, |ui| {
                    if logs.is_empty() {
                        ui.label(
                            egui::RichText::new(i18n::t("dev.no_logs"))
                                .color(ui::color::TEXT_MUTED),
                        );
                    }
                    for line in logs {
                        ui.label(egui::RichText::new(line).monospace().size(12.0));
                    }
                });
        });
}

/// Rasterize the common glyphs (Latin + Cyrillic + digits + punctuation) into
/// egui's font atlas by laying them out once. egui rasterizes glyphs lazily on
/// first layout/paint, so the first content-heavy page (the Modules manager,
/// with many names/descriptions — worse with the variable display font) would
/// otherwise rasterize a big batch in one frame and hitch the render thread.
/// Doing it on the first frame, behind the splash, pays that cost up-front.
pub(crate) fn prewarm_fonts(ctx: &egui::Context) {
    // Every glyph the host UI uses — the full Latin + Ukrainian-Cyrillic
    // alphabets, digits, punctuation, and the specific symbols/emoji (`• … ▸ ↗
    // × — · “ ” ⚙ 🛠`). Any glyph/size missing here would first appear on a
    // tab, forcing egui to rebuild + re-upload the whole atlas that frame — the
    // hitch that ate the entrance animation.
    const GLYPHS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 \
        .,:;!?'\"“”-—·•…()[]{}<>%/@#&№×↗▸ ⚙🛠 \
        АБВГҐДЕЄЖЗИІЇЙКЛМНОПРСТУФХЦЧШЩЬЮЯабвгґдеєжзиіїйклмнопрстуфхцчшщьюя";
    // Cover every size the UI renders text at, for both families.
    const SIZES: [f32; 11] = [
        10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 24.0, 30.0,
    ];
    ctx.fonts(|f| {
        for &size in &SIZES {
            let _ = f.layout_no_wrap(
                GLYPHS.to_owned(),
                egui::FontId::proportional(size),
                egui::Color32::WHITE,
            );
            let _ = f.layout_no_wrap(
                GLYPHS.to_owned(),
                egui::FontId::monospace(size),
                egui::Color32::WHITE,
            );
        }
    });
}
