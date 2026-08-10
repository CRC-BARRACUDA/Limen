//! The Limen mark, the Barracuda logo, and the startup splash.

use super::*;

/// The Barracuda logo, embedded as its source SVG (a flat set of straight-edged
/// facets — only `M`/`l`/`z` path commands, no curves), so it can be rendered
/// with the egui painter like the Limen mark instead of pulling in an SVG/image
/// dependency.
pub(crate) const BARRACUDA_SVG: &str = include_str!("../../../resources/barracuda-white.svg");

/// Parsed Barracuda artwork: each facet as a polygon of points, plus the overall
/// bounding box (for aspect-correct fitting into any target rect).
pub(crate) struct Barracuda {
    pub(crate) polys: Vec<Vec<[f32; 2]>>,
    pub(crate) min: egui::Pos2,
    pub(crate) max: egui::Pos2,
}

/// Parse one SVG path `d` string into a polyline of absolute points. Supports the
/// commands the logo actually uses — `M`/`m`, `L`/`l`, `H`/`h`, `V`/`v`, `C`/`c`
/// (cubic béziers, flattened to short segments), and `Z`/`z` — in both absolute
/// (uppercase) and relative (lowercase) forms, including implicit operand repeats.
pub(crate) fn parse_facet(d: &str) -> Vec<[f32; 2]> {
    let b = d.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut cur = [0.0f32, 0.0f32];
    let mut pts: Vec<[f32; 2]> = Vec::new();
    let mut cmd = 0u8;

    let read_number = |b: &[u8], i: &mut usize| -> Option<f32> {
        while *i < b.len() && matches!(b[*i], b',' | b' ' | b'\n' | b'\t' | b'\r') {
            *i += 1;
        }
        let start = *i;
        if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
            *i += 1;
        }
        while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == b'.') {
            *i += 1;
        }
        if *i == start {
            return None;
        }
        std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
    };

    while i < n {
        let c = b[i];
        if c.is_ascii_alphabetic() {
            cmd = c;
            i += 1;
            continue;
        }
        if matches!(c, b',' | b' ' | b'\n' | b'\t' | b'\r') {
            i += 1;
            continue;
        }
        let rel = cmd.is_ascii_lowercase();
        match cmd.to_ascii_uppercase() {
            b'H' => {
                let Some(x) = read_number(b, &mut i) else {
                    break;
                };
                cur[0] = if rel { cur[0] + x } else { x };
                pts.push(cur);
            }
            b'V' => {
                let Some(y) = read_number(b, &mut i) else {
                    break;
                };
                cur[1] = if rel { cur[1] + y } else { y };
                pts.push(cur);
            }
            b'C' => {
                let mut num = [0.0f32; 6];
                let mut ok = true;
                for slot in &mut num {
                    match read_number(b, &mut i) {
                        Some(v) => *slot = v,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    break;
                }
                let abs = |dx: f32, dy: f32| {
                    if rel {
                        [cur[0] + dx, cur[1] + dy]
                    } else {
                        [dx, dy]
                    }
                };
                let p0 = cur;
                let c1 = abs(num[0], num[1]);
                let c2 = abs(num[2], num[3]);
                let end = abs(num[4], num[5]);
                // Flatten the cubic into a few straight segments.
                const STEPS: usize = 8;
                for k in 1..=STEPS {
                    let t = k as f32 / STEPS as f32;
                    let u = 1.0 - t;
                    let bx = u * u * u * p0[0]
                        + 3.0 * u * u * t * c1[0]
                        + 3.0 * u * t * t * c2[0]
                        + t * t * t * end[0];
                    let by = u * u * u * p0[1]
                        + 3.0 * u * u * t * c1[1]
                        + 3.0 * u * t * t * c2[1]
                        + t * t * t * end[1];
                    pts.push([bx, by]);
                }
                cur = end;
            }
            // M / L (and their implicit repeats): a single coordinate pair. After
            // an `M`, implicit following pairs are linetos, so keep `cmd` as-is —
            // both M and L consume one pair here, which is the desired behaviour.
            _ => {
                let Some(x) = read_number(b, &mut i) else {
                    break;
                };
                let Some(y) = read_number(b, &mut i) else {
                    break;
                };
                cur = if rel {
                    [cur[0] + x, cur[1] + y]
                } else {
                    [x, y]
                };
                pts.push(cur);
            }
        }
    }
    pts
}

