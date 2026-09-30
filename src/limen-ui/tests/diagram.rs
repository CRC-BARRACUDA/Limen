//! Laying a diagram out: where the boxes go, and that they stay there.

use eframe::egui;
use limen_ui as ui;
use limen_ui::{DiagramEdge, DiagramFact, DiagramNode};

fn node(id: &str, kind: &str) -> DiagramNode {
    DiagramNode {
        id: id.into(),
        label: id.into(),
        kind: kind.into(),
        detail: String::new(),
        info: Vec::new(),
    }
}

fn edge(from: &str, to: &str) -> DiagramEdge {
    DiagramEdge {
        from: from.into(),
        to: to.into(),
        dashed: false,
    }
}

/// The pointer picks the circle it is on, and only that one.
///
/// The circle rather than the label under it: on a crowded ring two labels can
/// overlap, and a card describing the neighbour of the host you are pointing at
/// is worse than no card.
#[test]
fn the_pointer_picks_the_circle_it_is_on() {
    let nodes = vec![node("gw", "router"), node("pc1", "pc"), node("pc2", "pc")];
    let edges = vec![edge("pc1", "gw"), edge("pc2", "gw")];
    let places = limen_ui::diagram::layout(&nodes, &edges);
    let origin = egui::pos2(500.0, 400.0);
    let at = |i: usize| {
        egui::pos2(origin.x + places[i].x, origin.y + places[i].y)
    };

    for (i, node) in nodes.iter().enumerate() {
        assert_eq!(
            limen_ui::diagram::hit(&places, 1.0, origin, at(i)),
            Some(i),
            "{} did not answer for its own centre",
            node.id
        );
    }
    // Just off a circle is nothing at all, not the nearest thing on the map.
    let away = egui::pos2(at(0).x + 60.0, at(0).y);
    assert_eq!(limen_ui::diagram::hit(&places, 1.0, origin, away), None);
}

/// Every node is drawn as a circle — one each, no more.
#[test]
fn every_node_is_a_circle() {
    let nodes: Vec<DiagramNode> = std::iter::once(node("hub", "router"))
        .chain((0..6).map(|i| node(&format!("n{i}"), "pc")))
        .collect();
    let edges: Vec<DiagramEdge> = (0..6).map(|i| edge(&format!("n{i}"), "hub")).collect();

    let ctx = egui::Context::default();
    limen_ui::apply_theme(&ctx);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        )),
        ..Default::default()
    };
    let out = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            limen_ui::render_diagram(ui, "Topology", &nodes, &edges);
        });
    });
    let circles = out
        .shapes
        .iter()
        .filter(|s| matches!(s.shape, egui::epaint::Shape::Circle(_)))
        .count();
    assert_eq!(circles, nodes.len(), "one circle per node");
}

/// The card is the app's own: what a module sends as facts is what it shows,
/// and it shows them in the order they were given.
#[test]
fn the_facts_of_a_node_survive_the_wire() {
    let sent = limen_sdk_rust::ui::diagram(
        "Network",
        vec![limen_sdk_rust::ui::node("gw", "gateway")
            .kind("router")
            .detail("10.0.0.1")
            .info("address", "10.0.0.1")
            .info("hardware", "AA:BB:CC:DD:EE:01")],
        vec![],
    );
    let json = sent.into_value();
    let view: limen_ui::View =
        serde_json::from_value(serde_json::json!({ "title": "t", "widgets": [json] }))
            .expect("and the host reads it back");
    let limen_ui::Widget::Diagram { nodes, .. } = &view.widgets[0] else {
        panic!("not a diagram: {:?}", view.widgets[0]);
    };
    let facts: Vec<(&str, &str)> = nodes[0]
        .info
        .iter()
        .map(|f: &DiagramFact| (f.name.as_str(), f.value.as_str()))
        .collect();
    assert_eq!(
        facts,
        vec![("address", "10.0.0.1"), ("hardware", "AA:BB:CC:DD:EE:01")]
    );
    assert_eq!(nodes[0].kind, "router");
}

