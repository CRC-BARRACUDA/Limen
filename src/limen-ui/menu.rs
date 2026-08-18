//! The right-click menu: the window it opens in, and the flyouts under it.
//!
//! It owns its window rather than using egui's `context_menu`, for the reason
//! the date field owns its own: a menu whose contents stop being called the
//! moment it shuts has nowhere left to draw itself leaving. Here one key holds
//! which row's menu is open, so it can be drawn on the frames *after* it was
//! dismissed — fading and shrinking back toward the point it grew from — and a
//! submenu can do the same inside it.

use crate::*;

/// How long the menu takes to arrive, and to leave.
///
/// Quicker than a pop-up window's entrance: a window is somewhere you go, a menu
/// is part of a gesture that is already in progress, and a gesture that waits
/// for an animation is one people stop making.
const MENU_FADE: f32 = 0.10;

/// The narrowest a menu may be. Left to shrink-wrap its labels, a menu of short
/// words ("Open", "About") is a different width on every row it opens over.
const MENU_MIN_W: f32 = 168.0;

/// How far a flyout overlaps the item that owns it, so crossing the gap between
/// the two cannot land on the row underneath and close it.
const FLYOUT_OVERLAP: f32 = 4.0;

/// A table's open row menu.
///
/// Kept in egui memory rather than in the caller, because the table is redrawn
/// from a fresh view every frame and has nowhere of its own to remember this.
#[derive(Clone)]
struct RowMenu {
    /// Which row it belongs to — that row's items are what it shows.
    row: usize,
    /// Where the pointer was: the menu's corner, and the point it grows from.
    at: egui::Pos2,
    /// False once dismissed; the menu is still drawn until it has faded.
    open: bool,
    /// The submenus that have been opened, outermost first. An entry stays until
    /// its panel has finished animating away, so a flyout leaves as it arrived.
    path: Vec<usize>,
    /// How many of those are still open — the rest are on their way out.
    depth: usize,
    /// The frame it opened on. A right-click that opens the menu is also a press
    /// outside the menu that was open a moment ago, and that must not shut the
    /// one it just opened.
    frame: u64,
}

/// Where a table keeps its menu.
fn menu_key(table: egui::Id) -> egui::Id {
    table.with("limen_row_menu")
}

/// The menu a row shows: its own when it has one, the table's otherwise.
pub fn menu_for<'a>(menu: &'a [MenuItem], row_menus: &'a [Vec<MenuItem>], row: usize) -> &'a [MenuItem] {
    match row_menus.get(row) {
        Some(m) if !m.is_empty() => m,
        _ => menu,
    }
}

/// Open `table`'s menu for `row`, at the pointer.
///
/// Called on the right-click itself; drawing it is [`row_menu_layer`]'s job, on
/// this frame and every one after until it has finished closing.
pub fn open_row_menu(ui: &egui::Ui, table: egui::Id, row: usize) {
    let Some(at) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    let frame = ui.ctx().frame_nr();
    ui.data_mut(|d| {
        d.insert_temp(
            menu_key(table),
            RowMenu {
                row,
                at,
                open: true,
                path: Vec::new(),
                depth: 0,
                frame,
            },
        )
    });
}

