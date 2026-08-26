//! The pop-up windows the shell itself puts up: the licence, the changelog.

use super::*;

/// The full license text, embedded so it's always in sync with the repo.
pub(crate) const LICENSE_TEXT: &str = include_str!("../../../LICENSE");

/// The same license in Ukrainian — an unofficial translation, carried so a
/// Ukrainian-speaking analyst can read the terms in their own language rather
/// than take them on trust.
///
/// It is *not* the licence. The FSF approves no translation, and this one is
/// a draft (rev. 0.9.2, Andriy Rysin and the linux.org.ua community); the file
/// says so in both languages before its first clause. `LICENSE` in the repo
/// root is the text that governs, which is why the pop-up names the English
/// one whichever language it is showing.
pub(crate) const LICENSE_TEXT_UK: &str = include_str!("../../../resources/licenses/LICENSE.uk.txt");

/// Which text to show: the reader's language if there is one for it, and the
/// English otherwise.
pub(crate) fn license_text(lang: i18n::Lang) -> &'static str {
    match lang {
        i18n::Lang::Uk => LICENSE_TEXT_UK,
        _ => LICENSE_TEXT,
    }
}

/// What changed in the release on offer.
///
/// The same pop-up as the license, for the same reason: it is something you
/// open, read, and dismiss. Inline it was a 240pt scrolling box wedged between
/// the version line and the Update button — too short to read a changelog in,
/// and tall enough to push the button below the fold.
pub(crate) fn changes_dialog(
    ctx: &egui::Context,
    open: bool,
    release: Option<&(String, String)>,
) -> ui::Overlay {
    let opts = ui::OverlayOpts {
        width: 760.0,
        max_height: 560.0,
        title: None,
        close: true,
        ..Default::default()
    };
    ui::overlay(ctx, egui::Id::new("limen_changes"), open, &opts, |ui| {
        let (version, notes) = match release {
            Some((v, n)) => (v.as_str(), n.as_str()),
            // The update can land while the pop-up is open — say so rather than
            // showing an empty box.
            None => {
                ui.label(egui::RichText::new(i18n::t("update.no_changes")).strong());
                return;
            }
        };
        ui::typed_label(
            ui,
            egui::Id::new("changes_heading_type"),
            &i18n::t("update.changes_title").replace("{version}", version),
            &ui::Typed {
                font: egui::FontId::proportional(19.0),
                align: egui::Align::Center,
                ..Default::default()
            },
        );
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);

        // Bounded, so this scroll area cannot grow past the pop-up and leave
        // the outer one to do the scrolling.
        const BODY_H: f32 = 400.0;
        egui::ScrollArea::vertical()
            .max_height(BODY_H)
            .min_scrolled_height(BODY_H)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // A GitHub release body is Markdown, and reads as one.
                ui::markdown(ui, notes.trim());
            });
    })
}

/// The license, as a pop-up rather than a tab.
///
/// It is something you open, read a line of and dismiss — as a tab it stayed
/// open behind you, and the one thing nobody wants two of is a copy of the GPL.
pub(crate) fn license_dialog(ctx: &egui::Context, open: bool) -> ui::Overlay {
    let opts = ui::OverlayOpts {
        width: 760.0,
        max_height: 560.0,
        // No title bar text: the heading inside says which license this is,
        // and "License" above it said the same thing twice.
        title: None,
        close: true,
        ..Default::default()
    };
    ui::overlay(ctx, egui::Id::new("limen_license"), open, &opts, |ui| {
        // Everything types out a letter at a time, and centred: the layout is
        // measured once and revealed by clipping, so a centred line does not
        // slide about as it fills in.
        ui::typed_label(
            ui,
            egui::Id::new("license_heading_type"),
            &i18n::t("license.heading"),
            &ui::Typed {
                font: egui::FontId::proportional(19.0),
                align: egui::Align::Center,
                ..Default::default()
            },
        );
        ui.add_space(4.0);
        ui::typed_label(
            ui,
            egui::Id::new("license_intro_type"),
            &i18n::t("license.intro"),
            &ui::Typed {
                color: ui::color::TEXT_MUTED,
                align: egui::Align::Center,
                per_sec: 260.0,
                ..Default::default()
            },
        );
        // Whose translation this is, said before it is read rather than after.
        // Only in Ukrainian: in English there is nothing to disclaim.
        if i18n::locale() == i18n::Lang::Uk {
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(i18n::t("license.unofficial"))
                        .size(11.5)
                        .color(ui::color::WARNING),
                );
            });
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);

        // The license is not typed out: it is thirty-five thousand characters,
        // and nobody opens a license to watch it arrive. A plain label, so it
        // reads as the document it is and can be copied.
        //
        // Left-aligned, and flush with the rest of the pop-up. The block was
        // centred, which set it adrift: its left edge moved with the longest
        // line in it, so the licence started at a different place from the
        // heading above it and from the Ukrainian text, which wraps to a
        // different width. A document reads from a fixed left margin.
        //
        // The height is fixed, and that is load-bearing. The overlay already
        // puts its content in a scroll area; an unbounded one nested inside it
        // grows without limit, so it is the *outer* one that ends up scrolling
        // and the heading rides up out of the box. Bounded, the content fits,
        // the outer area never scrolls, and only the license moves.
        const BODY_H: f32 = 400.0;
        egui::ScrollArea::vertical()
            .max_height(BODY_H)
            .min_scrolled_height(BODY_H)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(license_text(i18n::locale()))
                            .monospace()
                            .size(11.5)
                            .color(ui::color::TEXT_MUTED),
                    )
                    .selectable(true),
                );
            });
    })
}

/// What a module said as it panicked.
///
/// The SDK catches the unwind and hands the message back as an error, which
/// means the text is a panic payload with a file and a line in it — useful to
/// whoever maintains the module, noise to whoever is using it. So it is here,
/// behind one button, rather than on the screen.
pub(crate) fn panic_dialog(ctx: &egui::Context, open: bool, reason: Option<&Inactive>) -> ui::Overlay {
    let (title, help, text) = match reason {
        Some(r) => (i18n::t("module.panic_title"), i18n::t(r.help_key()), r.detail()),
        None => (String::new(), String::new(), ""),
    };
    let opts = ui::OverlayOpts {
        width: 720.0,
        max_height: 420.0,
        title: Some(title),
        close: true,
        ..Default::default()
    };
    ui::overlay(ctx, egui::Id::new("limen_module_panic"), open, &opts, |ui| {
        ui.label(&help);
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Selectable and monospaced: this text is meant to be copied
                // into a bug report, not read for pleasure.
                let mut body = text;
                ui.add(
                    egui::TextEdit::multiline(&mut body)
                        .desired_width(f32::INFINITY)
                        .code_editor(),
                );
            });
    })
}