/// Parse (once) the embedded Barracuda SVG into paintable facets.
pub(crate) fn barracuda_art() -> &'static Barracuda {
    static ART: std::sync::OnceLock<Barracuda> = std::sync::OnceLock::new();
    ART.get_or_init(|| {
        let mut polys: Vec<Vec<[f32; 2]>> = Vec::new();
        // Each real facet lives in a `<path d="…">`; splitting on `<path` and
        // taking the first `d="` per chunk skips the metadata <g id="…"> nodes.
        for seg in BARRACUDA_SVG.split("<path").skip(1) {
            let Some(s) = seg.find("d=\"") else { continue };
            let rest = &seg[s + 3..];
            let Some(e) = rest.find('"') else { continue };
            let pts = parse_facet(&rest[..e]);
            if pts.len() >= 3 {
                polys.push(pts);
            }
        }
        let mut min = egui::pos2(f32::MAX, f32::MAX);
        let mut max = egui::pos2(f32::MIN, f32::MIN);
        for p in polys.iter().flatten() {
            min.x = min.x.min(p[0]);
            min.y = min.y.min(p[1]);
            max.x = max.x.max(p[0]);
            max.y = max.y.max(p[1]);
        }
        Barracuda { polys, min, max }
    })
}

/// Draw the Barracuda logo filled with `color`, fitted (aspect-correct, centered)
/// into `rect`.
pub(crate) fn draw_barracuda(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let art = barracuda_art();
    let src = art.max - art.min;
    if src.x <= 0.0 || src.y <= 0.0 {
        return;
    }
    let scale = (rect.width() / src.x).min(rect.height() / src.y);
    let drawn = src * scale;
    let origin = rect.center() - drawn * 0.5;
    let map = |p: &[f32; 2]| {
        egui::pos2(
            origin.x + (p[0] - art.min.x) * scale,
            origin.y + (p[1] - art.min.y) * scale,
        )
    };
    for poly in &art.polys {
        let pts: Vec<egui::Pos2> = poly.iter().map(map).collect();
        painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
    }
}

/// Splash phase durations (seconds): a fully-transparent blank lead (so the
/// window's clumsy opaque startup frames read as transparent and are trimmable),
/// then fade transparent→opaque, hold fully opaque, then the exit animation.
/// Their sum is the total splash length.
pub(crate) const SPLASH_LEAD: f32 = 0.4;
pub(crate) const SPLASH_FADE_IN: f32 = 0.2;
pub(crate) const SPLASH_HOLD: f32 = 0.75;
pub(crate) const SPLASH_ANIM: f32 = 0.25;
pub(crate) const SPLASH_SECS: f32 = SPLASH_LEAD + SPLASH_FADE_IN + SPLASH_HOLD + SPLASH_ANIM;

