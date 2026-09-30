//! Diagrams: boxes and the lines between them.
//!
//! The companion of [`render_chart`](crate::render_chart). A chart answers "how
//! much of each"; a diagram answers "what is connected to what" — a network's
//! hosts around their gateway, a dependency between modules, a chain of
//! processes.
//!
//! Drawn with the painter, here, rather than handed to a JavaScript graph
//! library in an exported page. A module that writes HTML pulling a library off
//! a CDN produces a report that is blank on the machine Limen is built for: a
//! portable tool on somebody else's network, with no route out of it.
//!
//! The layout is **deterministic** — no physics, no random seed, no settling.
//! The same nodes come out in the same places every frame and in every export,
//! which is what lets two people compare the same map, and what stops a
//! diagram rearranging itself while it is being read.

use crate::*;

/// One box.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DiagramNode {
    pub id: String,
    #[serde(default)]
    pub label: String,
    /// What it is, which decides its colour: `router`, `server`, `switch`,
    /// `pc`, `self`, or anything else for the neutral one.
    #[serde(default)]
    pub kind: String,
    /// A second line under the label — an address, a version, a count.
    #[serde(default)]
    pub detail: String,
    /// What the circle cannot say, shown when the pointer is on it: the rest of
    /// what the module knows about this host, as named values.
    #[serde(default)]
    pub info: Vec<DiagramFact>,
}

/// One named value on a node's hover card.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DiagramFact {
    pub name: String,
    pub value: String,
}

/// One line between two boxes.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DiagramEdge {
    pub from: String,
    pub to: String,
    /// Drawn as a dashed line: a relationship that is inferred rather than
    /// observed.
    #[serde(default)]
    pub dashed: bool,
}

/// The room a node takes up: the circle is small, but its label is not, and it
/// is the label that decides whether two nodes collide.
const NODE_W: f32 = 132.0;
const NODE_H: f32 = 58.0;
/// The circle itself.
const NODE_R: f32 = 13.0;
/// Room around the hub before the first ring, and between rings after it.
const RING: f32 = 104.0;

/// How far in a map may be zoomed, and how far back out.
const MIN_ZOOM: f32 = 0.4;
const MAX_ZOOM: f32 = 4.0;

/// Where a map has been dragged to, and how far into it the reader has gone.
///
/// A map is drawn to fit the width it is given, which on a big network means
/// drawn small. Rather than grow the page under everything else, the window
/// stays the size it was and the map moves inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Viewport {
    pan: egui::Vec2,
    zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

/// Where a node sits, in the diagram's own space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub x: f32,
    pub y: f32,
}

/// Lay the nodes out: hubs in the middle, everything else on a ring around the
/// hub it is joined to.
///
/// A network map is a star far more often than it is anything else — every
/// machine pointing at one gateway — so the layout is the one that draws a star
/// well and degrades into concentric rings when it is given something else.
///
/// Returns a position per node, in the same order as `nodes`.
pub fn layout(nodes: &[DiagramNode], edges: &[DiagramEdge]) -> Vec<Placed> {
    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let index = |id: &str| nodes.iter().position(|node| node.id == id);

    // The hub is what the most lines lead to. A tie is broken by the order the
    // module sent them, so the same map lays out the same way twice.
    let mut degree = vec![0usize; n];
    for e in edges {
        if let Some(i) = index(&e.to) {
            degree[i] += 1;
        }
        if let Some(i) = index(&e.from) {
            degree[i] += 1;
        }
    }
    let hub = (0..n).max_by_key(|i| (degree[*i], std::cmp::Reverse(*i)));
    let Some(hub) = hub.filter(|_| n > 1) else {
        return vec![Placed { x: 0.0, y: 0.0 }; n];
    };

    // Everything joined to the hub goes on the first ring; everything else on
    // the one outside it, so a node with no edges is still somewhere sensible
    // rather than on top of the middle.
    let joined: Vec<usize> = (0..n)
        .filter(|i| *i != hub)
        .filter(|i| {
            edges.iter().any(|e| {
                (index(&e.from) == Some(*i) && index(&e.to) == Some(hub))
                    || (index(&e.to) == Some(*i) && index(&e.from) == Some(hub))
            })
        })
        .collect();
    let loose: Vec<usize> = (0..n)
        .filter(|i| *i != hub && !joined.contains(i))
        .collect();

    let mut out = vec![Placed { x: 0.0, y: 0.0 }; n];
    for (ring, members) in [(1.0_f32, &joined), (2.0, &loose)] {
        let count = members.len();
        if count == 0 {
            continue;
        }
        // A ring wide enough that the boxes on it do not touch, however many
        // there are.
        let radius = (RING * ring).max(count as f32 * (NODE_W + 16.0) / std::f32::consts::TAU);
        for (k, i) in members.iter().enumerate() {
            // From the top, clockwise: a reader starts at twelve o'clock.
            let angle = -std::f32::consts::FRAC_PI_2
                + k as f32 * std::f32::consts::TAU / count as f32;
            out[*i] = Placed {
                x: radius * angle.cos(),
                y: radius * angle.sin() * 0.72, // flattened: pages are wider than they are tall
            };
        }
    }
    out
}