/// A network is a star: every machine pointing at one gateway. The gateway
/// belongs in the middle, and everything else around it.
#[test]
fn the_hub_is_what_the_lines_lead_to() {
    let nodes = vec![
        node("pc1", "pc"),
        node("gw", "router"),
        node("pc2", "pc"),
        node("pc3", "pc"),
    ];
    let edges = vec![edge("pc1", "gw"), edge("pc2", "gw"), edge("pc3", "gw")];
    let places = ui::diagram::layout(&nodes, &edges);

    // The gateway sits at the origin; nothing else does.
    assert_eq!((places[1].x, places[1].y), (0.0, 0.0), "the hub is not centred");
    for i in [0, 2, 3] {
        let d = places[i].x.hypot(places[i].y);
        assert!(d > 40.0, "{} sits on top of the hub", nodes[i].id);
    }
}

/// The same input lays out the same way, every time and on every machine.
///
/// No physics and no random seed: a map that settles differently each time it
/// is opened is one two people cannot compare, and one that moves while it is
/// being read.
#[test]
fn the_layout_is_the_same_twice() {
    let nodes: Vec<DiagramNode> = (0..9).map(|i| node(&format!("n{i}"), "pc")).collect();
    let edges: Vec<DiagramEdge> = (1..9).map(|i| edge(&format!("n{i}"), "n0")).collect();
    let once = ui::diagram::layout(&nodes, &edges);
    let twice = ui::diagram::layout(&nodes, &edges);
    assert_eq!(once.len(), nodes.len());
    for (a, b) in once.iter().zip(&twice) {
        assert_eq!((a.x, a.y), (b.x, b.y));
    }
}

/// Boxes on a ring do not overlap, however many are on it.
#[test]
fn the_ring_grows_with_what_is_on_it() {
    for count in [3usize, 8, 20, 40] {
        let nodes: Vec<DiagramNode> =
            std::iter::once(node("hub", "router"))
                .chain((0..count).map(|i| node(&format!("n{i}"), "pc")))
                .collect();
        let edges: Vec<DiagramEdge> =
            (0..count).map(|i| edge(&format!("n{i}"), "hub")).collect();
        let places = ui::diagram::layout(&nodes, &edges);

        // Neighbours on the ring are at least a box apart, measured between the
        // two that ended up closest.
        let ring = &places[1..];
        let mut closest = f32::MAX;
        for (i, a) in ring.iter().enumerate() {
            for b in ring.iter().skip(i + 1) {
                closest = closest.min((a.x - b.x).hypot(a.y - b.y));
            }
        }
        assert!(closest > 20.0, "{count} boxes are {closest} apart");
    }
}

/// A node joined to nothing still has somewhere to be — outside the ring of
/// those that are, rather than on top of the middle.
#[test]
fn an_unconnected_node_is_not_stacked_on_the_hub() {
    let nodes = vec![node("gw", "router"), node("pc", "pc"), node("lonely", "pc")];
    let edges = vec![edge("pc", "gw")];
    let places = ui::diagram::layout(&nodes, &edges);
    let lonely = places[2].x.hypot(places[2].y);
    let joined = places[1].x.hypot(places[1].y);
    assert!(lonely > joined, "the unjoined node is inside the ring");
}

/// The degenerate cases answer with something rather than panicking.
#[test]
fn nothing_and_one_thing_both_lay_out() {
    assert!(ui::diagram::layout(&[], &[]).is_empty());
    let one = vec![node("alone", "pc")];
    let places = ui::diagram::layout(&one, &[]);
    assert_eq!(places.len(), 1);
    assert_eq!((places[0].x, places[0].y), (0.0, 0.0));
    // An edge naming a node that is not there is ignored, not a panic.
    let places = ui::diagram::layout(&one, &[edge("alone", "ghost")]);
    assert_eq!(places.len(), 1);
}

/// Every circle lands inside the width the diagram was given, at any size.
#[test]
fn the_whole_diagram_is_drawn_inside_its_width() {
    for count in [2usize, 12, 30] {
        let nodes: Vec<DiagramNode> =
            std::iter::once(node("hub", "router"))
                .chain((0..count).map(|i| node(&format!("n{i}"), "pc")))
                .collect();
        let edges: Vec<DiagramEdge> =
            (0..count).map(|i| edge(&format!("n{i}"), "hub")).collect();

        let ctx = egui::Context::default();
        limen_ui::apply_theme(&ctx);
        let mut drawn: Vec<egui::Rect> = Vec::new();
        // Twice: the first frame lays out, the second draws at the size it
        // learned, and it is the second that has to stay inside the window.
        for _ in 0..2 {
            drawn.clear();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                ..Default::default()
            };
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    limen_ui::render_diagram(ui, "Topology", &nodes, &edges);
                });
            });
            for shape in out.shapes {
                if let egui::epaint::Shape::Circle(c) = shape.shape {
                    drawn.push(egui::Rect::from_center_size(
                        c.center,
                        egui::Vec2::splat(c.radius * 2.0),
                    ));
                }
            }
        }
        assert_eq!(drawn.len(), nodes.len(), "{count}: a node went missing");
        for r in &drawn {
            assert!(r.min.x >= -1.0 && r.max.x <= 901.0, "{count}: a circle at {r:?}");
        }
    }
}