/// Paint the startup splash: the Limen mark + Barracuda logo centre-screen. When
/// `animated`, it's fully transparent for `SPLASH_LEAD`, then fades in over
/// `SPLASH_FADE_IN`, holds for `SPLASH_HOLD`, then runs the `SPLASH_ANIM` exit
/// (scanline sweep + fade out). When not, the marks simply appear (opaque, no
/// motion) after the transparent lead. `t` is elapsed seconds since it began.
pub(crate) fn splash_screen(ctx: &egui::Context, t: f32, animated: bool) {
    egui::CentralPanel::default()
        // Transparent frame — the framebuffer is cleared transparent during the
        // splash (see `clear_color`), so only the icons show over the desktop.
        .frame(egui::Frame::none())
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            // Envelope: blank transparent lead, then either fade+scale in / hold /
            // fade out (animated) or a plain opaque appearance (static). `vis`
            // fades each element's own alpha (the window is transparent — there's
            // no background to veil against).
            let (vis, scale) = if animated {
                let vin = ui::ease_out(((t - SPLASH_LEAD) / SPLASH_FADE_IN).clamp(0.0, 1.0));
                let exit = ((t - (SPLASH_LEAD + SPLASH_FADE_IN + SPLASH_HOLD)) / SPLASH_ANIM)
                    .clamp(0.0, 1.0);
                let vout = 1.0 - ui::smoothstep(exit);
                (vin * vout, 0.92 + 0.08 * vin)
            } else {
                (if t >= SPLASH_LEAD { 1.0 } else { 0.0 }, 1.0)
            };

            let mark = 168.0 * scale;
            let gap = 44.0;
            let cx = rect.center().x;
            let cy = rect.center().y - 8.0;
            let half = mark / 2.0 + gap / 2.0 + 0.5;
            let painter = ui.painter();

            // Left: Limen mark · amber divider · right: Barracuda logo.
            let r1 =
                egui::Rect::from_center_size(egui::pos2(cx - half, cy), egui::vec2(mark, mark));
            let r2 =
                egui::Rect::from_center_size(egui::pos2(cx + half, cy), egui::vec2(mark, mark));
            draw_brand(painter, r1, vis, false);
            draw_barracuda(painter, r2, ui::with_alpha(ui::color::ACCENT_BRIGHT, vis));
            let dh = 92.0 * scale;
            painter.vline(
                cx,
                (cy - dh / 2.0)..=(cy + dh / 2.0),
                egui::Stroke::new(1.0_f32, ui::with_alpha(ui::color::ACCENT, 0.4 * vis)),
            );

            // A single amber scanline sweeps down across the marks (animated only),
            // over the hold into the start of the exit.
            if animated {
                let scan = ((t - SPLASH_LEAD - SPLASH_FADE_IN) / SPLASH_HOLD).clamp(0.0, 1.0);
                if scan > 0.0 && scan < 1.0 {
                    let y = egui::lerp((cy - mark * 0.7)..=(cy + mark * 0.7), scan);
                    let a = (1.0 - (scan * 2.0 - 1.0).abs()) * 0.5 * vis;
                    painter.hline(
                        (cx - mark * 1.4)..=(cx + mark * 1.4),
                        y,
                        egui::Stroke::new(1.5_f32, ui::with_alpha(ui::color::ACCENT_BRIGHT, a)),
                    );
                }
            }
        });
}