/// Draw `table`'s row menu, if one is open or still closing, and report what was
/// chosen into `clicked`.
///
/// Call it once per table, after its rows: the menu stands over them, so it is
/// drawn after them and asks to be the topmost thing on its layer.
pub fn row_menu_layer(
    ui: &mut egui::Ui,
    table: egui::Id,
    menu: &[MenuItem],
    row_menus: &[Vec<MenuItem>],
    row_ids: &[String],
    clicked: &mut Option<Invoke>,
) {
    let key = menu_key(table);
    let Some(mut state) = ui.data(|d| d.get_temp::<RowMenu>(key)) else {
        return;
    };
    let t = smoothstep(anim_bool(ui, key.with("t"), state.open, MENU_FADE));
    // Gone, and the last frame of it has been drawn: forget it, so the next
    // right-click opens a menu with no memory of this one's flyouts.
    if t <= 0.002 {
        ui.data_mut(|d| d.remove::<RowMenu>(key));
        return;
    }

    let ctx = ui.ctx().clone();
    let row_id = row_ids.get(state.row).cloned().unwrap_or_default();
    let mut items = menu_for(menu, row_menus, state.row);

    // Every panel currently on screen, so a press can be told inside from out.
    let mut rects: Vec<egui::Rect> = Vec::new();
    let mut picked: Option<Invoke> = None;
    // Where the pointer is: on an item that owns a submenu, or on one that does
    // not — which is how a flyout knows to open, and its neighbours to close.
    let mut hovered: Option<(usize, usize)> = None;
    let mut on_leaf: Option<usize> = None;

    let mut at = state.at;
    for depth in 0..=state.path.len() {
        // A flyout arrives and leaves on its own clock, inside the root's — so
        // dismissing the whole menu takes its open submenus with it.
        let on = state.open && (depth == 0 || depth <= state.depth);
        let pt = match depth {
            0 => t,
            _ => t * smoothstep(anim_bool_ctx(&ctx, key.with("open").with(depth), on, MENU_FADE)),
        };
        if pt <= 0.002 {
            // This flyout has finished leaving; the item that owned it is no
            // longer part of what is open.
            state.path.truncate(depth.saturating_sub(1));
            break;
        }

        let panel = menu_panel(&ctx, key, depth, at, items, &row_id, pt, state.path.get(depth).copied());
        rects.push(panel.rect);
        if panel.picked.is_some() {
            picked = panel.picked;
        }
        if let Some(i) = panel.hovered_child {
            hovered = Some((depth, i));
        }
        if panel.on_leaf {
            on_leaf = Some(depth);
        }

        // Down into the flyout that is open here, if any.
        let Some(&i) = state.path.get(depth) else {
            break;
        };
        let Some(item) = items.get(i) else {
            state.path.truncate(depth);
            break;
        };
        at = panel.next_at.unwrap_or(at);
        items = &item.children;
    }

    // The pointer decides what is open: an item with children opens its flyout,
    // one without closes whatever was hanging off a deeper level.
    if let Some((d, i)) = hovered {
        if state.path.get(d) != Some(&i) {
            state.path.truncate(d);
            state.path.push(i);
        }
        state.depth = d + 1;
    } else if let Some(d) = on_leaf {
        state.depth = state.depth.min(d);
    }

    // Dismissed by Escape, by choosing something, or by a press anywhere that is
    // none of the panels.
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.open = false;
    }
    let pressed = ctx.input(|i| i.pointer.any_pressed());
    if pressed && ctx.frame_nr() != state.frame {
        let outside = ctx
            .pointer_interact_pos()
            .is_some_and(|p| !rects.iter().any(|r| r.contains(p)));
        if outside {
            state.open = false;
        }
    }
    if picked.is_some() {
        *clicked = picked;
        state.open = false;
    }
    ui.data_mut(|d| d.insert_temp(key, state));
}

/// What one panel of a menu reported.
struct Panel {
    /// Where it ended up — it may have been nudged to stay on screen.
    rect: egui::Rect,
    /// Where a flyout hanging off this panel's open item belongs.
    next_at: Option<egui::Pos2>,
    /// An item with children that the pointer is on.
    hovered_child: Option<usize>,
    /// The pointer is on an item without children, so nothing deeper is wanted.
    on_leaf: bool,
    picked: Option<Invoke>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            // Filled in with the area's own rect once it has been laid out.
            rect: egui::Rect::NOTHING,
            next_at: None,
            hovered_child: None,
            on_leaf: false,
            picked: None,
        }
    }
}

