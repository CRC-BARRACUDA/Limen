//! The UI Kit gallery: every widget, as a module would ask for it.

use crate::*;

/// Every widget a module can ask for, built the way a module builds one.
///
/// Through the SDK's own builders, not by hand and not as a JSON literal. A
/// module is a separate process that sends a view over the wire; building this
/// gallery any other way would show what the renderer can draw rather than what
/// a module can ask for, which are not the same thing and have drifted apart
/// before. Built this way the two are checked against each other every time the
/// page is opened - and a widget the SDK cannot express is a widget nobody can
/// use, however well the engine renders it.
pub fn demo_view() -> Value {
    use limen_sdk_rust::json;
    use limen_sdk_rust::ui::*;

    // Every button in the gallery answers to the same made-up capability.
    let btn = |text: &str, method: &str| button(text, "ui.kit", method);

    window(
        "",
        vec![
            chrome(vec![label(
                "chrome — its children take no part in the entrance animation, \
                 for the parts of a screen that do not change",
            )
            .weak()]),
            separator(),
            label("label — heading").heading(),
            label("label — normal, the default"),
            label("label — strong").strong(),
            label("label — weak").weak(),
            label("label — mono").mono(),
            separator(),
            text("demo.text")
                .label("text")
                .placeholder("one line")
                .default("editable"),
            text("demo.multi")
                .label("text — multiline")
                .placeholder("several lines")
                .multiline(),
            text("demo.secret")
                .label("text — password")
                .placeholder("hidden as you type")
                .password(),
            date("demo.date")
                .label("date — with a time")
                .placeholder("2026-08-10T00:00:00")
                .with_time(),
            date("demo.day")
                .label("date — a day alone")
                .placeholder("2026-08-10"),
            file("demo.file")
                .label("file — a file or a folder")
                .files_or_dirs()
                .browse("File…")
                .browse_dir("Folder…"),
            select(
                "demo.select",
                vec!["one".into(), "two".into(), "three".into()],
            )
            .label("select")
            .default("one"),
            select("demo.select_live", vec!["alpha".into(), "beta".into()])
                .label("select — calls back on change")
                .default("alpha")
                .on_change("ui.kit", "changed"),
            checkbox("demo.check", "checkbox").checked(),
            separator(),
            label("step — every state").strong(),
            step("pending", "pending"),
            step("loading", "loading"),
            step("done", "done"),
            step("warning", "warning"),
            step("error", "error"),
            separator(),
            label("button — every style, and a spacer pushing the last one right").strong(),
            row(vec![
                btn("primary", "primary").primary(),
                btn("default", "default"),
                btn("danger", "danger").danger(),
                btn("disabled", "disabled").enabled(false),
                // A button can carry arguments to the method it calls.
                btn("carries args", "with_args").args(json!({ "which": "the third one" })),
                spacer(),
                btn("trash", "icon").icon("trash"),
            ]),
            row(vec![
                btn("asks first", "confirmed").confirm("Really?", "the thing"),
                btn("opens a tab", "tab").open_in_tab(),
                btn("open a pop-up", "popup"),
            ]),
            label("row — a text field shares its width with its neighbours").weak(),
            row(vec![
                text("demo.row_a").label("from").placeholder("0"),
                text("demo.row_b").label("to").placeholder("100"),
            ]),
            separator(),
            label("table — double-click a row, or right-click it").strong(),
            table(
                vec!["Level".into(), "Rule".into(), "Artefact".into()],
                vec![
                    vec![
                        "critical".into(),
                        "Mimikatz command line".into(),
                        "Security.evtx".into(),
                    ],
                    vec![
                        "high".into(),
                        "PsExec service install".into(),
                        "System.evtx".into(),
                    ],
                    vec![
                        "medium".into(),
                        "Suspicious PowerShell".into(),
                        "Microsoft-Windows-PowerShell%4Operational.evtx".into(),
                    ],
                ],
            )
            .row_ids(vec!["0".into(), "1".into(), "2".into()])
            .row_menu(vec![
                menu_item("Open", "ui.kit", "open"),
                menu_item("Open in a tab", "ui.kit", "open").open_in_tab(),
                submenu(
                    "More ▸",
                    vec![
                        menu_item("Copy the rule", "ui.kit", "copy"),
                        menu_item("Copy the artefact", "ui.kit", "copy"),
                    ],
                ),
            ])
            // Per-row menus override the shared one, so a row can offer
            // something different — or, as here, nothing at all.
            .row_menus(vec![
                vec![],
                vec![menu_item("Only this row has this", "ui.kit", "row")],
                vec![],
            ])
            .on_activate_here("ui.kit", "activate"),
            separator(),
            label("toast — a passing notice, top-right; click one to dismiss it early")
                .strong(),
            row(vec![
                btn("info", "toast.info"),
                btn("ok", "toast.ok"),
                btn("warning", "toast.warning"),
                btn("error", "toast.error"),
            ]),
            separator(),
            chart(
                "chart",
                vec![
                    ("critical".into(), 3.0),
                    ("high".into(), 11.0),
                    ("medium".into(), 27.0),
                    ("low".into(), 8.0),
                ],
            ),
        ],
    )
}

/// Where the gallery keeps what it is showing: a pop-up, a question.
fn demo_state() -> egui::Id {
    egui::Id::new("limen_kit_demo")
}

