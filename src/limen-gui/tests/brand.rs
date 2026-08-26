//! The Barracuda mark, parsed from its SVG.

use limen_gui::app::*;

#[test]
fn barracuda_svg_parses_into_facets() {
    let art = barracuda_art();
    // All 53 <path> facets parse into drawable polygons (curves flattened).
    assert_eq!(art.polys.len(), 53, "expected 53 drawable facets");
    assert!(art.polys.iter().all(|p| p.len() >= 3));
    // Bounding box must sit inside the SVG viewBox (418,31 .. 1118,731).
    assert!(art.min.x >= 418.0 && art.min.y >= 31.0, "min {:?}", art.min);
    assert!(
        art.max.x <= 1118.0 && art.max.y <= 731.0,
        "max {:?}",
        art.max
    );
    assert!(art.max.x - art.min.x > 100.0 && art.max.y - art.min.y > 100.0);
}

/// The mark: two brackets, and the gap between them.
///
/// The gap is the whole device — a *limen* is not the door, it is the part you
/// cross — so what is checked here is that nothing ever grows into it, at any
/// size, from either side.
#[test]
fn nothing_reaches_into_the_gap() {
    for near in [true, false] {
        for [x0, _, x1, _] in bracket_rects(near) {
            assert!(x0 < x1, "empty rectangle");
            if near {
                assert!(x1 <= ARM_NEAR, "the near bracket reached to {x1}");
            } else {
                assert!(x0 >= ARM_FAR, "the far bracket reached to {x0}");
            }
        }
        for [x, _] in bracket_outline(near) {
            let inside = x > ARM_NEAR && x < ARM_FAR;
            assert!(!inside, "an outline corner at {x} stands in the gap");
        }
    }
}

/// ...and that what does stand in the gap stays inside it, clear of both arms.
#[test]
fn the_marks_in_the_gap_stand_clear_of_the_brackets() {
    for [x0, y0, x1, y1] in gap_marks() {
        assert!(x0 > ARM_NEAR, "mark starts at {x0}, inside the near bracket");
        assert!(x1 < ARM_FAR, "mark ends at {x1}, inside the far bracket");
        assert!(y0 >= OUT_NEAR && y1 <= OUT_FAR, "mark runs off the device");
        assert!(x0 < x1 && y0 < y1, "empty mark");
    }
}

/// The device is symmetric about both axes. It is a pair of brackets: if the
/// two halves are not mirrors, the eye reads it as a mistake rather than as a
/// gap, and no amount of drawing skill recovers that.
#[test]
fn the_two_halves_mirror_each_other() {
    const MID: f32 = 128.0;
    let near = bracket_rects(true);
    let far = bracket_rects(false);
    for (n, f) in near.iter().zip(far.iter()) {
        // Mirroring the near piece in x lands exactly on the far one.
        assert!((n[0] - (2.0 * MID - f[2])).abs() < 0.01, "{n:?} vs {f:?}");
        assert!((n[2] - (2.0 * MID - f[0])).abs() < 0.01, "{n:?} vs {f:?}");
        assert_eq!((n[1], n[3]), (f[1], f[3]), "the arms sit at different heights");
    }
    let marks = gap_marks();
    for [x0, _, x1, _] in marks {
        assert!((x0 + x1 - 2.0 * MID).abs() < 0.01, "mark off-centre: {x0}..{x1}");
    }
    // The two ticks are a mirrored pair; the crossing sits on the centre line.
    assert!((marks[0][1] + marks[2][3] - 2.0 * MID).abs() < 0.01, "ticks unpaired");
    assert!((marks[1][1] + marks[1][3] - 2.0 * MID).abs() < 0.01, "crossing off-centre");
}

/// A bracket is painted as three rectangles. They must not overlap: at full
/// opacity an overlap is invisible, and the moment the splash fades the mark
/// the doubled paint shows up as a bright band across the corner.
#[test]
fn the_pieces_of_a_bracket_never_overlap() {
    for near in [true, false] {
        let r = bracket_rects(near);
        for (i, a) in r.iter().enumerate() {
            for b in r.iter().skip(i + 1) {
                let overlap = a[0].max(b[0]) < a[2].min(b[2]) && a[1].max(b[1]) < a[3].min(b[3]);
                assert!(!overlap, "{a:?} overlaps {b:?}");
            }
        }
    }
}

/// The outline the sheen runs along has to be the same shape the rectangles
/// fill, or the gleam floats off the edge it is meant to sit inside.
#[test]
fn the_outline_traces_the_shape_the_rectangles_fill() {
    for near in [true, false] {
        let rects = bracket_rects(near);
        let outline = bracket_outline(near);
        let bounds = |xs: Vec<f32>| {
            let lo = xs.iter().cloned().fold(f32::MAX, f32::min);
            let hi = xs.iter().cloned().fold(f32::MIN, f32::max);
            (lo, hi)
        };
        assert_eq!(
            bounds(rects.iter().flat_map(|r| [r[0], r[2]]).collect()),
            bounds(outline.iter().map(|p| p[0]).collect()),
            "the outline is a different width to the fill"
        );
        assert_eq!(
            bounds(rects.iter().flat_map(|r| [r[1], r[3]]).collect()),
            bounds(outline.iter().map(|p| p[1]).collect()),
            "the outline is a different height to the fill"
        );
    }
}

/// The window's mark and the taskbar's are one drawing, kept in two languages —
/// `draw_brand` here and `render()` in scripts/make-icon.py. Nothing but this
/// test stops them drifting apart, and the drift would only show up as an icon
/// that no longer matches the app.
#[test]
fn the_icon_script_draws_the_same_mark() {
    let script = include_str!("../../../scripts/make-icon.py");

    let number = |name: &str| -> f32 {
        let at = script
            .find(&format!("{name} = "))
            .unwrap_or_else(|| panic!("scripts/make-icon.py no longer defines {name}"));
        script[at + name.len() + 3..]
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .find(|s| !s.is_empty())
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("could not read {name} out of the script"))
    };

    for (name, ours) in [
        ("THICK", THICK),
        ("OUT_NEAR", OUT_NEAR),
        ("OUT_FAR", OUT_FAR),
        ("ARM_NEAR", ARM_NEAR),
        ("ARM_FAR", ARM_FAR),
        ("MARK_R", MARK_R),
    ] {
        let theirs = number(name);
        assert!(
            (ours - theirs).abs() < 0.001,
            "{name}: the app draws {ours}, the icon script draws {theirs}"
        );
    }

    // ...and the three marks in the gap, read out of the script's own tuples.
    let body = script
        .split_once("def gap_marks():")
        .expect("the script no longer has gap_marks()")
        .1;
    let theirs: Vec<f32> = body[..body.find(']').unwrap_or(body.len())]
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter(|s| !s.is_empty() && s.contains('.'))
        .filter_map(|s| s.parse().ok())
        .collect();
    let ours: Vec<f32> = gap_marks().iter().flatten().copied().collect();
    assert_eq!(theirs.len(), ours.len(), "the script draws a different number of marks");
    for (a, b) in ours.iter().zip(theirs.iter()) {
        assert!((a - b).abs() < 0.001, "gap marks differ: {ours:?} vs {theirs:?}");
    }
}
