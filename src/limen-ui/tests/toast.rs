//! Passing notices, and the four things they can be.

use eframe::egui;

use limen_ui::toast::Level;

/// The colours and codes are the platform ui-kit's, not this application's own:
/// the two are meant to read as one product, and a notice is the place that is
/// most obvious when they drift.
#[test]
fn every_level_is_the_kit_s_level() {
    // The four the kit defines, and no others.
    let all = [Level::Info, Level::Ok, Level::Warning, Level::Error];
    assert_eq!(all.len(), 4);

    // Each is distinct — a notice whose colour matched another's would be worse
    // than an uncoloured one, because it would be confidently wrong. The same
    // goes for the word beside the icon.
    for (i, a) in all.iter().enumerate() {
        assert!(!a.name().is_empty(), "{a:?} has nothing to call itself");
        for b in &all[i + 1..] {
            assert_ne!(a, b);
            assert_ne!(a.name(), b.name(), "{a:?} and {b:?} answer to the same word");
        }
    }
}

/// A notice must survive the frame it was raised in.
///
/// It is at zero opacity on that frame — it has only just started arriving — and
/// reading that as "finished fading out" removes it before it is ever drawn. The
/// bug looks exactly like the feature not working at all.
#[test]
fn a_notice_lives_past_the_frame_that_raised_it() {
    let ctx = egui::Context::default();

    // One frame: raise it while the frame is running, as a button would.
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |_ui| {
            limen_ui::toast::notify(ctx, Level::Ok, "still here?");
        });
        limen_ui::toast::draw(ctx);
    });
    assert_eq!(limen_ui::toast::showing(&ctx), 1, "gone on the frame it appeared");

    // And several frames later it is still there — the dwell is seconds long.
    for _ in 0..5 {
        let _ = ctx.run(egui::RawInput::default(), limen_ui::toast::draw);
    }
    assert_eq!(limen_ui::toast::showing(&ctx), 1, "gone before its time");
}

/// The bar along the bottom says how long is left. It has to start full, end
/// empty, and never run backwards — a countdown that grows is worse than none.
#[test]
fn the_countdown_runs_down_and_stops() {
    use limen_ui::toast::{remaining, DWELL};

    let born = 100.0;
    assert_eq!(remaining(born, born), 1.0, "full the moment it appears");
    assert!(
        (remaining(born, born + DWELL / 2.0) - 0.5).abs() < 0.01,
        "half way at half time"
    );
    assert_eq!(remaining(born, born + DWELL), 0.0, "empty when its time is up");
    // Past its time it stays empty rather than going negative.
    assert_eq!(remaining(born, born + 60.0), 0.0);
    // And never grows.
    let mut prev = 1.0;
    for i in 0..=50 {
        let r = remaining(born, born + i as f64 * 0.1);
        assert!(r <= prev, "the bar grew at {i}");
        prev = r;
    }
}

/// Every notice is the same width, and nothing is drawn outside its box.
///
/// A frame is as wide as its content unless told otherwise, and the bar along
/// the bottom was measured from the area instead — which is as wide as the area
/// was *allowed* to be. So a short notice got a narrow box with a full-width bar
/// through it, and the stack had a ragged right edge. Both are invisible to a
/// test that only asks whether a notice appeared, and obvious in a screenshot.
#[test]
fn every_notice_is_one_width_and_nothing_leaves_its_box() {
    fn rects(s: &egui::Shape, out: &mut Vec<egui::epaint::RectShape>) {
        match s {
            egui::Shape::Rect(r) => out.push(*r),
            egui::Shape::Vec(v) => v.iter().for_each(|s| rects(s, out)),
            _ => {}
        }
    }

    let ctx = egui::Context::default();
    // Text lengths chosen to disagree: one word, one that wraps, one between.
    let notices = [
        (Level::Info, "short"),
        (Level::Ok, "a much longer notice, raised from the UI Kit, which wraps"),
        (Level::Warning, "mid length one"),
        (Level::Error, "x"),
    ];
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        for (level, text) in notices {
            limen_ui::toast::notify(ctx, level, text);
        }
        limen_ui::toast::draw(ctx);
    });
    let out = ctx.run(egui::RawInput::default(), limen_ui::toast::draw);

    let mut all = Vec::new();
    for clipped in &out.shapes {
        rects(&clipped.shape, &mut all);
    }
    // The boxes are the ones with a border; everything else is painted inside.
    let (boxes, painted): (Vec<&egui::epaint::RectShape>, Vec<&egui::epaint::RectShape>) =
        all.iter().partition(|r| r.stroke.width > 0.0);
    assert_eq!(boxes.len(), notices.len(), "one bordered box per notice");

    let width = boxes[0].rect.width();
    for b in &boxes {
        assert_eq!(b.rect.width(), width, "the stack has a ragged edge");
    }

    // The countdown bar and the leading edge: inside the border, not over it.
    for p in &painted {
        let home = boxes
            .iter()
            .find(|b| b.rect.expand(0.5).contains_rect(p.rect))
            .map(|b| b.rect);
        assert!(home.is_some(), "{:?} is drawn outside every box", p.rect);
        let b = home.unwrap();
        assert!(p.rect.right() <= b.right() - 1.0, "drawn over the right border");
        assert!(p.rect.bottom() <= b.bottom() - 1.0, "drawn over the bottom border");
        assert!(p.rect.left() >= b.left() + 1.0, "drawn over the left border");
    }
}