/// Hovering a circle puts the card up, and the app draws it: the facts the
/// module sent, in the app's own colours, with no browser and no tooltip the
/// desktop owns.
#[test]
fn hovering_a_circle_shows_its_facts_in_limens_colours() {
    let mut gw = node("gw", "router");
    gw.label = "gateway".into();
    gw.info = vec![
        DiagramFact { name: "Address".into(), value: "10.0.0.1".into() },
        DiagramFact { name: "Hardware address".into(), value: "AA:BB:CC:DD:EE:01".into() },
    ];
    let nodes = vec![gw, node("pc1", "pc"), node("pc2", "pc")];
    let edges = vec![edge("pc1", "gw"), edge("pc2", "gw")];

    let ctx = egui::Context::default();
    limen_ui::apply_theme(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));

    // Where the hub's circle ended up, read off the first frame rather than
    // guessed: the diagram centres itself in the width it is given.
    let mut centre = egui::Pos2::ZERO;
    let find = |out: egui::FullOutput, centre: &mut egui::Pos2| {
        let mut texts = Vec::new();
        let mut fills = Vec::new();
        for shape in out.shapes {
            match shape.shape {
                egui::epaint::Shape::Circle(c) if c.fill == limen_ui::color::ACCENT => {
                    *centre = c.center;
                }
                egui::epaint::Shape::Text(t) => texts.push(t.galley.text().to_owned()),
                egui::epaint::Shape::Rect(r) => fills.push(r.fill),
                _ => {}
            }
        }
        (texts, fills)
    };
    let input = |pos: Option<egui::Pos2>| egui::RawInput {
        screen_rect: Some(screen),
        events: pos.map(|p| vec![egui::Event::PointerMoved(p)]).unwrap_or_default(),
        ..Default::default()
    };
    let draw = |ctx: &egui::Context, input: egui::RawInput| {
        ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                limen_ui::render_diagram(ui, "Topology", &nodes, &edges);
            });
        })
    };

    find(draw(&ctx, input(None)), &mut centre);
    assert_ne!(centre, egui::Pos2::ZERO, "the router's circle was not drawn");

    // Away from every circle: no card.
    let (texts, _) = find(draw(&ctx, input(Some(egui::pos2(5.0, 690.0)))), &mut centre.clone());
    assert!(!texts.iter().any(|t| t == "Hardware address"), "a card with nothing hovered");

    // On the circle: the card, with what the module sent. Twice — a tooltip
    // goes up on the frame after the one that noticed the pointer.
    draw(&ctx, input(Some(centre)));
    let out = draw(&ctx, input(Some(centre)));
    let (texts, fills) = find(out, &mut centre.clone());
    for want in ["gateway", "Address", "10.0.0.1", "Hardware address", "AA:BB:CC:DD:EE:01"] {
        assert!(texts.iter().any(|t| t == want), "{want:?} is not on the card: {texts:?}");
    }
    assert!(
        fills.contains(&limen_ui::color::BG),
        "the card is not painted in the app's background: {fills:?}"
    );
}

/// A map drawn small enough to fit is still a map you can take hold of and
/// move: dragging it moves it, by exactly as far as the pointer went.
#[test]
fn dragging_moves_the_map() {
    let mut app = Harness::new("pan");
    let before = app.circles(vec![]);
    let start = before[0];
    let moved = app.drag(start, egui::vec2(60.0, -35.0));
    for (a, b) in before.iter().zip(&moved) {
        assert!(
            ((*b - *a) - egui::vec2(60.0, -35.0)).length() < 0.5,
            "the map went {:?} for a drag of (60, -35)",
            *b - *a
        );
    }
}

