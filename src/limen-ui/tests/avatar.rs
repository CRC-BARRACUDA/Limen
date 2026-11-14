//! Avatars: pictures decoded and animated, every effect stepped and drawn,
//! the effect's size floor, and the hover card's timing.

use std::sync::Arc;

use eframe::egui::{self, Pos2, Rect, Vec2};
use limen_ui::avatar::canvas::{Canvas, Face, FaceShape};
use limen_ui::avatar::effects::{self, EffectKind, Env};
use limen_ui::avatar::{self, AvatarLook, EffectSpec, Picture};
use limen_ui::user_card::{self, CardInfo};

/// The animations switch is one global, so a test that sets it cannot run
/// beside one that relies on it. They take turns.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run(ctx: &egui::Context, time: f64, events: Vec<egui::Event>, f: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
    let mut f = f;
    ctx.run(
        egui::RawInput {
            time: Some(time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            // No panel background: every shape counted is the avatar's own.
            egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| f(ui));
        },
    )
}

/// Every point in every shape, so a NaN anywhere is caught.
fn all_points(shapes: &[egui::epaint::ClippedShape]) -> Vec<Pos2> {
    fn walk(s: &egui::Shape, out: &mut Vec<Pos2>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            egui::Shape::Mesh(m) => out.extend(m.vertices.iter().map(|v| v.pos)),
            egui::Shape::Path(p) => out.extend(p.points.iter().copied()),
            egui::Shape::LineSegment { points, .. } => out.extend(points.iter().copied()),
            // A frame with nothing to paint still emits its rect: not drawing.
            egui::Shape::Rect(r) if r.fill.a() > 0 || (r.stroke.width > 0.0 && r.stroke.color.a() > 0) => {
                out.extend([r.rect.min, r.rect.max])
            }
            egui::Shape::Circle(c) => out.push(c.center),
            egui::Shape::Text(t) => out.push(t.pos),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in shapes {
        walk(&c.shape, &mut out);
    }
    out
}

fn face_texture(ctx: &egui::Context) -> (egui::TextureHandle, egui::TextureHandle) {
    let img = egui::ColorImage::new([8, 8], egui::Color32::from_rgb(200, 120, 48));
    (
        ctx.load_texture("t:still", img.clone(), Default::default()),
        ctx.load_texture("t:inv", img, Default::default()),
    )
}

#[test]
fn every_effect_steps_and_draws_with_a_hover_and_a_click() {
    let ctx = egui::Context::default();
    let (still, inv) = face_texture(&ctx);
    for kind in EffectKind::ALL {
        for size in [40.0, 64.0, 72.0] {
            for face in [
                Face::Picture { still: still.id(), inverted: inv.id() },
                Face::None,
                Face::Initials("JD".into()),
            ] {
                let env = Env::new(kind, size, &[], 0.5);
                let mut fx = effects::create(kind, &env, 42);
                let c = env.extent / 2.0;
                let mut drew = 0;
                // Three seconds: at rest, hovered for a while, a click, at rest.
                for step in 0..180 {
                    let pointer = (60..120).contains(&step).then_some(Pos2::new(c, c));
                    if step == 90 {
                        fx.press(Pos2::new(c, c));
                    }
                    fx.update(16.0, pointer);
                    let out = run(&ctx, step as f64 / 60.0, vec![], |ui| {
                        let pad = (env.extent - size) / 2.0;
                        let face_rect = Rect::from_min_size(Pos2::new(pad, pad), Vec2::splat(size));
                        fx.draw(&Canvas::new(ui.painter().clone(), Pos2::new(100.0, 100.0), face.clone(), kind.face_shape(), face_rect));
                    });
                    let pts = all_points(&out.shapes);
                    assert!(pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()), "{kind:?} {size}: a NaN");
                    if !pts.is_empty() {
                        drew += 1;
                    }
                }
                // Every effect has something standing every frame — a frame,
                // brackets, scanlines — not only during its bursts.
                assert_eq!(drew, 180, "{kind:?} at {size}px drew on only {drew} of 180 frames");
            }
        }
    }
}

#[test]
fn effects_follow_the_kit_face_shapes_and_defaults() {
    assert_eq!(EffectKind::Neon.face_shape(), FaceShape::Rounded);
    for k in EffectKind::ALL {
        assert!(!k.default_colors().is_empty(), "{k:?}");
        assert_eq!(EffectKind::parse(&format!("{k:?}").to_lowercase()), Some(k));
        if k != EffectKind::Neon {
            assert_eq!(k.face_shape(), FaceShape::Square, "{k:?}");
        }
    }
    let env = Env::new(EffectKind::Glitch, 64.0, &["#0ff".into(), "nope".into()], 0.5);
    assert!(env.custom_colors && env.colors.len() == 1, "only checked colours are taken");
    assert_eq!((env.extent, env.scale), (128.0, 1.0), "a canvas twice the avatar's size");
}

