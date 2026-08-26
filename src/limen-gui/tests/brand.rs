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

/// The mark: a rounded slab parted by a stepped seam.
///
/// The seam is the whole device — if the two leaves touch anywhere, or the step
/// pinches the gap as it crosses, it stops reading as a door drawn open — so
/// what is checked here is the gap, at every height, on both runs and the step.
#[test]
fn the_two_leaves_are_parted_evenly_all_the_way_down() {
    const GAP: f32 = 14.0;
    let heights = || (0..=1520).map(|i| SLAB + i as f32 / 10.0);

    for y in heights() {
        let (near, far) = (seam_at(y, LEAF_NEAR), seam_at(y, LEAF_FAR));
        // Both leaves have width everywhere: the slab's rounded edge never
        // reaches the seam, so neither leaf is pinched off at the corners.
        let (left, right) = (SLAB + slab_inset(y), 256.0 - SLAB - slab_inset(y));
        assert!(left < near, "the near leaf closed up at y={y}");
        assert!(far < right, "the far leaf closed up at y={y}");
        // Measured across, the leaves are never nearer than the gap.
        assert!(far - near >= GAP - 0.01, "the seam pinched at y={y}");
    }

    // Above and below the step, the two edges are vertical and the gap is what
    // it is measured across.
    for y in [SLAB, 100.0, 150.0, 204.0] {
        let gap = seam_at(y, LEAF_FAR) - seam_at(y, LEAF_NEAR);
        assert!((gap - GAP).abs() < 0.01, "gap {gap} at y={y}");
    }

    // Over the step both edges slant together, so the gap measured across grows
    // — but measured *perpendicular* to them it is the same gap, which is what
    // keeps the seam an even cut rather than a wedge.
    let rise = LEAF_NEAR.2 - LEAF_NEAR.1;
    let slant = SEAM_JOG / (SEAM_JOG * SEAM_JOG + rise * rise).sqrt();
    for y in [122.0, 128.0, 135.0] {
        let across = seam_at(y, LEAF_FAR) - seam_at(y, LEAF_NEAR);
        let perpendicular = across * (1.0 - slant * slant).sqrt();
        assert!(
            (perpendicular - GAP).abs() < 0.5,
            "perpendicular gap {perpendicular} at y={y}"
        );
    }
}

/// The slab's corners: straight down the middle of each side, pulled fully in at
/// the very top and bottom, and never beyond the radius.
#[test]
fn the_slab_rounds_only_at_its_corners() {
    assert_eq!(slab_inset(128.0), 0.0, "straight through the middle");
    assert!((slab_inset(SLAB) - SLAB_R).abs() < 0.01, "fully in at the top");
    assert!(
        (slab_inset(256.0 - SLAB) - SLAB_R).abs() < 0.01,
        "and at the bottom"
    );
    for i in 0..=152 {
        let inset = slab_inset(SLAB + i as f32);
        assert!((0.0..=SLAB_R).contains(&inset), "inset {inset} at row {i}");
    }
}

/// Every height the leaves are sampled at is inside the slab, in order, and the
/// heights the step turns at are among them — sample past a corner and it gets
/// mitred off.
#[test]
fn the_sampled_rows_span_the_slab_and_catch_every_corner() {
    let ys = seam_rows();
    assert_eq!(ys.first().copied(), Some(SLAB));
    assert_eq!(ys.last().copied(), Some(256.0 - SLAB));
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "sorted, no duplicates");
    for turn in [LEAF_NEAR.1, LEAF_NEAR.2, LEAF_FAR.1, LEAF_FAR.2] {
        assert!(ys.contains(&turn), "the step's corner at {turn} was not sampled");
    }
}