/// The map cannot be thrown away: however far it is dragged, something of it is
/// still in the window. An empty box with no way to tell why is worse than a map
/// that will not go any further.
#[test]
fn the_map_cannot_be_dragged_out_of_sight() {
    let mut app = Harness::new("pan.far");
    let area = app.area();
    let start = app.circles(vec![])[0];
    let flung = app.drag(start, egui::vec2(9000.0, 9000.0));
    assert!(
        flung.iter().any(|c| area.expand(60.0).contains(*c)),
        "every circle left the window: {flung:?}"
    );
}

/// And a way back from wherever it was dragged to.
#[test]
fn double_clicking_puts_the_map_back() {
    let mut app = Harness::new("pan.reset");
    let home = app.circles(vec![]);
    let start = home[0];
    app.drag(start, egui::vec2(120.0, 90.0));
    let here = app.circles(vec![])[0];
    let back = app.double_click(here);
    for (a, b) in home.iter().zip(&back) {
        assert!((*b - *a).length() < 0.5, "it did not come back: {a:?} -> {b:?}");
    }
}

/// The plain wheel belongs to the page the map is on. A map that swallowed it
/// would trap the scroll of every view it ever appears in.
#[test]
fn the_wheel_alone_does_not_zoom_the_map() {
    let mut app = Harness::new("pan.wheel");
    let before = app.circles(vec![]);
    let after = app.circles(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, -8.0),
        modifiers: egui::Modifiers::NONE,
    }]);
    for (a, b) in before.iter().zip(&after) {
        assert!((*b - *a).length() < 0.5, "the wheel moved the map");
    }
}

/// Whatever it is dragged to, the map is drawn inside the box it was given and
/// not over what the module put above and below it.
#[test]
fn a_panned_map_stays_inside_its_own_box() {
    let mut app = Harness::new("pan.clip");
    let start = app.circles(vec![])[0];
    app.drag(start, egui::vec2(400.0, 300.0));
    let clips = app.clip_rects();
    assert!(!clips.is_empty(), "nothing was drawn");
    for c in &clips {
        assert!(
            c.height() < 690.0,
            "the map is drawn against the whole window ({c:?}), so it is not clipped"
        );
    }
}

/// A screen that is a map gets the screen: the map takes every bit of height
/// that is left, and the buttons under it go to the very bottom of the window
/// rather than sitting directly beneath it with the rest of the room empty.
#[test]
fn a_filling_diagram_takes_the_screen_and_the_buttons_go_under_it() {
    let view: limen_ui::View = serde_json::from_value(serde_json::json!({
        "title": "Network map",
        "widgets": [
            { "kind": "label", "text": "everything this machine has spoken to" },
            { "kind": "separator" },
            limen_sdk_rust::ui::diagram(
                "Network map",
                vec![
                    limen_sdk_rust::ui::node("gw", "gateway").kind("router"),
                    limen_sdk_rust::ui::node("pc", "workstation").kind("pc"),
                ],
                vec![("pc".into(), "gw".into(), false)],
            )
            .fills()
            .into_value(),
            { "kind": "separator" },
            { "kind": "row", "children": [
                { "kind": "button", "text": "Back", "action": { "capability": "c", "method": "ui" } },
            ]},
        ]
    }))
    .expect("the host reads the view back");

    let ctx = egui::Context::default();
    limen_ui::apply_theme(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
    let mut inputs = std::collections::HashMap::new();
    let mut circles = Vec::new();
    let mut back = None;
    let mut texts: Vec<String> = Vec::new();
    // The second frame runs later than the first: the tail's height is what
    // decides the map's, and the view's entrance has to be over before anything
    // is where it will stay — at zero opacity egui draws nothing at all.
    for time in [1.0, 6.0] {
        circles.clear();
        let out = ctx.run(
            egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    limen_ui::render_view(ui, &view, &mut inputs, None, 0.0);
                });
            },
        );
        for shape in out.shapes {
            match shape.shape {
                egui::epaint::Shape::Circle(c) => circles.push(c.center),
                // Buttons draw their label in capitals.
                egui::epaint::Shape::Text(t) => {
                    texts.push(t.galley.text().to_owned());
                    if t.galley.text().eq_ignore_ascii_case("back") {
                        back = Some(t.pos);
                    }
                }
                _ => {}
            }
        }
    }

    let back = back.unwrap_or_else(|| panic!("the button was not drawn; saw {texts:?}"));
    // Inside the window, and against its bottom. Without the map filling, the
    // buttons follow it directly and the rest of the screen is left empty —
    // which on a 700px window puts them off the bottom of it entirely.
    assert!(
        (560.0..700.0).contains(&back.y),
        "the buttons are at y={} on a 700px screen, not at the bottom of it",
        back.y
    );
    assert_eq!(circles.len(), 2, "the map was not drawn");
    // The map is centred in everything it was given, rather than sitting in a
    // band at the top with the room under it wasted. It is not blown up to fill
    // it — a two-node network drawn 700px tall reads as a mistake — so what
    // fills the screen is the map's own canvas, which is what can be dragged.
    let middle = circles.iter().map(|c| c.y).sum::<f32>() / circles.len() as f32;
    assert!(
        (280.0..back.y).contains(&middle),
        "the map sits at {middle}, not in the middle of the room above the buttons"
    );
    for c in &circles {
        assert!(c.y < back.y, "a circle at {c:?} is under the buttons");
    }
}

