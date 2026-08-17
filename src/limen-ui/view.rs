//! The wire format a module answers with, and the renderer that draws it.

use crate::*;

/// A module-described view.
#[derive(Debug, Clone, Deserialize)]
pub struct View {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub widgets: Vec<Widget>,
    /// When present, the GUI auto-invokes this action once, right after the view
    /// is shown — with no user click. Lets a module chain a multi-step flow
    /// (e.g. a progress checklist that advances on its own): each step returns a
    /// view carrying the next step's `auto`, and the chain ends at a view without
    /// one.
    #[serde(default)]
    pub auto: Option<AutoAction>,
    /// A passing notice to raise when this view arrives — the module reporting
    /// what just happened, rather than finding room for it on the screen.
    ///
    /// Raised once, when the view is accepted, not on every frame that draws
    /// it: a notice is an event, and a redraw is not one.
    #[serde(default)]
    pub notice: Option<Notice>,
    /// Show this view as a pop-up over whatever is already on screen, instead of
    /// replacing it. The value is the pop-up's identity.
    ///
    /// The view underneath stays visible and stops responding, so a module can
    /// put settings or a sub-form in front of the user without losing the screen
    /// they came from. Pop-ups stack: one can open another, and the last one
    /// opened is the live one.
    ///
    /// The identity is what lets a pop-up redraw itself. A view whose id is
    /// already open replaces that pop-up in place; a new id opens over it. Without
    /// it, a form that refreshes after every edit — adding a file to a list, say —
    /// would pile up a new pop-up per keystroke.
    #[serde(default)]
    pub modal: Option<String>,
    /// How wide this pop-up wants to be, in points. The host clamps it to the
    /// window — a module cannot know how much room there is, and a text field
    /// asking for infinite width must never be what decides.
    #[serde(default)]
    pub modal_width: Option<f32>,
    /// How tall it may grow before its contents scroll, in points. Also clamped.
    #[serde(default)]
    pub modal_height: Option<f32>,
}

/// What a module wants said in the corner: how serious, and in what words.
#[derive(Debug, Clone, Deserialize)]
pub struct Notice {
    /// "info", "ok", "warning" or "error". Anything else is read as info — a
    /// misspelt level should still deliver the message.
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub text: String,
}

impl Notice {
    pub fn level(&self) -> crate::toast::Level {
        match self.level.as_str() {
            "ok" | "success" => crate::toast::Level::Ok,
            "warning" | "warn" => crate::toast::Level::Warning,
            "error" => crate::toast::Level::Error,
            _ => crate::toast::Level::Info,
        }
    }
}

/// The capability + method a button invokes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Action {
    pub capability: String,
    pub method: String,
}

/// An action a [`View`] asks the GUI to invoke automatically once it renders.
#[derive(Debug, Clone, Deserialize)]
pub struct AutoAction {
    pub capability: String,
    pub method: String,
    /// Extra params merged into the call.
    #[serde(default)]
    pub args: serde_json::Map<String, Value>,
    /// Put what comes back in a tab of its own, instead of on this screen.
    ///
    /// The chain that polls a long job runs in the tab that started it, and what
    /// it ends with is often not another step but a *result* — a scan report, a
    /// generated document. That belongs beside the screen that produced it
    /// rather than on top of it: the tab that ran the job goes back to being
    /// ready for the next one, and the result keeps its own place.
    #[serde(default)]
    pub open_in_tab: bool,
}

impl AutoAction {
    /// The [`Invoke`] this auto-action dispatches.
    pub fn into_invoke(self) -> Invoke {
        Invoke {
            dismiss: false,
            confirm: None,
            action: Action {
                capability: self.capability,
                method: self.method,
            },
            args: self.args,
            open_in_tab: self.open_in_tab,
        }
    }
}

/// What double-clicking a table row does.
#[derive(Debug, Clone, Deserialize)]
pub struct RowAction {
    pub action: Action,
    /// Open the returned view in a new tab rather than replacing the current one.
    #[serde(default)]
    pub open_in_tab: bool,
}

