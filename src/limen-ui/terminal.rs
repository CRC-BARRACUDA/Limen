//! A terminal: what has been run and what it printed, and the prompt to type
//! the next command at — in one box, as an SSH session is.
//!
//! The module owns the transcript (`lines`) and the prompt; Limen owns the line
//! being typed. Enter sends it to the module and empties it at once, as a shell
//! does, and the module answers with the transcript grown. While `busy`, the
//! prompt is not shown — a shell running a command takes no new one — and it
//! comes back, with the cursor in it, when the command is done.
//!
//! Ctrl+ and Ctrl− make the terminal's text bigger and smaller, and Ctrl+0
//! puts it back, while the terminal has the keyboard: those keys zoom the whole
//! window everywhere else, and here they are taken before that happens.

use crate::*;

const DEFAULT_ROWS: u32 = 20;
/// The text size a terminal starts at, and how far Ctrl+ and Ctrl− may take it.
const FONT_SIZE: f32 = 13.0;
const FONT_MIN: f32 = 8.0;
const FONT_MAX: f32 = 32.0;

/// Where a terminal's text size is kept: by its id, so it survives the module
/// redrawing the screen, and every command run.
pub fn terminal_font_id(id: &str) -> egui::Id {
    egui::Id::new(("limen_terminal_font", id))
}

/// Draw a [`Widget::Terminal`]. Anything else is ignored.
pub(crate) fn render(
    ui: &mut egui::Ui,
    widget: &Widget,
    inputs: &mut HashMap<String, String>,
    clicked: &mut Option<Invoke>,
) {
    let Widget::Terminal {
        id,
        lines,
        prompt,
        on_submit,
        rows,
        busy,
        fill,
    } = widget
    else {
        return;
    };
    let ctx = ui.ctx().clone();
    let input_id = egui::Id::new(("limen_terminal_input", id.as_str()));
    let size_key = terminal_font_id(id);
    let mut size: f32 = ctx.data(|d| d.get_temp(size_key)).unwrap_or(FONT_SIZE);

    // The keyboard's zoom keys, while the terminal has the keyboard — taken
    // before egui's own end-of-frame zoom of the whole window can see them.
    if ctx.memory(|m| m.has_focus(input_id)) {
        use egui::gui_zoom::kb_shortcuts::*;
        let was = size;
        ctx.input_mut(|i| {
            if i.consume_shortcut(&ZOOM_RESET) {
                size = FONT_SIZE;
            }
            if i.consume_shortcut(&ZOOM_IN) || i.consume_shortcut(&ZOOM_IN_SECONDARY) {
                size = (size + 1.0).min(FONT_MAX);
            }
            if i.consume_shortcut(&ZOOM_OUT) {
                size = (size - 1.0).max(FONT_MIN);
            }
        });
        if size != was {
            ctx.data_mut(|d| d.insert_temp(size_key, size));
        }
    }
    let font = egui::FontId::monospace(size);
    // The box is sized from the text it starts with, not the text it has now:
    // Ctrl+ and Ctrl− change how much fits in it, as in a terminal window,
    // not how much of the screen it takes.
    let row_h = ui.fonts(|f| f.row_height(&egui::FontId::monospace(FONT_SIZE)));
    let rows = if *rows == 0 { DEFAULT_ROWS } else { *rows } as f32;
    // What is left of the window below the box, less the box's own margin and
    // border — so it ends at the bottom edge rather than a line short of it or
    // a scrollbar past it. Never less than a few lines, and only where there
    // is a bottom to reach: on an unbounded page, the rows it was given.
    let left = ui.available_height() - 2.0 * (8.0 + 1.0) - 2.0;
    let height = if *fill && left.is_finite() {
        left.max(row_h * 4.0)
    } else {
        row_h * rows
    };

    // A command finished: the prompt is back, and so is the cursor.
    let busy_key = egui::Id::new(("limen_terminal_busy", id.as_str()));
    let was_busy: Option<bool> = ctx.data(|d| d.get_temp(busy_key));
    ctx.data_mut(|d| d.insert_temp(busy_key, *busy));
    let take_focus = !*busy && was_busy != Some(false);

    let frame = egui::Frame::none()
        .fill(color::BG_ELEVATED)
        .stroke(egui::Stroke::new(1.0_f32, color::BORDER))
        .inner_margin(egui::Margin::same(8.0));
    let shown = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical()
            .id_source(("limen_terminal", id.as_str()))
            .max_height(height)
            .min_scrolled_height(height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if !lines.is_empty() {
                    // Through a `&str`, which egui knows it cannot edit: the
                    // transcript can be selected and copied, and nothing typed
                    // over it changes it.
                    let mut text: &str = lines.trim_end_matches('\n');
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .frame(false)
                            .margin(egui::Margin::ZERO)
                            .font(font.clone())
                            .desired_rows(1)
                            .desired_width(f32::INFINITY),
                    );
                }
                if *busy {
                    return;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if !prompt.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("{prompt} "))
                                .font(font.clone())
                                .color(color::ACCENT),
                        );
                    }
                    let line = inputs.entry(id.clone()).or_default();
                    let resp = ui.add(
                        egui::TextEdit::singleline(line)
                            .id(input_id)
                            .frame(false)
                            .margin(egui::Margin::ZERO)
                            .font(font.clone())
                            .desired_width(f32::INFINITY),
                    );
                    // Enter: the line goes to the module and empties at once.
                    // egui lets go of a single-line field on Enter; the
                    // terminal keeps the keyboard, ready for the next command.
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let command = std::mem::take(line);
                        resp.request_focus();
                        if !command.trim().is_empty() {
                            let mut args = on_submit.args.clone();
                            args.insert(id.clone(), Value::String(command));
                            *clicked = Some(Invoke {
                                action: Action {
                                    capability: on_submit.capability.clone(),
                                    method: on_submit.method.clone(),
                                },
                                args,
                                open_in_tab: false,
                                dismiss: false,
                                confirm: None,
                            });
                        }
                    }
                    if take_focus {
                        resp.request_focus();
                    }
                });
            });
    });
    // A click anywhere in the box puts the cursor at the prompt, as in a
    // terminal window. Not over the transcript's text, which is for selecting.
    if !*busy && shown.response.interact(egui::Sense::click()).clicked() {
        ctx.memory_mut(|m| m.request_focus(input_id));
    }
}