/// Double-clicking a circle asks about that host, and says which one: the id
/// the module named it by, so it can answer without the map having to carry the
/// answer around with it.
#[test]
fn double_clicking_a_circle_asks_about_that_host() {
    let view = map_view(true);
    let mut app = ViewHarness::new(view);
    let circles = app.circles(vec![]);
    let asked = app.double_click(circles[0]);

    let asked = asked.expect("nothing was asked");
    assert_eq!(asked.action.method, "host");
    assert!(asked.open_in_tab, "a host belongs in a tab of its own");
    // Which circle was double-clicked, by the id the module gave it.
    let id = asked.args.get("id").and_then(|v| v.as_str());
    assert!(
        matches!(id, Some("gw") | Some("pc")),
        "it asked about {id:?}, which is not a node on this map"
    );
}

/// The gesture only asks when there is a circle under it. On the map itself a
/// double click is still the way back from wherever it has been dragged to.
#[test]
fn double_clicking_the_map_itself_asks_nothing_and_puts_it_back() {
    let view = map_view(true);
    let mut app = ViewHarness::new(view);
    let home = app.circles(vec![]);
    let empty = egui::pos2(60.0, 300.0);
    assert!(
        home.iter().all(|c| c.distance(empty) > 40.0),
        "the spot picked for 'empty map' has a circle on it"
    );

    app.drag(home[0], egui::vec2(150.0, 60.0));
    let asked = app.double_click(empty);
    assert!(asked.is_none(), "double-clicking the map asked about a host");
    let back = app.circles(vec![]);
    for (a, b) in home.iter().zip(&back) {
        assert!((*b - *a).length() < 0.5, "it did not come back: {a:?} -> {b:?}");
    }
}

/// A map whose module asked for nothing does nothing: no tab, and the double
/// click goes back to being the way home.
#[test]
fn a_map_that_was_given_no_question_asks_none() {
    let mut app = ViewHarness::new(map_view(false));
    let circles = app.circles(vec![]);
    assert!(app.double_click(circles[0]).is_none());
}

/// A view holding one map, with or without something to ask when a circle is
/// double-clicked.
fn map_view(activates: bool) -> limen_ui::View {
    let mut d = limen_sdk_rust::ui::diagram(
        "",
        vec![
            limen_sdk_rust::ui::node("gw", "gateway").kind("router"),
            limen_sdk_rust::ui::node("pc", "workstation").kind("pc"),
        ],
        vec![("pc".into(), "gw".into(), false)],
    )
    .fills();
    if activates {
        d = d.on_activate("network.map", "host");
    }
    serde_json::from_value(serde_json::json!({
        "title": "Network map",
        "widgets": [d.into_value()],
    }))
    .expect("the host reads the view back")
}

/// A whole view driven through real frames, for what the pointer asks of it.
struct ViewHarness {
    ctx: egui::Context,
    view: limen_ui::View,
    inputs: std::collections::HashMap<String, String>,
    time: f64,
}