/// One right-click menu entry on a table row. A leaf carries an `action`; an
/// entry with `children` is a submenu (e.g. the Windows "Open path" submenu).
#[derive(Debug, Clone, Deserialize)]
pub struct MenuItem {
    pub label: String,
    #[serde(default)]
    pub action: Option<Action>,
    /// Extra params merged into the call (e.g. `{"via":"explorer"}`).
    #[serde(default)]
    pub args: serde_json::Map<String, Value>,
    /// Open the result in a new tab instead of replacing the current view.
    #[serde(default)]
    pub open_in_tab: bool,
    /// Ask before running this, exactly as a button can. A destructive action
    /// is no less destructive for being in a menu.
    #[serde(default)]
    pub confirm: Option<Confirm>,
    /// Submenu entries; when present, `action` is ignored and this is a submenu.
    #[serde(default)]
    pub children: Vec<MenuItem>,
}

/// A question the host asks before running an action.
///
/// Carried by the button rather than handled by the module: the module never
/// sees the click unless the answer was yes, so it cannot forget to ask, and
/// every confirmation in the app looks and behaves the same — including the
/// module manager's own "remove this module?".
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Confirm {
    #[serde(default)]
    pub title: String,
    /// What the action happens *to*, drawn in its own frame. The thing at stake
    /// should be unmistakable rather than something to skim past in a sentence.
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub confirm_label: String,
    #[serde(default)]
    pub cancel_label: String,
}

/// A dispatched interaction: an [`Action`] plus any extra params (e.g. the
/// activated row's id) and whether its result view opens in a new tab. Produced
/// by a clicked button, a row context-menu item, or a row double-click.
#[derive(Debug, Clone)]
pub struct Invoke {
    pub action: Action,
    pub args: serde_json::Map<String, Value>,
    pub open_in_tab: bool,
    /// Close the top pop-up instead of calling the module. Set by a button the
    /// module marked `dismiss` — a Cancel that has nothing to cancel remotely,
    /// so it is answered here rather than by a round trip.
    pub dismiss: bool,
    /// Ask this before running it. The host handles the asking.
    pub confirm: Option<Confirm>,
}