/// The extent of a laid-out diagram, boxes included.
pub fn bounds(places: &[Placed]) -> (f32, f32, f32, f32) {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in places {
        x0 = x0.min(p.x - NODE_W / 2.0);
        y0 = y0.min(p.y - NODE_H / 2.0);
        x1 = x1.max(p.x + NODE_W / 2.0);
        y1 = y1.max(p.y + NODE_H / 2.0);
    }
    if places.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    (x0, y0, x1, y1)
}

/// The colour a kind is drawn in.
fn kind_colour(kind: &str) -> egui::Color32 {
    match kind {
        "router" | "gateway" => color::ACCENT,
        "server" => egui::Color32::from_rgb(0x5b, 0x9d, 0xd9),
        "switch" => egui::Color32::from_rgb(0xc9, 0xa2, 0x27),
        "self" => color::ACCENT_BRIGHT,
        _ => color::TEXT_MUTED,
    }
}

/// Draw a diagram, fitted to the width it is given.
pub fn render_diagram(
    ui: &mut egui::Ui,
    title: &str,
    nodes: &[DiagramNode],
    edges: &[DiagramEdge],
) {
    render_diagram_in(ui, title, nodes, edges, false, None, &mut None);
}

/// Draw a diagram; with `fill`, in all the height it has rather than only the
/// height it needs.
///
/// A map among other widgets takes the room its own contents ask for — anything
/// else and it would push everything under it off the screen. A map that *is*
/// the screen is given the screen.
/// `on_activate` is what a double click on a circle asks for, carrying that
/// node's id — the same bargain a table's rows make, so a module answers one
/// the way it answers the other.
#[allow(clippy::too_many_arguments)]
pub fn render_diagram_in(
    ui: &mut egui::Ui,
    title: &str,
    nodes: &[DiagramNode],
    edges: &[DiagramEdge],
    fill: bool,
    on_activate: Option<&crate::view::RowAction>,
    clicked: &mut Option<crate::view::Invoke>,
) {
    if !title.is_empty() {
        ui.label(egui::RichText::new(title).strong());
        ui.add_space(4.0);
    }
    if nodes.is_empty() {
        ui.label(egui::RichText::new("—").color(color::TEXT_MUTED));
        return;
    }

    let places = layout(nodes, edges);
    let (x0, y0, x1, y1) = bounds(&places);
    let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
    let middle = egui::vec2((x0 + x1) / 2.0, (y0 + y1) / 2.0);

    // The diagram is scaled to the width it has, never past life size: a map of
    // three machines blown up to fill a window reads as a mistake.
    let avail = ui.available_width().max(120.0);
    // Filling, the map is fitted to both sides of the room it has; otherwise to
    // its width, and it takes only the height that leaves.
    let view_h = if fill {
        ui.available_height().max(160.0)
    } else {
        (h * (avail / w).min(1.0) + 8.0).max(160.0)
    };
    let fit = if fill {
        (avail / w).min((view_h - 8.0) / h).min(1.0)
    } else {
        (avail / w).min(1.0)
    };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(avail, view_h),
        egui::Sense::click_and_drag(),
    );

    // Where the map has been dragged to, and how far it has been zoomed in.
    // Kept against this diagram's own id, so two on one screen move separately
    // and neither forgets where it was between frames.
    let id = ui.make_persistent_id(("diagram", title));
    let mut view: Viewport = ui.data(|d| d.get_temp(id)).unwrap_or_default();

    // Dragging moves the map under the window, which is the whole point of
    // being able to draw it bigger than the room it has.
    if response.dragged() {
        view.pan += response.drag_delta();
    }
    // Pinching, or the wheel with ctrl held. The plain wheel is left alone: it
    // belongs to the page the map is on, and a map that swallowed it would trap
    // the scroll of every view it appears in.
    if response.hovered() {
        let step = ui.input(|i| {
            let pinch = i.zoom_delta();
            if (pinch - 1.0).abs() > f32::EPSILON {
                pinch
            } else if i.modifiers.ctrl || i.modifiers.command {
                1.0 + i.smooth_scroll_delta.y * 0.002
            } else {
                1.0
            }
        });
        if (step - 1.0).abs() > f32::EPSILON {
            let was = view.zoom;
            view.zoom = (view.zoom * step).clamp(MIN_ZOOM, MAX_ZOOM);
            // Zoom about the pointer, so the thing being looked at stays under
            // it rather than sliding away as the map grows.
            if let Some(pos) = response.hover_pos() {
                let grew = view.zoom / was;
                view.pan = (view.pan + (pos - rect.center())) * grew - (pos - rect.center());
            }
        }
    }
    // A double click on a circle asks about that host; on the map itself it is
    // the way back from wherever the map has been dragged to. Two gestures in
    // one place, told apart by what is under the pointer.
    if response.double_clicked() {
        let on = response
            .interact_pointer_pos()
            .and_then(|pos| hit(&places, fit * view.zoom, origin_of(rect, view, middle, fit), pos));
        match (on, on_activate) {
            (Some(i), Some(act)) => {
                let mut args = serde_json::Map::new();
                args.insert("id".into(), serde_json::Value::String(nodes[i].id.clone()));
                *clicked = Some(crate::view::Invoke {
                    dismiss: false,
                    // A double click is not a menu entry; nothing to ask.
                    confirm: None,
                    action: act.action.clone(),
                    args,
                    open_in_tab: act.open_in_tab,
                });
            }
            // Anywhere else — the map itself, or a circle on a map whose module
            // has nothing to say about it — it is the way home.
            _ => view = Viewport::default(),
        }
    }

    let scale = fit * view.zoom;
    // Never let the map be dragged clean out of the window: what is left when
    // nothing is on screen is an empty box and no way to tell why.
    let slack = egui::vec2(
        (w * scale + rect.width()) / 2.0 - 24.0,
        (h * scale + rect.height()) / 2.0 - 24.0,
    );
    view.pan = view.pan.clamp(-slack, slack);
    ui.data_mut(|d| d.insert_temp(id, view));

    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // The screen point of the diagram's own origin, which everything else — the
    // circles, the lines, and the pointer — is measured from.
    let origin = origin_of(rect, view, middle, fit);
    let at = |p: Placed| egui::pos2(origin.x + p.x * scale, origin.y + p.y * scale);

    let ctx = ui.ctx().clone();
    let layer = ui.layer_id();
    // Clipped: a map panned past its edge stops at it, rather than painting over
    // whatever the module put above and below.
    let painter = ui.painter().with_clip_rect(rect);
    // Lines first, so a box is never drawn under one of them.
    for e in edges {
        let (Some(a), Some(b)) = (
            nodes.iter().position(|n| n.id == e.from),
            nodes.iter().position(|n| n.id == e.to),
        ) else {
            continue;
        };
        // Trimmed to the circles' edges: a line that runs under a dot and out
        // the other side reads as two lines crossing.
        let (from, to) = trim(at(places[a]), at(places[b]), NODE_R * scale);
        let stroke = egui::Stroke::new(1.0_f32, with_alpha(color::TEXT_MUTED, 0.55));
        if e.dashed {
            painter.add(egui::Shape::dashed_line(
                &[from, to],
                stroke,
                4.0 * scale,
                4.0 * scale,
            ));
        } else {
            painter.line_segment([from, to], stroke);
        }
    }

    let font = egui::FontId::monospace((11.0 * scale).max(7.0));
    let small = egui::FontId::monospace((9.0 * scale).max(6.0));
    // Not while dragging: the card belongs to the circle under the pointer, and
    // during a drag the pointer is holding the whole map rather than pointing at
    // anything in it.
    let hovered = (!response.dragged())
        .then(|| response.hover_pos())
        .flatten()
        .and_then(|pos| hit(&places, scale, origin, pos));
    for (i, node) in nodes.iter().enumerate() {
        let centre = at(places[i]);
        let colour = kind_colour(&node.kind);
        let on = hovered == Some(i);
        // The circle: solid, in the colour of what it is. Hovering swells it and
        // rings it, so the card that appears is plainly about *this* one.
        let r = NODE_R * scale;
        if on {
            painter.circle_filled(centre, r + 4.0, with_alpha(colour, 0.25));
        }
        painter.circle(
            centre,
            r,
            colour,
            egui::Stroke::new(1.5_f32, if on { color::ACCENT_BRIGHT } else { color::BG }),
        );
        // The label goes outside the circle rather than inside it: a name is
        // longer than a dot is wide, and it stays readable as the map shrinks.
        // It goes on the side away from the middle, so the line joining this
        // node to the hub never runs through its own label.
        let up = places[i].y < 0.0;
        let (align, step) = if up {
            (egui::Align2::CENTER_BOTTOM, -1.0)
        } else {
            (egui::Align2::CENTER_TOP, 1.0)
        };
        let mut y = centre.y + step * (r + 3.0 * scale);
        painter.text(
            egui::pos2(centre.x, y),
            align,
            ellipsis(ui, &node.label, &font, NODE_W * scale),
            font.clone(),
            if on { color::ACCENT_BRIGHT } else { color::TEXT },
        );
        y += step * (font.size + 1.0);
        if !node.detail.is_empty() {
            painter.text(
                egui::pos2(centre.x, y),
                align,
                ellipsis(ui, &node.detail, &small, NODE_W * scale),
                small.clone(),
                color::TEXT_MUTED,
            );
        }
    }

    // What the circle could not show, while the pointer is on it — drawn here,
    // by the app, in the app's own colours. A module hands over what it knows
    // and nothing about where it goes.
    if let Some(i) = hovered {
        let node = &nodes[i];
        egui::show_tooltip_at_pointer(&ctx, layer, egui::Id::new("diagram.host"), |ui| {
            ui.set_max_width(340.0);
            ui.label(
                egui::RichText::new(&node.label)
                    .strong()
                    .color(color::TEXT),
            );
            if !node.kind.is_empty() {
                ui.label(
                    egui::RichText::new(&node.kind)
                        .small()
                        .color(kind_colour(&node.kind)),
                );
            }
            // `detail` is already under the circle; the card repeats it only
            // when there is nothing else to say.
            let facts: Vec<(&str, &str)> = if node.info.is_empty() {
                node.detail
                    .is_empty()
                    .then(Vec::new)
                    .unwrap_or_else(|| vec![("", node.detail.as_str())])
            } else {
                node.info
                    .iter()
                    .map(|f| (f.name.as_str(), f.value.as_str()))
                    .collect()
            };
            if facts.is_empty() {
                return;
            }
            ui.add_space(4.0);
            egui::Grid::new("diagram.host.facts")
                .num_columns(2)
                .spacing(egui::vec2(12.0, 3.0))
                .show(ui, |ui| {
                    for (name, value) in facts {
                        ui.label(egui::RichText::new(name).small().color(color::TEXT_MUTED));
                        ui.label(
                            egui::RichText::new(value)
                                .monospace()
                                .color(color::TEXT),
                        );
                        ui.end_row();
                    }
                });
        });
    }
}