impl ViewHarness {
    fn new(view: limen_ui::View) -> Self {
        let ctx = egui::Context::default();
        limen_ui::apply_theme(&ctx);
        let mut me = Self { ctx, view, inputs: Default::default(), time: 0.0 };
        // Let the view's entrance finish: at zero opacity egui draws nothing,
        // and nothing is where it will stay until it is over.
        me.frame(vec![]);
        me.time += 5.0;
        me.frame(vec![]);
        me
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (Vec<egui::Pos2>, Option<limen_ui::Invoke>) {
        self.time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            events,
            time: Some(self.time),
            ..Default::default()
        };
        let (view, inputs) = (&self.view, &mut self.inputs);
        let mut asked = None;
        let out = self.ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some(a) = limen_ui::render_view(ui, view, inputs, None, 0.0) {
                    asked = Some(a);
                }
            });
        });
        let circles = out
            .shapes
            .into_iter()
            .filter_map(|s| match s.shape {
                egui::epaint::Shape::Circle(c) => Some(c.center),
                _ => None,
            })
            .collect();
        (circles, asked)
    }

    fn circles(&mut self, events: Vec<egui::Event>) -> Vec<egui::Pos2> {
        self.frame(events).0
    }

    fn drag(&mut self, from: egui::Pos2, by: egui::Vec2) {
        self.frame(vec![
            egui::Event::PointerMoved(from),
            egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        self.frame(vec![egui::Event::PointerMoved(from + by)]);
        self.frame(vec![egui::Event::PointerButton {
            pos: from + by,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
    }

    fn double_click(&mut self, at: egui::Pos2) -> Option<limen_ui::Invoke> {
        let click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        let mut asked = None;
        for _ in 0..2 {
            asked = asked.or(self.frame(vec![click(true), click(false)]).1);
        }
        asked.or(self.frame(vec![]).1)
    }
}

/// Driving a real diagram through real frames: the only way to find out what the
/// pointer does to it.
struct Harness {
    ctx: egui::Context,
    nodes: Vec<DiagramNode>,
    edges: Vec<DiagramEdge>,
    title: String,
    /// Frames happen at a time; a double click is two clicks close together in
    /// one, and a drag is a press in one and a move in the next.
    time: f64,
}

impl Harness {
    fn new(title: &str) -> Self {
        let ctx = egui::Context::default();
        limen_ui::apply_theme(&ctx);
        let nodes: Vec<DiagramNode> = std::iter::once(node("hub", "router"))
            .chain((0..5).map(|i| node(&format!("n{i}"), "pc")))
            .collect();
        let edges: Vec<DiagramEdge> = (0..5).map(|i| edge(&format!("n{i}"), "hub")).collect();
        Self { ctx, nodes, edges, title: title.into(), time: 0.0 }
    }

    fn area(&self) -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))
    }

    /// Run one frame with these events and hand back every shape it painted.
    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
        self.time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(self.area()),
            events,
            time: Some(self.time),
            ..Default::default()
        };
        let (nodes, edges, title) = (&self.nodes, &self.edges, &self.title);
        self.ctx
            .run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    limen_ui::render_diagram(ui, title, nodes, edges);
                });
            })
            .shapes
    }

    /// Where the circles ended up after a frame with these events.
    fn circles(&mut self, events: Vec<egui::Event>) -> Vec<egui::Pos2> {
        self.frame(events)
            .into_iter()
            .filter_map(|s| match s.shape {
                egui::epaint::Shape::Circle(c) => Some(c.center),
                _ => None,
            })
            .collect()
    }

    fn clip_rects(&mut self) -> Vec<egui::Rect> {
        self.frame(vec![])
            .into_iter()
            .filter(|s| matches!(s.shape, egui::epaint::Shape::Circle(_)))
            .map(|s| s.clip_rect)
            .collect()
    }

    /// Press at `from`, then move by `by` — one frame each, as a pointer
    /// actually does it: egui learns a drag from where the pointer was last
    /// frame and where it is in this one.
    fn drag(&mut self, from: egui::Pos2, by: egui::Vec2) -> Vec<egui::Pos2> {
        self.circles(vec![
            egui::Event::PointerMoved(from),
            egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let moved = self.circles(vec![egui::Event::PointerMoved(from + by)]);
        self.circles(vec![egui::Event::PointerButton {
            pos: from + by,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        moved
    }

    /// Two clicks in the same place, close enough together to be one gesture.
    fn double_click(&mut self, at: egui::Pos2) -> Vec<egui::Pos2> {
        let click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.circles(vec![egui::Event::PointerMoved(at)]);
        for _ in 0..2 {
            self.circles(vec![click(true), click(false)]);
        }
        // The second click lands as an event; the frame after it is the one
        // that draws what it did.
        self.circles(vec![])
    }
}