/// One bar in a [`Widget::Chart`].
#[derive(Debug, Clone, Deserialize)]
pub struct ChartBar {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub value: f64,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelStyle {
    #[default]
    Normal,
    Heading,
    Strong,
    Weak,
    Mono,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    #[default]
    Default,
    Primary,
    /// Red: the action destroys something. Available to modules too — a module
    /// that deletes, revokes or wipes should be able to say so in the same
    /// language the host uses for its own destructive actions.
    Danger,
}

/// Serde default for `#[serde(default = "default_true")]` bool fields.
pub fn default_true() -> bool {
    true
}

/// One widget in a [`View`]. `kind` is the tag.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Widget {
    Label {
        text: String,
        #[serde(default)]
        style: LabelStyle,
    },
    Text {
        id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        multiline: bool,
        #[serde(default)]
        default: String,
        /// Mask the input (single-line only), for secrets.
        #[serde(default)]
        password: bool,
    },
    /// A date, typed or picked off a calendar. Its value reaches the module in
    /// one canonical form whichever way it was given.
    #[serde(rename = "date")]
    DateField {
        id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        default: String,
        /// Carry a time of day as well as a day.
        #[serde(default)]
        time: bool,
    },
    /// Widgets that belong to the module rather than to the screen: a title, a
    /// version, the picker that chooses which screen you are on.
    ///
    /// They are drawn plainly and take no part in the entrance. The entrance
    /// exists so that swapping one screen for another reads as a change, and
    /// chrome is the part that did *not* change — replaying it makes the header
    /// flinch every time the thing below it is replaced.
    Chrome {
        children: Vec<Widget>,
    },
    Select {
        id: String,
        #[serde(default)]
        label: String,
        options: Vec<String>,
        #[serde(default)]
        default: String,
        /// Invoked as soon as the selection changes, so the module can answer
        /// with a different screen. Without it a dropdown can only record an
        /// answer, never act on one — which is the difference between a form
        /// field and a way of choosing what the form is.
        #[serde(default)]
        on_change: Option<AutoAction>,
    },
    /// A filesystem path input. The user can type a path, drop a file or folder
    /// onto it, or press Browse for the OS picker. Its `id` keys the chosen path
    /// in params, exactly like [`Widget::Text`].
    File {
        id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        default: String,
        /// What this field takes: `"file"`, `"dir"`, or `"file|dir"` for either.
        /// Empty falls back to `directory`, so a module built before this field
        /// existed keeps behaving as it did.
        #[serde(default)]
        accepts: String,
        /// Superseded by `accepts`. Kept so a module compiled against an earlier
        /// SDK still gets a directory picker rather than silently becoming a
        /// file picker.
        #[serde(default)]
        directory: bool,
        /// Label for the Browse button. Supplied by the module so it can be
        /// localized alongside the rest of its view; `ui` stays i18n-free.
        #[serde(default)]
        browse: String,
        /// Label for the *folder* button on a field that takes either kind.
        /// The OS has no "pick a file or a folder" dialog — GTK, the XDG portal
        /// and Windows all treat them as separate — so a dual field offers both.
        /// Left empty, only the file button shows and folders arrive by drag.
        #[serde(default)]
        browse_dir: String,
    },
    /// An animated on/off checkbox; its boolean state is returned in params.
    Checkbox {
        id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        default: bool,
    },
    /// A progress step with an animated status icon — a loading spinner that
    /// morphs into a check when done. `state`: "pending" | "loading" | "done".
    Step {
        #[serde(default)]
        label: String,
        #[serde(default)]
        state: String,
    },
    Button {
        text: String,
        action: Action,
        #[serde(default)]
        style: ButtonStyle,
        /// Whether the button is clickable (default true).
        #[serde(default = "default_true")]
        enabled: bool,
        /// Extra params merged into the call (e.g. a device id/path).
        #[serde(default)]
        args: serde_json::Map<String, Value>,
        /// Open the result view in a new tab instead of replacing this one.
        #[serde(default)]
        open_in_tab: bool,
        /// Close the pop-up this button is in, without calling the module.
        #[serde(default)]
        dismiss: bool,
        /// Ask before running this. The host shows the question and only calls
        /// the module if the answer is yes.
        #[serde(default)]
        confirm: Option<Confirm>,
        /// Draw a painted icon instead of the label, which becomes its tooltip.
        ///
        /// Painted rather than a glyph because the app ships one font and
        /// JetBrains Mono has no trash character — a module asking for "🗑"
        /// would get an empty box on every machine.
        #[serde(default)]
        icon: String,
    },
    Separator,
    /// Inside a [`Widget::Row`], pushes everything after it to the right edge.
    ///
    /// What makes a column of trailing buttons line up when the text before them
    /// does not: without it, each button sits wherever its own row's text ended.
    Spacer,
    Row {
        children: Vec<Widget>,
    },
    /// A table with a header row and string cells. Rows become interactive
    /// (right-click menu + double-click) when `menu` or `on_activate` is set.
    Table {
        #[serde(default)]
        columns: Vec<String>,
        #[serde(default)]
        rows: Vec<Vec<String>>,
        /// Per-row identity (parallel to `rows`), sent as `id` on a row action.
        #[serde(default)]
        row_ids: Vec<String>,
        /// Right-click context-menu items shared by every row.
        #[serde(default)]
        menu: Vec<MenuItem>,
        /// Optional per-row menus (parallel to `rows`); a non-empty entry here
        /// overrides the shared `menu` for that row — so rows can offer
        /// different actions (or none).
        #[serde(default)]
        row_menus: Vec<Vec<MenuItem>>,
        /// What double-clicking a row does.
        #[serde(default)]
        on_activate: Option<RowAction>,
    },
    /// A horizontal bar chart (a value per labelled bar).
    Chart {
        #[serde(default)]
        title: String,
        #[serde(default)]
        data: Vec<ChartBar>,
    },
}