/// The screen point of the diagram's own origin, under the current pan and zoom.
fn origin_of(rect: egui::Rect, view: Viewport, middle: egui::Vec2, fit: f32) -> egui::Pos2 {
    rect.center() + view.pan - middle * (fit * view.zoom)
}

/// A line between two circles, shortened so it starts and ends on their edges.
fn trim(from: egui::Pos2, to: egui::Pos2, r: f32) -> (egui::Pos2, egui::Pos2) {
    let d = to - from;
    let len = d.length();
    if len <= r * 2.0 + 1.0 {
        return (from, to);
    }
    let step = d / len * r;
    (from + step, to - step)
}

/// Which circle the pointer is on, if any.
///
/// The circle, not the label under it: two labels can sit close enough to
/// overlap on a crowded ring, and a card that belongs to the neighbour of the
/// thing you are pointing at is worse than none. The nearest wins, so an overlap
/// still resolves to one node.
pub fn hit(places: &[Placed], scale: f32, origin: egui::Pos2, pos: egui::Pos2) -> Option<usize> {
    let reach = NODE_R * scale + 4.0;
    places
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let c = egui::pos2(origin.x + p.x * scale, origin.y + p.y * scale);
            (i, c.distance(pos))
        })
        .filter(|(_, d)| *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// `text` cut to `width` with an ellipsis, measured in the font it is drawn in.
fn ellipsis(ui: &egui::Ui, text: &str, font: &egui::FontId, width: f32) -> String {
    let measure = |s: &str| {
        ui.fonts(|f| f.layout_no_wrap(s.to_owned(), font.clone(), color::TEXT).size().x)
    };
    if measure(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut next = out.clone();
        next.push(ch);
        if measure(&format!("{next}…")) > width {
            break;
        }
        out = next;
    }
    format!("{out}…")
}
