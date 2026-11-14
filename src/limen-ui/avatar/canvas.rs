//! What the effects draw with: the handful of canvas-2D operations the UI
//! kit's effects use, done with egui shapes, in the same coordinates — canvas
//! pixels, the avatar's centre at `extent / 2`.
//!
//! * `globalAlpha` is the `alpha` every call takes.
//! * `globalCompositeOperation = 'lighter'` is [`Blend::Add`]: a premultiplied
//!   colour with no alpha adds itself to what is under it.
//! * `shadowBlur` is drawn as a few wider, fainter passes under a stroke.
//! * A conic gradient along a stroke is a mesh with a colour per point.
//! * A face drawn torn, tinted or inverted is the face texture, cut by UV.

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

/// A colour as an effect has it: straight (not premultiplied), with alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ink {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Ink {
    pub const WHITE: Ink = Ink { r: 255, g: 255, b: 255, a: 1.0 };
    pub const BLACK: Ink = Ink { r: 0, g: 0, b: 0, a: 1.0 };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// `#rgb` / `#rrggbb`; anything else is None (the spec was checked, the
    /// defaults are ours — this only guards the code against itself).
    pub fn hex(s: &str) -> Option<Self> {
        let h = s.strip_prefix('#')?;
        let v = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
        match h.len() {
            3 => Some(Self::rgb(v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17)),
            6 => Some(Self::rgb(v(0, 2)?, v(2, 2)?, v(4, 2)?)),
            _ => None,
        }
    }

    /// As painted with `alpha` on top, the usual way.
    pub fn over(self, alpha: f32) -> Color32 {
        let a = (self.a * alpha).clamp(0.0, 1.0);
        Color32::from_rgba_unmultiplied(self.r, self.g, self.b, (a * 255.0).round() as u8)
    }

    /// As painted with `alpha`, adding light to what is under it.
    pub fn lighter(self, alpha: f32) -> Color32 {
        let a = (self.a * alpha).clamp(0.0, 1.0);
        let k = |c: u8| (c as f32 * a).round() as u8;
        Color32::from_rgba_premultiplied(k(self.r), k(self.g), k(self.b), 0)
    }

    pub fn paint(self, alpha: f32, blend: Blend) -> Color32 {
        match blend {
            Blend::Over => self.over(alpha),
            Blend::Add => self.lighter(alpha),
        }
    }

    /// Between `self` (t = 0) and `to` (t = 1).
    pub fn mix(self, to: Ink, t: f32) -> Ink {
        let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Ink { r: l(self.r, to.r), g: l(self.g, to.g), b: l(self.b, to.b), a: self.a + (to.a - self.a) * t }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    /// `source-over`.
    Over,
    /// `lighter`.
    Add,
}

/// The face an effect can draw from.
#[derive(Clone)]
pub enum Face {
    /// Not yet, or never: nothing to distort.
    None,
    /// The picture, square, and its inverse.
    Picture { still: egui::TextureId, inverted: egui::TextureId },
    /// The initials tile.
    Initials(String),
}

/// The face's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceShape {
    Circle,
    Square,
    Rounded,
}

/// One effect's drawing surface for one frame.
pub struct Canvas {
    painter: egui::Painter,
    /// Where canvas (0, 0) is on screen.
    origin: Pos2,
    pub face: Face,
    pub face_shape: FaceShape,
    /// The face's rect in canvas pixels.
    pub face_rect: Rect,
}