/// Render a view; returns the action of a clicked button, if any. `busy` is the
/// action currently in flight (its button shows a spinner).
/// Draw a module's view.
///
/// `reveal_at` is when this view's entrance begins: top-level widgets fade in
/// one after another from that moment, so switching to a module plays the same
/// staggered entrance the module cards use. Pass `0.0` for "already revealed" —
/// the elapsed time is then effectively infinite and everything draws at full
/// opacity immediately.
pub fn render_view(
    ui: &mut egui::Ui,
    view: &View,
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
    reveal_at: f64,
) -> Option<Invoke> {
    let mut clicked = None;
    let now = ui.input(|i| i.time);

    // A module that swaps one screen for another — Basic to Advanced, settings
    // to results — gets the entrance again, so the change reads as a change
    // rather than the view blinking into a different arrangement. Keyed on the
    // shape, so a view that merely refreshes in place keeps its clock.
    let shape = view_shape(view);
    let id = ui.id().with("limen_view_shape");
    let changed_at = ui.data_mut(|d| {
        let seen = d.get_temp::<(u64, f64)>(id);
        match seen {
            Some((s, at)) if s == shape => at,
            _ => {
                d.insert_temp(id, (shape, now));
                now
            }
        }
    });
    // Whichever entrance is newer: the tab's, or this screen's own.
    let reveal_at = reveal_at.max(changed_at);

    // Publish this entrance so widgets nested deeper — tables, which clock
    // themselves — can join it instead of snapping in.
    ui.data_mut(|d| d.insert_temp(view_reveal_id(), reveal_at));
    for (i, w) in view.widgets.iter().enumerate() {
        // Chrome is not part of the entrance: it was already there.
        if matches!(w, Widget::Chrome { .. }) {
            render_widget(ui, w, inputs, busy, &mut clicked);
            continue;
        }
        let t = reveal_t(ui, i, reveal_at, now, 0.02, 0.13);
        // Slide as well as fade. A fade alone is easy to miss on a screen that
        // swaps for a similar one — Basic to Advanced keeps the same header and
        // target field — so the motion is what reads as "this changed".
        //
        // The offset is a frame margin, not a `horizontal` wrapper: wrapping
        // turns the child into a *row*, which stands separators on end and puts
        // every label beside its field instead of above it.
        let dx = (1.0 - t) * 18.0;
        egui::Frame::none()
            .outer_margin(egui::Margin {
                left: dx,
                ..Default::default()
            })
            .show(ui, |ui| {
                ui.set_opacity(t);
                render_widget(ui, w, inputs, busy, &mut clicked);
            });
    }
    clicked
}

pub fn render_widgets(
    ui: &mut egui::Ui,
    widgets: &[Widget],
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
    clicked: &mut Option<Invoke>,
) {
    for w in widgets {
        render_widget(ui, w, inputs, busy, clicked);
    }
}