/// Draw the Limen brand mark — the diamond ring with a solid core — retinted to
/// the Barracuda amber-HUD palette: a warm near-black tile under an amber→orange
/// diagonal-gradient diamond, so it reads as one product with the Barracuda logo.
///
/// (The OS icon files — `resources/icon.png`/`.ico` — still carry the original
/// cyan mark; regenerating those needs image tooling and is a separate step.)
pub(crate) fn draw_brand(painter: &egui::Painter, rect: egui::Rect, alpha: f32, show_tile: bool) {
    use egui::{Color32, Mesh, Pos2, Shape};

    // Scale every colour's alpha by `alpha` (1.0 = opaque) so the mark can fade
    // as a whole — used by the transparent startup splash.
    let fa = |c: Color32| {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * alpha).round() as u8)
    };
    // Diamond: one diagonal gradient across the whole device — light at the
    // top-left, dark at the bottom-right — so the top *and left* vertices are
    // bright amber while the right and bottom ones deepen to orange.
    let light = Color32::from_rgb(0xf4, 0xc0, 0x78); // bright amber
    let dark = Color32::from_rgb(0xf9, 0x73, 0x16); // orange

    let s = rect.width().min(rect.height()) / 256.0;

    // The dark rounded tile (the app-icon background). Skipped for the splash,
    // where the mark floats transparently beside the Barracuda logo.
    if show_tile {
        let tile_top = fa(Color32::from_rgb(0x24, 0x1a, 0x10));
        let tile_bottom = fa(Color32::from_rgb(0x0d, 0x0a, 0x06));
        let tile = egui::Rect::from_center_size(rect.center(), egui::vec2(224.0 * s, 224.0 * s));
        painter.add(Shape::mesh(rounded_rect_mesh(
            tile,
            44.0 * s,
            tile_top,
            tile_bottom,
        )));
    }

    let c = rect.center();
    let diamond = |half: f32| {
        [
            Pos2::new(c.x, c.y - half),
            Pos2::new(c.x + half, c.y),
            Pos2::new(c.x, c.y + half),
            Pos2::new(c.x - half, c.y),
        ]
    };
    let outer = diamond(88.0 * s);
    let inner = diamond(48.0 * s);
    let core = diamond(20.0 * s);

    // Position along the top-left → bottom-right diagonal, normalized to 0..1.
    let span = 2.0 * 88.0 * s;
    let shade = |p: Pos2| {
        fa(lerp_color(
            light,
            dark,
            ((p.x - c.x) + (p.y - c.y)) / span + 0.5,
        ))
    };

    let mut mesh = Mesh::default();
    let mut quad = |pts: [Pos2; 4]| {
        let base = mesh.vertices.len() as u32;
        for p in pts {
            mesh.colored_vertex(p, shade(p));
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    };
    // The ring as four convex trapezoids. Building it this way — rather than
    // punching a hole with the background color — keeps the tile's own gradient
    // visible through the middle.
    for i in 0..4 {
        let j = (i + 1) % 4;
        quad([outer[i], outer[j], inner[j], inner[i]]);
    }
    // The core is not a lighter accent: it's the same gradient, continued.
    quad(core);
    painter.add(Shape::mesh(mesh));

    // A flat highlight runs just inside the outer edge — the one part of the
    // device that doesn't follow the gradient. It measures ~1.4px on the icon's
    // 256px grid, so at the sizes drawn here it lands sub-pixel and reads as a
    // faint sheen along the edge rather than a distinct line.
    painter.add(Shape::closed_line(
        outer.to_vec(),
        egui::Stroke::new(1.4 * s, fa(Color32::from_rgb(0xff, 0xe0, 0xb0))),
    ));
}

/// Linear blend between two colors, `t` clamped to `0..=1`.
pub(crate) fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// A rounded rectangle as a colored mesh, filled with a vertical `top`→`bottom`
/// gradient. `egui`'s `rect_filled` takes a single color, so a gradient fill has
/// to be built by hand: walk the outline, then fan-triangulate from the center
/// (the shape is convex, so a fan is exact).
pub(crate) fn rounded_rect_mesh(
    rect: egui::Rect,
    radius: f32,
    top: egui::Color32,
    bottom: egui::Color32,
) -> egui::Mesh {
    /// Segments per corner arc — enough to look round at the sizes we draw.
    const SEGMENTS: usize = 8;

    let radius = radius.min(rect.width().min(rect.height()) / 2.0);
    // Arc centers with their start/end angles, walked clockwise from top-left.
    let corners = [
        (
            egui::pos2(rect.left() + radius, rect.top() + radius),
            180.0f32,
            270.0f32,
        ),
        (
            egui::pos2(rect.right() - radius, rect.top() + radius),
            270.0,
            360.0,
        ),
        (
            egui::pos2(rect.right() - radius, rect.bottom() - radius),
            0.0,
            90.0,
        ),
        (
            egui::pos2(rect.left() + radius, rect.bottom() - radius),
            90.0,
            180.0,
        ),
    ];
    let mut outline = Vec::with_capacity(4 * (SEGMENTS + 1));
    for (center, from, to) in corners {
        for i in 0..=SEGMENTS {
            let a = (from + (to - from) * i as f32 / SEGMENTS as f32).to_radians();
            outline.push(egui::pos2(
                center.x + radius * a.cos(),
                center.y + radius * a.sin(),
            ));
        }
    }

    let shade = |y: f32| lerp_color(top, bottom, (y - rect.top()) / rect.height());
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), shade(rect.center().y));
    for p in &outline {
        mesh.colored_vertex(*p, shade(p.y));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    mesh
}

#[cfg(test)]
mod barracuda_tests {
    use super::*;

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
}