fn gif(frames: &[([u8; 4], u32)], side: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut buf);
        enc.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
        let frames = frames.iter().map(|(px, ms)| {
            image::Frame::from_parts(
                image::RgbaImage::from_pixel(side, side, image::Rgba(*px)),
                0,
                0,
                image::Delay::from_numer_denom_ms(*ms, 1),
            )
        });
        enc.encode_frames(frames).unwrap();
    }
    buf
}

#[test]
fn an_animated_gif_keeps_its_frames_and_their_timing() {
    let bytes = gif(&[([255, 0, 0, 255], 100), ([0, 255, 0, 255], 200), ([0, 0, 255, 255], 0)], 16);
    let d = avatar::decode(&bytes, avatar::MAX_SIDE).unwrap();
    let delays: Vec<u32> = d.frames.iter().map(|(_, ms)| *ms).collect();
    assert_eq!(delays, [100, 200, 100], "a 0ms frame shows for 100ms, as browsers show it");
    assert_eq!(d.frames[1].0.pixels[0], egui::Color32::from_rgb(0, 255, 0));

    let ctx = egui::Context::default();
    let p = Picture::upload(&ctx, "t", d);
    assert!(p.animated());
    let id = |ms: f64, on: bool| p.frame_at(ms, on).0.id();
    assert_eq!(id(50.0, true), id(0.0, true), "still the first frame at 50ms");
    assert_ne!(id(150.0, true), id(50.0, true), "the second by 150ms");
    assert_eq!(id(400.0 + 50.0, true), id(50.0, true), "and round again after 400ms");
    assert_eq!(p.frame_at(150.0, false).1, None, "animations off: the first frame, still");
    assert_eq!(id(150.0, false), id(0.0, true));
    let (_, next) = p.frame_at(150.0, true);
    assert_eq!(next, Some(150.0), "the next frame is due when the second ends");
}

#[test]
fn a_big_still_is_scaled_down_and_garbage_is_refused() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(1000, 500, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let d = avatar::decode(&png, avatar::MAX_SIDE).unwrap();
    assert_eq!(d.frames.len(), 1);
    assert_eq!(d.frames[0].0.size, [avatar::MAX_SIDE as usize, avatar::MAX_SIDE as usize / 2]);
    let p = Picture::upload(&egui::Context::default(), "big", d);
    assert!(!p.animated());
    assert_eq!(p.aspect, 2.0);
    assert!(avatar::decode(b"<svg onload=alert(1)>", avatar::MAX_SIDE).is_err());
    assert!(avatar::decode(&[0u8; 64], avatar::MAX_SIDE).is_err());
}

#[test]
fn initials_are_first_and_last() {
    assert_eq!(avatar::initials("Ada Lovelace"), "AL");
    assert_eq!(avatar::initials("ada  byron lovelace"), "AL");
    assert_eq!(avatar::initials("jdoe"), "J");
    assert_eq!(avatar::initials("  "), "?");
}

fn glitch_look(ctx: &egui::Context) -> AvatarLook {
    let d = avatar::decode(&gif(&[([200, 120, 48, 255], 0)], 8), avatar::MAX_SIDE).unwrap();
    AvatarLook {
        user_id: "u1".into(),
        name: "Jane Doe".into(),
        picture: Some(Picture::upload(ctx, "face", d)),
        frame: None,
        effect: Some(EffectSpec { kind: EffectKind::Glitch, colors: vec![], intensity: 0.5 }),
    }
}

/// How far past the avatar's own rect anything was drawn, over a second.
fn spill(size: f32, animations: bool) -> f32 {
    spill_from(size, animations, avatar::EFFECT_MIN_SIZE)
}

fn spill_from(size: f32, animations: bool, floor: f32) -> f32 {
    limen_ui::set_animations(animations);
    let ctx = egui::Context::default();
    let look = glitch_look(&ctx);
    let mut rect = Rect::NOTHING;
    let mut furthest: f32 = 0.0;
    for f in 0..60 {
        let out = run(&ctx, f as f64 / 60.0, vec![], |ui| {
            ui.add_space(100.0);
            ui.horizontal(|ui| {
                ui.add_space(100.0);
                rect = avatar::avatar_from(ui, egui::Id::new("a"), &look, size, false, None, floor).rect;
            });
        });
        for p in all_points(&out.shapes) {
            if rect.is_positive() {
                let d = (rect.left() - p.x).max(p.x - rect.right()).max(rect.top() - p.y).max(p.y - rect.bottom());
                furthest = furthest.max(d);
            }
        }
    }
    limen_ui::set_animations(true);
    furthest
}

#[test]
fn the_effect_spills_past_the_avatar_from_40px_and_only_when_animating() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    assert!(spill(40.0, true) > 2.0, "glitch's frame stands outside the face");
    assert!(spill(39.0, true) <= 1.0, "below 40px the effect is not drawn");
    assert!(spill(40.0, false) <= 1.0, "animations off: no effect");
    assert!(spill_from(24.0, true, 24.0) > 1.0, "the title bar's 24px avatar keeps its effect");
}