pub fn render_widget(
    ui: &mut egui::Ui,
    widget: &Widget,
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
    clicked: &mut Option<Invoke>,
) {
    match widget {
        Widget::Label { text, style } => {
            ui.label(styled(text, *style));
        }
        Widget::Text {
            id,
            label,
            placeholder,
            multiline,
            default,
            password,
        } => {
            if !label.is_empty() {
                ui.label(styled(label, LabelStyle::Weak));
            }
            let value = inputs.entry(id.clone()).or_insert_with(|| default.clone());
            if *multiline {
                ui.add(
                    egui::TextEdit::multiline(value)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .hint_text(placeholder.as_str()),
                );
            } else {
                // Single-line fields — including password/secret ones — get the
                // animated focus border.
                text_field(ui, value, placeholder.as_str(), f32::INFINITY, *password);
            }
        }
        Widget::Select {
            id,
            label,
            options,
            default,
            on_change,
        } => {
            if !label.is_empty() {
                ui.label(styled(label, LabelStyle::Weak));
            }
            let initial = if default.is_empty() {
                options.first().cloned().unwrap_or_default()
            } else {
                default.clone()
            };
            let value = inputs.entry(id.clone()).or_insert(initial);
            let before = value.clone();
            dropdown(ui, id.clone(), value, options);
            if let (Some(a), true) = (on_change, *value != before) {
                let mut args = a.args.clone();
                // The new value goes with the call, so the module does not have
                // to guess which of its dropdowns moved.
                args.insert(id.clone(), Value::String(value.clone()));
                *clicked = Some(Invoke {
                    action: Action {
                        capability: a.capability.clone(),
                        method: a.method.clone(),
                    },
                    args,
                    open_in_tab: false,
                    dismiss: false,
                    confirm: None,
                });
            }
        }
        Widget::File {
            id,
            label,
            placeholder,
            default,
            accepts,
            directory,
            browse,
            browse_dir,
        } => {
            if !label.is_empty() {
                ui.label(styled(label, LabelStyle::Weak));
            }
            let value = inputs.entry(id.clone()).or_insert_with(|| default.clone());
            let mut path = value.clone();
            let browse_label = if browse.is_empty() {
                "Browse…"
            } else {
                browse
            };
            if file_field(
                ui,
                id,
                &mut path,
                placeholder,
                browse_label,
                browse_dir,
                Accepts::of(accepts, *directory),
            ) {
                *value = path;
            }
        }
        Widget::Checkbox { id, label, default } => {
            let entry = inputs
                .entry(id.clone())
                .or_insert_with(|| default.to_string());
            let mut on = entry.as_str() == "true";
            toggle(ui, &mut on, label);
            *entry = on.to_string();
        }
        Widget::Step { label, state } => {
            ui.horizontal(|ui| {
                step_icon(ui, label, state);
                ui.add_space(8.0);
                let s = match state.as_str() {
                    "loading" => LabelStyle::Strong,
                    "done" => LabelStyle::Normal,
                    _ => LabelStyle::Weak,
                };
                ui.label(styled(label, s));
            });
        }
        Widget::Button {
            text,
            action,
            style,
            enabled,
            args,
            open_in_tab,
            dismiss,
            confirm,
            icon,
        } => {
            let running = busy == Some(action);
            ui.horizontal(|ui| {
                // Module buttons use the shared animated widgets, so a module's UI
                // animates just like the host's chrome.
                let resp = ui
                    .add_enabled_ui(*enabled, |ui| {
                        if !icon.is_empty() {
                            // The label becomes the tooltip: an icon with no
                            // name is a guess, and this one deletes things.
                            return icon_button(ui, icon, matches!(style, ButtonStyle::Danger))
                                .on_hover_text(text);
                        }
                        match style {
                            ButtonStyle::Primary => primary_button(ui, text, egui::Vec2::ZERO),
                            ButtonStyle::Danger => danger_button(ui, text, egui::Vec2::ZERO),
                            ButtonStyle::Default => outline_button(ui, text, egui::Vec2::ZERO),
                        }
                    })
                    .inner;
                if resp.clicked() {
                    *clicked = Some(Invoke {
                        action: action.clone(),
                        args: args.clone(),
                        open_in_tab: *open_in_tab,
                        dismiss: *dismiss,
                        confirm: confirm.clone(),
                    });
                }
                if running {
                    ui.add_space(6.0);
                    ui.spinner(); // animates while this action is in flight
                }
            });
        }
        Widget::Separator => {
            ui.separator();
        }
        // Only meaningful inside a Row, which handles it; on its own it is
        // nothing rather than an error.
        Widget::Spacer => {}
        Widget::DateField {
            id,
            label,
            placeholder,
            default,
            time,
        } => {
            if !label.is_empty() {
                ui.label(styled(label, LabelStyle::Weak));
            }
            let value = inputs.entry(id.clone()).or_insert_with(|| default.clone());
            let mut v = value.clone();
            if date_field(ui, id, &mut v, placeholder, *time) {
                *value = v;
            }
        }
        Widget::Chrome { children } => {
            render_widgets(ui, children, inputs, busy, clicked);
        }
        Widget::Row { children } => {
            // Top-align so a row of mixed-height widgets (e.g. buttons) lines up
            // by their tops instead of being vertically centered.
            ui.horizontal_top(|ui| {
                // Anything after a spacer hugs the right edge. Laid out
                // right-to-left, so the tail is rendered in reverse to come out
                // in the order it was written.
                if let Some(at) = children.iter().position(|c| matches!(c, Widget::Spacer)) {
                    let (head, tail) = children.split_at(at);
                    render_widgets(ui, head, inputs, busy, clicked);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for c in tail[1..].iter().rev() {
                            render_widget(ui, c, inputs, busy, clicked);
                        }
                    });
                    return;
                }
                // Path and text fields ask for all the width there is, so the
                // first one in a row takes it and the rest are left as stubs —
                // three thresholds side by side rendered as one wide box and two
                // small ones. Share the row between them instead.
                let greedy = children
                    .iter()
                    .filter(|c| matches!(c, Widget::Text { .. } | Widget::File { .. }))
                    .count();
                if greedy < 2 {
                    render_widgets(ui, children, inputs, busy, clicked);
                    return;
                }
                let gap = ui.spacing().item_spacing.x;
                // What the fields have left once the labels and buttons beside
                // them have taken their share.
                let fixed: f32 = gap * (children.len().saturating_sub(1)) as f32;
                let share = ((ui.available_width() - fixed) / greedy as f32).max(72.0);
                for c in children {
                    if matches!(c, Widget::Text { .. } | Widget::File { .. }) {
                        // A field's label sits *beside* its box here, so it is
                        // centred against it rather than left on its top edge.
                        // The zero desired height is what keeps that honest:
                        // centring inside a region that was given the pop-up's
                        // whole remaining height puts the pair in the middle of
                        // a band of empty space. The row itself stays
                        // top-aligned, which is what a row of buttons wants.
                        ui.allocate_ui_with_layout(
                            egui::vec2(share, 0.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| render_widget(ui, c, inputs, busy, clicked),
                        );
                    } else {
                        render_widget(ui, c, inputs, busy, clicked);
                    }
                }
            });
        }
        Widget::Table {
            columns,
            rows,
            row_ids,
            menu,
            row_menus,
            on_activate,
        } => render_table(
            ui,
            columns,
            rows,
            row_ids,
            menu,
            row_menus,
            on_activate.as_ref(),
            clicked,
        ),
        Widget::Chart { title, data } => render_chart(ui, title, data),
    }
}