impl Canvas {
    pub fn new(painter: egui::Painter, origin: Pos2, face: Face, face_shape: FaceShape, face_rect: Rect) -> Self {
        Self { painter, origin, face, face_shape, face_rect }
    }

    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.origin + Vec2::new(x, y)
    }

    fn screen_rect(&self, r: Rect) -> Rect {
        r.translate(self.origin.to_vec2())
    }

    /// Draw inside the face only, as the effects' `clipFace`. A rounded or round
    /// face is clipped to its square: no effect that clips is drawn over one.
    pub fn clipped_to_face(&self) -> Canvas {
        let clip = self.screen_rect(self.face_rect).intersect(self.painter.clip_rect());
        Canvas {
            painter: self.painter.with_clip_rect(clip),
            origin: self.origin,
            face: self.face.clone(),
            face_shape: self.face_shape,
            face_rect: self.face_rect,
        }
    }

    /// `fillRect` under a `globalAlpha` and a composite operation.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_rect(&self, x: f32, y: f32, w: f32, h: f32, ink: Ink, alpha: f32, blend: Blend) {
        if w <= 0.0 || h <= 0.0 || alpha <= 0.0 {
            return;
        }
        let r = Rect::from_min_size(self.at(x, y), Vec2::new(w, h));
        self.painter.rect_filled(r, 0.0, ink.paint(alpha, blend));
    }

    pub fn fill_circle(&self, x: f32, y: f32, radius: f32, ink: Ink, alpha: f32) {
        self.painter.circle_filled(self.at(x, y), radius, ink.over(alpha));
    }

    /// `strokeRect` under a `globalAlpha`.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_rect(&self, x: f32, y: f32, w: f32, h: f32, width: f32, ink: Ink, alpha: f32) {
        let r = Rect::from_min_size(self.at(x, y), Vec2::new(w, h));
        self.painter.rect_stroke(r, 0.0, egui::Stroke::new(width, ink.over(alpha)));
    }

    /// A line through `points` (canvas pixels), with a soft glow of `blur` under
    /// it in `glow` — `shadowBlur`, as a few wider, fainter passes.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke(&self, points: &[Pos2], closed: bool, width: f32, ink: Ink, alpha: f32, blend: Blend, glow: Option<(Ink, f32)>) {
        if points.len() < 2 || alpha <= 0.0 {
            return;
        }
        let pts: Vec<Pos2> = points.iter().map(|p| self.at(p.x, p.y)).collect();
        if let Some((g, blur)) = glow {
            for (k, a) in [(1.0, 0.10), (0.6, 0.16), (0.3, 0.24)] {
                let w = width + blur * k;
                self.polyline(&pts, closed, w, g.lighter(alpha * a));
            }
        }
        self.polyline(&pts, closed, width, ink.paint(alpha, blend));
    }

    fn polyline(&self, pts: &[Pos2], closed: bool, width: f32, color: Color32) {
        let stroke = egui::Stroke::new(width, color);
        let shape = if closed {
            egui::Shape::closed_line(pts.to_vec(), stroke)
        } else {
            egui::Shape::line(pts.to_vec(), stroke)
        };
        self.painter.add(shape);
    }

    /// A line with a colour per point — a gradient along it — over a glow of
    /// one colour, as a canvas stroke with a gradient and a `shadowColor`.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_shaded(&self, points: &[(Pos2, Ink)], closed: bool, width: f32, alpha: f32, blend: Blend, glow: Option<(Ink, f32)>) {
        if points.len() < 2 || alpha <= 0.0 {
            return;
        }
        if let Some((g, blur)) = glow {
            let pts: Vec<Pos2> = points.iter().map(|(p, _)| self.at(p.x, p.y)).collect();
            for (k, a) in [(1.0, 0.10), (0.6, 0.16), (0.3, 0.24)] {
                self.polyline(&pts, closed, width + blur * k, g.lighter(alpha * a));
            }
        }
        self.ribbon(points, closed, width, |i| i.paint(alpha, blend));
    }

    fn ribbon(&self, points: &[(Pos2, Ink)], closed: bool, width: f32, color: impl Fn(Ink) -> Color32) {
        let mut mesh = egui::Mesh::default();
        let n = points.len();
        let segs = if closed { n } else { n - 1 };
        for i in 0..segs {
            let (a, ca) = points[i];
            let (b, cb) = points[(i + 1) % n];
            let (a, b) = (self.at(a.x, a.y), self.at(b.x, b.y));
            let d = b - a;
            let len = d.length();
            if len < 1e-3 {
                continue;
            }
            // A little longer than the segment, so neighbours overlap at joints.
            let along = d / len * (width * 0.25);
            let side = Vec2::new(-d.y, d.x) / len * (width / 2.0);
            let (a, b) = (a - along, b + along);
            let base = mesh.vertices.len() as u32;
            for (p, c) in [(a + side, ca), (a - side, ca), (b + side, cb), (b - side, cb)] {
                mesh.colored_vertex(p, color(c));
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
        self.painter.add(egui::Shape::mesh(mesh));
    }

    /// Part of the face — `uv` in 0..1 of it — into `dest` (canvas pixels),
    /// from the still or its inverse, tinted, as `drawImage` with a channel mask.
    pub fn face_part(&self, inverted: bool, uv: Rect, dest: Rect, tint: Color32) {
        let Face::Picture { still, inverted: inv } = &self.face else { return };
        let mut mesh = egui::Mesh::with_texture(if inverted { *inv } else { *still });
        mesh.add_rect_with_uv(self.screen_rect(dest), uv, tint);
        self.painter.add(egui::Shape::mesh(mesh));
    }

    /// Where a canvas point falls on the face, in 0..1 of it.
    fn face_uv(&self, p: Pos2) -> Pos2 {
        let f = self.face_rect;
        Pos2::new((p.x - f.min.x) / f.width(), (p.y - f.min.y) / f.height())
    }

    /// A line from `a` to `b` (canvas pixels) drawn in the face beneath it —
    /// the still or its inverse — rather than in a colour.
    pub fn face_line(&self, a: Pos2, b: Pos2, width: f32, inverted: bool, alpha: f32) {
        let Face::Picture { still, inverted: inv } = &self.face else { return };
        let d = b - a;
        let len = d.length();
        if len < 1e-3 {
            return;
        }
        let side = Vec2::new(-d.y, d.x) / len * (width / 2.0);
        let mut mesh = egui::Mesh::with_texture(if inverted { *inv } else { *still });
        let tint = Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8);
        let base = mesh.vertices.len() as u32;
        for p in [a + side, a - side, b + side, b - side] {
            mesh.vertices.push(egui::epaint::Vertex { pos: self.at(p.x, p.y), uv: self.face_uv(p), color: tint });
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
        self.painter.add(egui::Shape::mesh(mesh));
    }

    /// A filled rect (canvas pixels) in the face beneath it.
    pub fn face_fill(&self, x: f32, y: f32, w: f32, h: f32, inverted: bool) {
        let dest = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h));
        let uv = Rect::from_min_max(self.face_uv(dest.min), self.face_uv(dest.max));
        self.face_part(inverted, uv, dest, Color32::WHITE);
    }

    /// A band of the initials tile — rows `uv_y0..uv_y0 + uv_h` of it — into
    /// `dest`, tinted: the initials face, torn as `signal` tears a picture.
    pub fn initials_part(&self, text: &str, uv_y0: f32, dest: Rect, tint: Ink, blend: Blend) {
        let size = self.face_rect.width();
        let screen = self.screen_rect(dest);
        let clipped = self.painter.with_clip_rect(screen.intersect(self.painter.clip_rect()));
        let tile = Ink::rgb(0x1f, 0x16, 0x0d);
        let ink = Ink::rgb(0xf0, 0xa8, 0x50);
        let mul = |a: Ink| Ink::rgb(
            (a.r as u16 * tint.r as u16 / 255) as u8,
            (a.g as u16 * tint.g as u16 / 255) as u8,
            (a.b as u16 * tint.b as u16 / 255) as u8,
        );
        clipped.rect_filled(screen, 0.0, mul(tile).paint(1.0, blend));
        // The tile's centre, where row uv_y0 lands at the top of `dest`.
        let centre = Pos2::new(screen.min.x + size / 2.0, screen.min.y - uv_y0 * size + size / 2.0);
        clipped.text(
            centre,
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::monospace((size * 0.38).max(8.0)),
            mul(ink).paint(1.0, blend),
        );
    }

    /// Text running downwards from (x, y), glyph tops facing right — the
    /// canvas's `rotate(π/2)` + `fillText` at the alphabetic baseline. Its length.
    pub fn text_down(&self, x: f32, y: f32, text: &str, px: f32, ink: Ink, alpha: f32) -> f32 {
        let galley = self.painter.layout_no_wrap(text.to_owned(), egui::FontId::monospace(px), ink.over(alpha));
        let (w, h) = (galley.size().x, galley.size().y);
        // Turned a quarter clockwise about its top-left corner, the galley's top
        // edge runs down the screen at x; its baseline, ~0.8 of the row down,
        // is to the left of that — so the corner goes that far to the right.
        let shape = egui::epaint::TextShape::new(self.at(x + h * 0.8, y), galley, ink.over(alpha))
            .with_angle(std::f32::consts::FRAC_PI_2);
        self.painter.add(shape);
        w
    }

    /// Text with its bottom-left at (x, y): `textBaseline = 'bottom'`.
    pub fn text_above(&self, x: f32, y: f32, text: &str, px: f32, color: Color32) {
        self.painter.text(self.at(x, y), egui::Align2::LEFT_BOTTOM, text, egui::FontId::monospace(px), color);
    }

    /// The initials, as the face tile has them, in `ink`.
    pub fn initials(&self, text: &str, centre: Pos2, size: f32, ink: Color32) {
        self.painter.text(
            self.at(centre.x, centre.y),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::monospace(size),
            ink,
        );
    }

    /// How wide `text` is at `px`.
    pub fn text_width(&self, text: &str, px: f32) -> f32 {
        self.painter
            .layout_no_wrap(text.to_owned(), egui::FontId::monospace(px), Color32::WHITE)
            .size()
            .x
    }
}
