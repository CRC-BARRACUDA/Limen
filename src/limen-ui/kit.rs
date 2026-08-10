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
                spacer(),
                btn("trash", "icon").icon("trash"),
            ]),
            row(vec![
                btn("asks first", "confirmed").confirm("Really?", "the thing"),
                btn("opens a tab", "tab").open_in_tab(),
                btn("dismisses a pop-up", "dismiss").dismiss(),
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
            ])
            .on_activate_here("ui.kit", "activate"),
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
}