/// Carry out what a gallery button asked for.
///
/// The toast buttons raise a toast; the confirm button asks; the pop-up button
/// opens one, and the dismiss button inside it closes it. Opening a tab is the
/// one that cannot be shown here — the gallery is not a module, and has no view
/// of its own to send anywhere — so it says so rather than doing nothing.
fn demo_action(ctx: &egui::Context, inv: &Invoke) {
    let say = |level, text: &str| toast::notify(ctx, level, text);
    match inv.action.method.as_str() {
        "toast.info" => say(toast::Level::Info, "An info notice, raised from the UI Kit."),
        "toast.ok" => say(toast::Level::Ok, "An ok notice, raised from the UI Kit."),
        "toast.warning" => say(toast::Level::Warning, "A warning notice, raised from the UI Kit."),
        "toast.error" => say(toast::Level::Error, "An error notice, raised from the UI Kit."),
        "tab" => say(
            toast::Level::Info,
            "A module's view would open in a tab of its own. The gallery has no \
             view to send, so this is only a description.",
        ),
        "popup" => ctx.data_mut(|d| d.insert_temp(demo_state(), true)),
        // A button marked `confirm` is answered by the host: the question goes
        // up, and the module is called only if the answer is yes.
        _ if inv.confirm.is_some() => {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("limen_kit_ask"), true))
        }
        // `dismiss` is answered by the host before the module ever hears about
        // it, so a button marked with it closes the pop-up it is in.
        "dismiss" => ctx.data_mut(|d| d.insert_temp(demo_state(), false)),
        _ => {}
    }
}

/// The pop-up the gallery opens, and the confirm question it asks.
///
/// Drawn after the page, so it sits over it.
fn demo_overlays(ctx: &egui::Context) {
    let open: bool = ctx.data(|d| d.get_temp(demo_state()).unwrap_or(false));
    let opts = OverlayOpts {
        width: 420.0,
        title: Some("A pop-up".into()),
        close: true,
        ..Default::default()
    };
    let out = overlay(ctx, egui::Id::new("limen_kit_popup"), open, &opts, |ui| {
        ui.label(styled(
            "Anything a module can draw, it can draw in here. The button below \
             carries `dismiss`, which the host answers itself — the module is \
             never called.",
            LabelStyle::Weak,
        ));
        ui.add_space(10.0);
        if primary_button(ui, "Dismiss", egui::Vec2::ZERO).clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(demo_state(), false));
        }
    });
    if out.close || out.back {
        ctx.data_mut(|d| d.insert_temp(demo_state(), false));
    }

    // And the question a button marked `confirm` puts up before it runs.
    let asking: bool = ctx.data(|d| d.get_temp(egui::Id::new("limen_kit_ask")).unwrap_or(false));
    if let Some(yes) = confirm_dialog(
        ctx,
        "kit",
        asking,
        "Really?",
        Some("the thing"),
        "Do it",
        "Cancel",
        None,
    ) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("limen_kit_ask"), false));
        toast::notify(
            ctx,
            if yes { toast::Level::Ok } else { toast::Level::Info },
            if yes { "Answered yes." } else { "Answered no." },
        );
    }
}

/// The component gallery — the UI Kit shown in the Developer window, and the
/// standardized source of truth for module widget styling.
pub fn render_demo_ui(ui: &mut egui::Ui, inputs: &mut HashMap<String, String>) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.heading("UI Kit");
            ui.label(styled(
                "Every widget a module can ask for, drawn by the same code that draws a \
                 module's screen — so this is what a module actually gets, not an imitation \
                 of it.",
                LabelStyle::Weak,
            ));

            // What was last pressed, so the interactive parts can be seen to do
            // something. Written after the view is drawn and read on the frame
            // after, which is honest about when it happened.
            let last = inputs.get("demo.last").cloned().unwrap_or_default();
            if !last.is_empty() {
                ui.label(styled(&format!("last action: {last}"), LabelStyle::Mono));
            }
            ui.separator();

            match serde_json::from_value::<View>(demo_view()) {
                Ok(view) => {
                    if let Some(inv) = render_view(ui, &view, inputs, None, 0.0) {
                        // Every button here does the thing it is showing. A
                        // gallery that displays a feature without performing it
                        // is a picture of the feature, which is what this page
                        // exists to stop being.
                        demo_action(ui.ctx(), &inv);
                        let args = if inv.args.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", Value::Object(inv.args.clone()))
                        };
                        inputs.insert(
                            "demo.last".into(),
                            format!("{}.{}{args}", inv.action.capability, inv.action.method),
                        );
                    }
                }
                // The gallery is built from a literal, so this can only mean the
                // literal and the widgets have drifted apart — worth saying out
                // loud rather than rendering nothing.
                Err(e) => {
                    ui.label(styled(
                        &format!("the UI Kit view does not parse: {e}"),
                        LabelStyle::Strong,
                    ));
                }
            }

            ui.add_space(14.0);
            ui.separator();
            ui.label(styled("Palette", LabelStyle::Strong));
            ui.horizontal(|ui| {
                for (name, c) in [
                    ("bg", color::BG),
                    ("elevated", color::BG_ELEVATED),
                    ("widget", color::BG_WIDGET),
                    ("border", color::BORDER),
                    ("text", color::TEXT),
                    ("muted", color::TEXT_MUTED),
                    ("accent", color::ACCENT),
                    ("bright", color::ACCENT_BRIGHT),
                    ("on accent", color::ON_ACCENT),
                ] {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(56.0, 34.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, egui::Rounding::ZERO, c);
                    ui.painter().text(
                        rect.center_bottom() + egui::vec2(0.0, -2.0),
                        egui::Align2::CENTER_BOTTOM,
                        name,
                        egui::FontId::proportional(10.0),
                        color::TEXT,
                    );
                }
            });
        });

    // The pop-up and the question those buttons open, drawn over the page.
    demo_overlays(ui.ctx());
}