/// Render a [`Widget::Chart`] as labelled horizontal bars scaled to the largest
/// value, with the value shown at the end of each row.
pub fn render_chart(ui: &mut egui::Ui, title: &str, data: &[ChartBar]) {
    if !title.is_empty() {
        ui.label(egui::RichText::new(title).strong());
        ui.add_space(2.0);
    }
    if data.is_empty() {
        return;
    }
    let max = data
        .iter()
        .map(|b| b.value)
        .fold(0.0_f64, f64::max)
        .max(f64::MIN_POSITIVE);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let measure = |s: &str| {
        ui.fonts(|f| {
            f.layout_no_wrap(s.to_owned(), font.clone(), color::TEXT)
                .size()
                .x
        })
    };
    let label_w = data
        .iter()
        .map(|b| measure(&b.label))
        .fold(0.0_f32, f32::max)
        .clamp(24.0, 200.0);
    let gap = 8.0;
    let val_w = 52.0;
    let row_h = font.size + 8.0;

    for b in data {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_h),
            egui::Sense::hover(),
        );
        // Label.
        ui.painter().text(
            egui::pos2(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            &b.label,
            font.clone(),
            color::TEXT,
        );
        // Bar track + filled portion.
        let bx = rect.left() + label_w + gap;
        let bar_area = (rect.right() - val_w - gap - bx).max(24.0);
        let bh = row_h * 0.58;
        let track = egui::Rect::from_min_size(
            egui::pos2(bx, rect.center().y - bh / 2.0),
            egui::vec2(bar_area, bh),
        );
        ui.painter().rect_filled(
            track,
            egui::Rounding::ZERO,
            with_alpha(color::BG_WIDGET, 0.55),
        );
        let frac = (b.value / max).clamp(0.0, 1.0) as f32;
        let fill = egui::Rect::from_min_size(track.min, egui::vec2(bar_area * frac, bh));
        ui.painter()
            .rect_filled(fill, egui::Rounding::ZERO, color::ACCENT);
        // Value.
        ui.painter().text(
            egui::pos2(rect.right(), rect.center().y),
            egui::Align2::RIGHT_CENTER,
            fmt_num(b.value),
            font.clone(),
            color::TEXT_MUTED,
        );
    }
}

