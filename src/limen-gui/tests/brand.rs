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