#[test]
fn the_card_opens_after_a_rest_and_closes_after_leaving() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    limen_ui::set_animations(true);
    let ctx = egui::Context::default();
    let look = glitch_look(&ctx);
    let details = vec!["jane@example.com".to_string()];
    let mut t = 0.0;
    let chip = std::cell::Cell::new(Rect::NOTHING);
    // Whether the card's name was drawn this frame.
    let frame = |t: f64, at: Option<Pos2>| -> bool {
        let events = at.map(|p| vec![egui::Event::PointerMoved(p)]).unwrap_or_else(|| vec![egui::Event::PointerGone]);
        let out = run(&ctx, t, events, |ui| {
            ui.add_space(50.0);
            let resp = avatar::avatar(ui, egui::Id::new("chip"), &look, 40.0, true, None);
            chip.set(resp.rect);
            user_card::hover_card(
                ui,
                egui::Id::new("card"),
                &resp,
                &CardInfo { look: &look, name: "Jane Doe", username: "jdoe", details: &details, tag: Some("You"), link: None, sign_out: None },
            );
        });
        let texts: Vec<String> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        texts.iter().any(|s| s == "Jane Doe")
    };
    frame(t, None);
    let on = chip.get().center();
    // Resting on the chip: nothing for the first 350ms.
    while t < 0.3 {
        t += 1.0 / 60.0;
        assert!(!frame(t, Some(on)), "the card waited only {t:.2}s");
    }
    let mut opened = false;
    while t < 0.5 {
        t += 1.0 / 60.0;
        opened |= frame(t, Some(on));
    }
    assert!(opened, "the card opened after resting on the chip");
    // Away: it stays for the close delay and its exit, then it is gone.
    let away = Pos2::new(780.0, 580.0);
    t += 1.0 / 60.0;
    assert!(frame(t, Some(away)), "not gone the moment the pointer leaves");
    let mut gone_at = None;
    while t < 1.2 {
        t += 1.0 / 60.0;
        if !frame(t, Some(away)) && gone_at.is_none() {
            gone_at = Some(t);
        }
    }
    let left_at = 0.5 + 1.0 / 60.0;
    let gone = gone_at.expect("the card closed") - left_at;
    assert!((0.25..0.45).contains(&gone), "closed {gone:.2}s after leaving: 150ms, then a 140ms exit");
}

#[test]
fn signing_out_from_the_card_says_so_and_closes_it() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    limen_ui::set_animations(true);
    let ctx = egui::Context::default();
    let look = glitch_look(&ctx);
    let chip = std::cell::Cell::new(Rect::NOTHING);
    let said = std::cell::Cell::new(false);
    // Draw a frame; where "SIGN OUT" was drawn, if it was.
    let frame = |t: f64, events: Vec<egui::Event>| -> Option<Pos2> {
        let out = run(&ctx, t, events, |ui| {
            ui.add_space(50.0);
            let resp = avatar::avatar(ui, egui::Id::new("chip"), &look, 40.0, true, None);
            chip.set(resp.rect);
            let info = CardInfo { look: &look, name: "Jane Doe", username: "jdoe", details: &[], tag: None, link: None, sign_out: Some("Sign out") };
            if user_card::hover_card(ui, egui::Id::new("card"), &resp, &info) {
                said.set(true);
            }
        });
        out.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "SIGN OUT" => Some(t.pos + t.galley.size() / 2.0),
            _ => None,
        })
    };
    frame(0.0, vec![]);
    let on = chip.get().center();
    let mut t = 0.0;
    let mut at = None;
    while t < 0.8 {
        t += 1.0 / 60.0;
        at = frame(t, vec![egui::Event::PointerMoved(on)]).or(at);
    }
    let at = at.expect("your own card offers to sign out");
    // Over to it — the card stays while the pointer is on it — and click.
    for _ in 0..3 {
        t += 1.0 / 60.0;
        frame(t, vec![egui::Event::PointerMoved(at)]);
    }
    for pressed in [true, false] {
        t += 1.0 / 60.0;
        frame(t, vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }]);
    }
    assert!(said.get(), "the click came back as a sign-out");
    t += 1.0 / 60.0;
    assert_eq!(frame(t, vec![egui::Event::PointerMoved(at)]), None, "and the card is gone at once");
}

#[test]
fn a_picture_arc_can_be_shared() {
    // AvatarLook is Clone and Send between frames; its pictures are shared.
    let ctx = egui::Context::default();
    let look = glitch_look(&ctx);
    let copy = look.clone();
    assert!(Arc::ptr_eq(look.picture.as_ref().unwrap(), copy.picture.as_ref().unwrap()));
}