/// One panel of the menu: the root at the pointer, or a flyout beside the item
/// that owns it.
///
/// `anchor_of` is the item whose flyout is open here, if any — the caller needs
/// where it sits to hang the next panel off it.
#[allow(clippy::too_many_arguments)]
fn menu_panel(
    ctx: &egui::Context,
    key: egui::Id,
    depth: usize,
    at: egui::Pos2,
    items: &[MenuItem],
    row_id: &str,
    t: f32,
    anchor_of: Option<usize>,
) -> Panel {
    let mut out = Panel::default();
    let shown = egui::Area::new(key.with("panel").with(depth))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        // Kept on screen: a menu opened by a right-click near an edge would
        // otherwise hang half of itself past it.
        .constrain(true)
        .show(ctx, |ui| {
            ui.set_opacity(t);
            egui::Frame::none()
                .fill(color::BG_ELEVATED)
                .stroke(egui::Stroke::new(1.0_f32, color::BORDER))
                .inner_margin(egui::Margin::same(4.0))
                .show(ui, |ui| {
                    // Sized to its widest label, because an Area is unbounded:
                    // asked for the width it has available, an item would run
                    // to the far edge of the window.
                    ui.set_width(panel_width(ui, items));
                    for (i, item) in items.iter().enumerate() {
                        let resp = menu_row(ui, item, t, i, items.len());
                        if Some(i) == anchor_of {
                            out.next_at = Some(
                                resp.rect.right_top() + egui::vec2(FLYOUT_OVERLAP, -6.0),
                            );
                        }
                        if resp.hovered() {
                            if item.children.is_empty() {
                                out.on_leaf = true;
                            } else {
                                out.hovered_child = Some(i);
                            }
                        }
                        if resp.clicked()
                            && item.children.is_empty()
                            && let Some(action) = &item.action
                        {
                            let mut args = item.args.clone();
                            args.insert("id".into(), Value::String(row_id.to_string()));
                            out.picked = Some(Invoke {
                                dismiss: false,
                                // A menu entry can carry a question just as a
                                // button can.
                                confirm: item.confirm.clone(),
                                action: action.clone(),
                                args,
                                open_in_tab: item.open_in_tab,
                            });
                        }
                    }
                });
        });
    // Above everything else on its layer — including the pop-up window a table
    // may be standing in, which is on the same one.
    ctx.move_to_top(shown.response.layer_id);
    out.rect = shown.response.rect;
    // Grows from the corner it opened at, which for the root is where the
    // pointer was.
    pop_layer_about(ctx, shown.response.layer_id, out.rect.min, t);
    out
}

/// How wide a panel has to be for its widest label, and never below
/// [`MENU_MIN_W`].
fn panel_width(ui: &egui::Ui, items: &[MenuItem]) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let widest = items
        .iter()
        .map(|item| {
            let w = ui.fonts(|f| {
                f.layout_no_wrap(item.label.clone(), font.clone(), color::TEXT)
                    .size()
                    .x
            });
            // Room for the arrow, on the items that have one.
            w + if item.children.is_empty() { 0.0 } else { 18.0 }
        })
        .fold(0.0_f32, f32::max);
    (widest + 24.0).max(MENU_MIN_W)
}

/// One entry: a choice that highlights under the pointer, a submenu with its
/// arrow, or — an item with neither action nor children — a plain heading.
///
/// The entries fade in one after another, the same stagger the dropdown's
/// options use, so a menu unrolls rather than appearing all at once.
fn menu_row(ui: &mut egui::Ui, item: &MenuItem, t: f32, i: usize, n: usize) -> egui::Response {
    let step = ((t * 1.5) - i as f32 * (0.6 / n.max(1) as f32)).clamp(0.0, 1.0);
    ui.scope(|ui| {
        ui.multiply_opacity(step);
        if item.children.is_empty() && item.action.is_none() {
            let text = egui::RichText::new(&item.label).color(color::TEXT_MUTED);
            ui.add(egui::Label::new(text).selectable(false))
        } else {
            let resp = dropdown_option(ui, &item.label, false);
            if !item.children.is_empty() {
                submenu_arrow(ui, resp.rect);
            }
            resp
        }
    })
    .inner
}

/// The `›` on an entry that opens a flyout, drawn rather than written so it is
/// the same weight as the chevron on a dropdown.
fn submenu_arrow(ui: &egui::Ui, rect: egui::Rect) {
    let x = rect.right() - 12.0;
    let y = rect.center().y;
    let half = 3.5;
    let stroke = egui::Stroke::new(1.6_f32, color::TEXT_MUTED);
    let painter = ui.painter();
    painter.line_segment(
        [egui::pos2(x - half * 0.6, y - half), egui::pos2(x + half * 0.6, y)],
        stroke,
    );
    painter.line_segment(
        [egui::pos2(x + half * 0.6, y), egui::pos2(x - half * 0.6, y + half)],
        stroke,
    );
}