/// Format a chart value: integers without a decimal, otherwise two places.
pub fn fmt_num(v: f64) -> String {
    if v.fract().abs() < 1e-9 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// A fingerprint of a view's *shape* — its title and the kinds of widget it is
/// made of, not their contents.
///
/// This is what decides whether a view has become a different screen or merely
/// refreshed. Switching a module between Basic and Advanced adds controls and so
/// changes the shape; a progress checklist ticking through its steps swaps a
/// label's text and keeps it, which must not count, or the screen would restart
/// its entrance on every tick and strobe.
pub fn view_shape(view: &View) -> u64 {
    use std::hash::{Hash, Hasher};
    fn walk<H: std::hash::Hasher>(widgets: &[Widget], h: &mut H) {
        widgets.len().hash(h);
        for w in widgets {
            std::mem::discriminant(w).hash(h);
            match w {
                // A button's label is part of the shape. Two screens can be
                // built from the same widgets and still be different screens —
                // a mode switch offering "Scan processes" or "Scan autostart"
                // differs only in what its one button says. Safe to include
                // because a screen that merely refreshes keeps its buttons: a
                // progress view's Stop is Stop on every tick, while the counts
                // beside it change and are deliberately left out.
                Widget::Button { text, .. } => text.hash(h),
                Widget::Row { children } | Widget::Chrome { children } => walk(children, h),
                _ => {}
            }
        }
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    view.title.hash(&mut h);
    walk(&view.widgets, &mut h);
    h.finish()
}

/// Where [`render_view`] parks the current entrance's start time, for widgets
/// that clock themselves rather than taking a stagger index from the top level.
pub fn view_reveal_id() -> egui::Id {
    egui::Id::new("limen_view_reveal")
}

pub fn styled(text: &str, style: LabelStyle) -> egui::RichText {
    let rt = egui::RichText::new(text);
    match style {
        LabelStyle::Normal => rt,
        LabelStyle::Heading => rt.heading(),
        LabelStyle::Strong => rt.strong(),
        LabelStyle::Weak => rt.color(color::TEXT_MUTED),
        LabelStyle::Mono => rt.monospace(),
    }
}

// --------------------------------------------------------------------------- //
// A tiny Markdown renderer — just enough for GitHub release notes (headings,
// bold, inline code, bullet lists, links). Not full CommonMark; anything it
// doesn't recognise degrades to readable text.
// --------------------------------------------------------------------------- //

/// Gather the current values of every input widget into a params object,
/// keyed by widget `id`.
pub fn collect_params(view: &View, inputs: &HashMap<String, String>) -> Value {
    let mut map = serde_json::Map::new();
    collect_ids(&view.widgets, inputs, &mut map);
    Value::Object(map)
}

/// Every input id a view declares.
///
/// A widget's `default` only seeds its entry the first time it is drawn, so a
/// pop-up whose fields are left behind when it closes would reopen showing what
/// was typed and abandoned — a Cancel that cancels nothing. The caller forgets
/// these when the pop-up goes, and the next open takes the module's values again.
pub fn widget_ids(view: &View) -> Vec<String> {
    fn walk(widgets: &[Widget], out: &mut Vec<String>) {
        for w in widgets {
            match w {
                Widget::Text { id, .. }
                | Widget::Select { id, .. }
                | Widget::File { id, .. }
                | Widget::DateField { id, .. }
                | Widget::Checkbox { id, .. } => out.push(id.clone()),
                Widget::Row { children } | Widget::Chrome { children } => walk(children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&view.widgets, &mut out);
    out
}

pub fn collect_ids(
    widgets: &[Widget],
    inputs: &HashMap<String, String>,
    map: &mut serde_json::Map<String, Value>,
) {
    for w in widgets {
        match w {
            Widget::Text { id, .. }
            | Widget::Select { id, .. }
            | Widget::File { id, .. }
            | Widget::DateField { id, .. } => {
                if let Some(v) = inputs.get(id) {
                    map.insert(id.clone(), Value::String(v.clone()));
                }
            }
            Widget::Checkbox { id, default, .. } => {
                let on = inputs.get(id).map(|s| s == "true").unwrap_or(*default);
                map.insert(id.clone(), Value::Bool(on));
            }
            Widget::Row { children } | Widget::Chrome { children } => {
                collect_ids(children, inputs, map)
            }
            _ => {}
        }
    }
}
