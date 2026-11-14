//! The code effects, ported from the UI kit (`components/avatar/effects`)
//! one for one: the same geometry, timings, odds and colours, drawn with
//! [`Canvas`] instead of a 2D context. A spec names an effect; it never
//! carries one — these are the only things a decoration can make move.
//!
//! Every effect is pure drawing (the kit's `maxAlive` is 0 for all of them),
//! so the kit's page-wide particle budget has nothing to count here.

use eframe::egui::{Pos2, Rect, Vec2};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use super::canvas::{Blend, Canvas, Face, FaceShape, Ink};

/// Effects are authored against a 64px avatar.
pub const REFERENCE_SIZE: f32 = 64.0;

/// The effects, as a spec names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectKind {
    Aurora,
    Blobinvert,
    Blobtrack,
    Glitch,
    Neon,
    Neonsplit,
    Signal,
    Tracking,
}

impl EffectKind {
    pub const ALL: [EffectKind; 8] = [
        EffectKind::Aurora,
        EffectKind::Blobinvert,
        EffectKind::Blobtrack,
        EffectKind::Glitch,
        EffectKind::Neon,
        EffectKind::Neonsplit,
        EffectKind::Signal,
        EffectKind::Tracking,
    ];

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "aurora" => Self::Aurora,
            "blobinvert" => Self::Blobinvert,
            "blobtrack" => Self::Blobtrack,
            "glitch" => Self::Glitch,
            "neon" => Self::Neon,
            "neonsplit" => Self::Neonsplit,
            "signal" => Self::Signal,
            "tracking" => Self::Tracking,
            _ => return None,
        })
    }

    /// Used when the spec gives no colours.
    pub fn default_colors(self) -> &'static [&'static str] {
        match self {
            Self::Aurora => &["#50dcff", "#b36bff", "#ff8a3c"],
            Self::Blobinvert | Self::Blobtrack | Self::Tracking => &["#ffffff"],
            Self::Glitch => &["#ff2bd6", "#2be8ff", "#ffffff"],
            Self::Neon => &["#ff2bd6", "#8a5cff", "#2bb8ff"],
            Self::Neonsplit => &["#50dcff", "#ff8a3c"],
            Self::Signal => &["#ff2bd6", "#2be8ff"],
        }
    }

    /// The face shape the effect is designed around; square when it has none.
    pub fn face_shape(self) -> FaceShape {
        match self {
            Self::Neon => FaceShape::Rounded,
            _ => FaceShape::Square,
        }
    }
}

/// A small, fast random source: xorshift64*. Seeded per avatar.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    /// 0..1, as `Math.random()`.
    pub fn unit(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// What an effect is made with.
#[derive(Debug, Clone)]
pub struct Env {
    /// Avatar side, px.
    pub size: f32,
    /// Canvas side, px; the avatar's centre is `extent / 2`.
    pub extent: f32,
    /// `size / 64`.
    pub scale: f32,
    pub colors: Vec<Ink>,
    /// The colours came from the spec rather than the defaults.
    pub custom_colors: bool,
    /// 0..1.
    pub intensity: f32,
}

impl Env {
    /// For `kind` at `size`, with the spec's colours (already checked) or the
    /// effect's own.
    pub fn new(kind: EffectKind, size: f32, colors: &[String], intensity: f32) -> Self {
        let pad = (size / 2.0).round();
        let custom: Vec<Ink> = colors.iter().filter_map(|c| Ink::hex(c)).collect();
        let custom_colors = !custom.is_empty();
        let colors = if custom_colors {
            custom
        } else {
            kind.default_colors().iter().filter_map(|c| Ink::hex(c)).collect()
        };
        Self { size, extent: size + pad * 2.0, scale: size / REFERENCE_SIZE, colors, custom_colors, intensity }
    }

    fn color(&self, i: usize) -> Ink {
        self.colors[i % self.colors.len()]
    }
}

/// The contract every effect keeps: advance by a step of at most 50ms, draw
/// in canvas pixels, and answer a click.
pub trait Effect: Send {
    /// `pointer` is in canvas px while an interactive avatar is hovered.
    fn update(&mut self, dt_ms: f32, pointer: Option<Pos2>);
    fn draw(&self, c: &Canvas);
    fn press(&mut self, _at: Pos2) {}
}

/// Build the effect a spec names.
pub fn create(kind: EffectKind, env: &Env, seed: u64) -> Box<dyn Effect> {
    let rng = Rng::new(seed);
    match kind {
        EffectKind::Aurora => Box::new(Aurora::new(env)),
        EffectKind::Glitch => Box::new(Glitch::new(env, rng, None)),
        EffectKind::Neon => Box::new(Neon::new(env)),
        EffectKind::Neonsplit => Box::new(Neonsplit::new(env, rng)),
        EffectKind::Signal => Box::new(Signal::new(env, rng)),
        EffectKind::Tracking => Box::new(Tracking::new(env, rng)),
        EffectKind::Blobtrack => Box::new(Blobtrack::new(env, rng, false)),
        EffectKind::Blobinvert => Box::new(Blobtrack::new(env, rng, true)),
    }
}

/// Ease a hover level towards 1 or 0 over ~200ms rather than snapping.
fn ease_hover(hover: &mut f32, pointer: Option<Pos2>, dt_ms: f32) {
    let target = if pointer.is_some() { 1.0 } else { 0.0 };
    *hover += (target - *hover) * (dt_ms / 200.0).min(1.0);
}

/// The colour a gradient of evenly spaced `stops` has at `t` (0..1).
fn gradient_at(stops: &[(f32, Ink)], t: f32) -> Ink {
    let t = t.rem_euclid(1.0);
    for w in stops.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if t >= a && t <= b {
            return ca.mix(cb, if b > a { (t - a) / (b - a) } else { 0.0 });
        }
    }
    stops.last().map(|s| s.1).unwrap_or(Ink::WHITE)
}

/// A conic gradient's colour at `p`, starting at `angle`, clockwise.
fn conic(stops: &[(f32, Ink)], c: f32, angle: f32, p: Pos2) -> Ink {
    let a = (p.y - c).atan2(p.x - c);
    gradient_at(stops, (a - angle) / TAU)
}

// --------------------------------------------------------------------------- //
// aurora
// --------------------------------------------------------------------------- //

/// Bands of coloured light flowing around a square avatar.
struct Aurora {
    env: Env,
    t: f32,
    hover: f32,
}

impl Aurora {
    fn new(env: &Env) -> Self {
        Self { env: env.clone(), t: 0.0, hover: 0.0 }
    }
}

/// The point a fraction `f` (wraps) of the way round a square of half-side
/// `half` centred on (c, c), clockwise from the top-left corner.
fn on_square(c: f32, half: f32, f: f32) -> Pos2 {
    let u = f.rem_euclid(1.0) * 4.0;
    let side = u.floor();
    let k = (u - side) * 2.0 - 1.0;
    match side as u8 {
        0 => Pos2::new(c + k * half, c - half),
        1 => Pos2::new(c + half, c + k * half),
        2 => Pos2::new(c - k * half, c + half),
        _ => Pos2::new(c - half, c - k * half),
    }
}

impl Effect for Aurora {
    fn update(&mut self, dt: f32, pointer: Option<Pos2>) {
        ease_hover(&mut self.hover, pointer, dt);
        self.t += (dt / 1000.0) * (0.6 + self.env.intensity * 0.8) * (1.0 + self.hover * 1.5);
    }

    fn draw(&self, cv: &Canvas) {
        let e = &self.env;
        let c = e.extent / 2.0;
        let bands = 3;
        for b in 0..bands {
            let bf = b as f32;
            let color = e.color(b);
            let half = e.size / 2.0 + (3.0 + bf * 2.5) * e.scale;
            let perimeter = half * 8.0;
            let start = (self.t * (0.16 + bf * 0.05) + bf / bands as f32).rem_euclid(1.0);
            let length = 0.28 + 0.12 * (self.t * 0.7 + bf).sin();
            let blur = (6.0 + 6.0 * e.intensity + 6.0 * self.hover) * e.scale;
            let alpha = 0.35 + 0.35 * e.intensity + 0.2 * self.hover;
            let width = ((1.5 + e.intensity * 2.0) * e.scale).max(1.0);
            let steps = ((length * perimeter / 2.0).ceil() as usize).max(8);
            let points: Vec<Pos2> =
                (0..=steps).map(|i| on_square(c, half, start + length * i as f32 / steps as f32)).collect();
            cv.stroke(&points, false, width, color, alpha, Blend::Add, Some((color, blur)));
        }
    }
}

// --------------------------------------------------------------------------- //
// glitch
// --------------------------------------------------------------------------- //

type Seg = (f32, f32);

#[derive(Clone)]
struct Band {
    y0: f32,
    y1: f32,
    dx: f32,
}

#[derive(Clone)]
struct Bar {
    y: f32,
    h: f32,
    color: usize,
    alpha: f32,
}

/// "Signal lost": a square frame split into RGB channels, with interference
/// bursts that tear it sideways, break its edges and flicker bars across the face.
struct Glitch {
    rng: Rng,
    intensity: f32,
    scale: f32,
    side: f32,
    x0: f32,
    y0: f32,
    thick: f32,
    inks: [Ink; 3],
    idle_edges: Vec<Vec<Seg>>,
    burst_left: f32,
    reshuffle: f32,
    edges: Vec<Vec<Seg>>,
    bands: Vec<Band>,
    bars: Vec<Bar>,
    split: f32,
    rate: f32,
}

/// An edge as solid segments with `gaps` random holes.
fn cut(rng: &mut Rng, gaps: usize) -> Vec<Seg> {
    let mut holes: Vec<Seg> = (0..gaps)
        .map(|_| {
            let at = 0.1 + rng.unit() * 0.8;
            let w = 0.03 + rng.unit() * 0.1;
            (at, (at + w).min(1.0))
        })
        .collect();
    holes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut segs = Vec::new();
    let mut from = 0.0f32;
    for (a, b) in holes {
        if a > from {
            segs.push((from, a));
        }
        from = from.max(b);
    }
    if from < 1.0 {
        segs.push((from, 1.0));
    }
    segs
}

impl Glitch {
    /// `core` replaces the third colour (neonsplit's faint one).
    fn new(env: &Env, mut rng: Rng, core: Option<Ink>) -> Self {
        let c = env.extent / 2.0;
        let half = env.size / 2.0 + 4.0 * env.scale;
        let idle_edges: Vec<Vec<Seg>> = (0..4).map(|_| cut(&mut rng, 1)).collect();
        Self {
            intensity: env.intensity,
            scale: env.scale,
            side: half * 2.0,
            x0: c - half,
            y0: c - half,
            thick: (2.2 * env.scale).max(1.5),
            inks: [env.color(0), env.color(1), core.unwrap_or_else(|| env.color(2))],
            edges: idle_edges.clone(),
            idle_edges,
            burst_left: 0.0,
            reshuffle: 0.0,
            bands: Vec::new(),
            bars: Vec::new(),
            split: env.scale,
            rate: 0.25 + env.intensity * 1.5,
            rng,
        }
    }

    fn reroll(&mut self, strength: f32) {
        let rng = &mut self.rng;
        self.edges = (0..4)
            .map(|_| {
                let gaps = 1 + (rng.unit() * 3.0).floor() as usize;
                cut(rng, gaps)
            })
            .collect();
        self.split = (1.5 + rng.unit() * 4.0 * strength) * self.scale;
        let n_bands = 1 + (rng.unit() * 3.0).floor() as usize;
        self.bands = (0..n_bands)
            .map(|_| {
                let y = self.y0 + rng.unit() * self.side;
                let h = (2.0 + rng.unit() * 10.0) * self.scale;
                Band { y0: y, y1: y + h, dx: (rng.unit() - 0.5) * 14.0 * strength * self.scale }
            })
            .collect();
        let n_bars = (rng.unit() * (2.0 + 3.0 * strength)).floor() as usize;
        self.bars = (0..n_bars)
            .map(|_| Bar {
                y: self.y0 + rng.unit() * self.side,
                h: ((0.5 + rng.unit() * 3.0) * self.scale).max(1.0),
                color: (rng.unit() * 3.0).floor() as usize % 3,
                alpha: 0.12 + rng.unit() * 0.25,
            })
            .collect();
    }

    fn calm(&mut self) {
        self.edges = self.idle_edges.clone();
        self.bands.clear();
        self.bars.clear();
        self.split = self.scale;
    }

    fn start_burst(&mut self, ms: f32, strength: f32) {
        self.burst_left = self.burst_left.max(ms);
        self.reshuffle = 0.0;
        self.reroll(strength);
    }

    /// The x offset for something at height y: the tear it falls in, if any.
    fn tear(&self, y: f32) -> f32 {
        self.bands.iter().find(|b| y >= b.y0 && y <= b.y1).map(|b| b.dx).unwrap_or(0.0)
    }

    fn frame(&self, cv: &Canvas, ink: Ink, ox: f32, alpha: f32) {
        let (x0, y0, side, thick) = (self.x0, self.y0, self.side, self.thick);
        for (edge, y) in [(0, y0), (2, y0 + side - thick)] {
            let dx = self.tear(y) + ox;
            for &(a, b) in &self.edges[edge] {
                cv.fill_rect(x0 + a * side + dx, y, (b - a) * side, thick, ink, alpha, Blend::Over);
            }
        }
        let mut stops: Vec<f32> = vec![y0, y0 + side];
        stops.extend(self.bands.iter().flat_map(|b| [b.y0, b.y1]));
        stops.retain(|y| *y >= y0 && *y <= y0 + side);
        stops.sort_by(f32::total_cmp);
        for (edge, x) in [(3, x0), (1, x0 + side - thick)] {
            for &(a, b) in &self.edges[edge] {
                let (ya, yb) = (y0 + a * side, y0 + b * side);
                for w in stops.windows(2) {
                    let top = ya.max(w[0]);
                    let bottom = yb.min(w[1]);
                    if bottom <= top {
                        continue;
                    }
                    cv.fill_rect(x + self.tear((top + bottom) / 2.0) + ox, top, thick, bottom - top, ink, alpha, Blend::Over);
                }
            }
        }
    }
}

impl Effect for Glitch {
    fn update(&mut self, dt: f32, pointer: Option<Pos2>) {
        let hovering = pointer.is_some();
        let chance = (if hovering { 8.0 } else { self.rate }) * (dt / 1000.0);
        if self.burst_left <= 0.0 && self.rng.unit() < chance {
            let ms = 80.0 + self.rng.unit() * 220.0;
            self.start_burst(ms, 0.4 + self.intensity * 0.6);
        }
        if self.burst_left > 0.0 {
            self.burst_left -= dt;
            self.reshuffle -= dt;
            if self.burst_left <= 0.0 {
                self.calm();
            } else if self.reshuffle <= 0.0 {
                // Jump to new geometry a few times per burst: the stutter is the effect.
                self.reshuffle = 40.0 + self.rng.unit() * 60.0;
                self.reroll(0.4 + self.intensity * 0.6);
            }
        }
    }

    fn draw(&self, cv: &Canvas) {
        for bar in &self.bars {
            cv.fill_rect(self.x0 + self.tear(bar.y), bar.y, self.side, bar.h, self.inks[bar.color], bar.alpha, Blend::Over);
        }
        // Channels first, core on top: the split shows as coloured fringes.
        self.frame(cv, self.inks[0], -self.split, 0.9);
        self.frame(cv, self.inks[1], self.split, 0.9);
        self.frame(cv, self.inks[2], 0.0, 1.0);
    }

    fn press(&mut self, _at: Pos2) {
        self.start_burst(450.0, 1.0);
    }
}

// --------------------------------------------------------------------------- //
// neon
// --------------------------------------------------------------------------- //

/// A neon-tube frame: a rounded square in a slowly rotating gradient with a
/// glow, and a white-hot highlight running along it.
struct Neon {
    env: Env,
    t: f32,
    hover: f32,
    flash: f32,
}

impl Neon {
    fn new(env: &Env) -> Self {
        Self { env: env.clone(), t: 0.0, hover: 0.0, flash: 0.0 }
    }
}

/// The point a fraction `f` of the way round a rounded square, clockwise from
/// the start of the top edge, by arc length.
fn on_rounded_square(c: f32, half: f32, r: f32, f: f32) -> Pos2 {
    let straight = 2.0 * (half - r);
    let arc = FRAC_PI_2 * r;
    let side = straight + arc;
    let mut d = f.rem_euclid(1.0) * side * 4.0;
    let s = ((d / side).floor() as usize).min(3);
    d -= s as f32 * side;
    let (ax, ay) = if d <= straight {
        (-half + r + d, -half)
    } else {
        let a = -FRAC_PI_2 + (d - straight) / r;
        (half - r + a.cos() * r, -half + r + a.sin() * r)
    };
    let (x, y) = match s {
        0 => (ax, ay),
        1 => (-ay, ax),
        2 => (-ax, -ay),
        _ => (ay, -ax),
    };
    Pos2::new(c + x, c + y)
}

impl Effect for Neon {
    fn update(&mut self, dt: f32, pointer: Option<Pos2>) {
        ease_hover(&mut self.hover, pointer, dt);
        self.flash = (self.flash - dt / 700.0).max(0.0);
        self.t += (dt / 1000.0) * (0.25 + 0.35 * self.env.intensity) * (1.0 + self.hover * 2.0);
    }

    fn draw(&self, cv: &Canvas) {
        let e = &self.env;
        let c = e.extent / 2.0;
        let half = e.size / 2.0 + 2.5 * e.scale;
        let radius = e.size * 0.18 + 2.5 * e.scale;
        let tube = ((2.0 + e.intensity * 1.5) * e.scale).max(1.5);
        let angle = self.t * TAU;
        // Closed gradient: the first colour again at the end, so the loop is seamless.
        let n = e.colors.len();
        let stops: Vec<(f32, Ink)> = (0..=n).map(|i| (i as f32 / n as f32, e.colors[i % n])).collect();
        let glow = (8.0 + 10.0 * e.intensity + 8.0 * self.hover + 16.0 * self.flash) * e.scale;
        let ring: Vec<(Pos2, Ink)> = (0..160)
            .map(|i| {
                let p = on_rounded_square(c, half, radius, i as f32 / 160.0);
                (p, conic(&stops, c, angle, p))
            })
            .collect();
        // Wide soft pass for the bloom, then the tube itself.
        let bloom = 0.45 + 0.25 * self.hover + 0.3 * self.flash;
        cv.stroke_shaded(&ring, true, tube * 2.2, bloom, Blend::Over, Some((e.colors[0], glow)));
        cv.stroke_shaded(&ring, true, tube, 1.0, Blend::Over, Some((e.colors[0], glow * 0.5)));
        // A white-hot core along a stretch of the tube.
        let len = 0.12 + 0.1 * self.hover + 0.5 * self.flash;
        let from = (self.t * 0.9).rem_euclid(1.0);
        let core: Vec<Pos2> =
            (0..=48).map(|i| on_rounded_square(c, half, radius, from + len * i as f32 / 48.0)).collect();
        let alpha = 0.55 + 0.35 * self.hover.max(self.flash);
        cv.stroke(&core, false, (tube * 0.45).max(1.0), Ink::WHITE, alpha, Blend::Over, Some((Ink::WHITE, glow * 0.6)));
    }

    fn press(&mut self, _at: Pos2) {
        self.flash = 1.0;
    }
}

// --------------------------------------------------------------------------- //
// neonsplit
// --------------------------------------------------------------------------- //

/// A broken neon ring inside a chromatically split frame (glitch's).
struct Neonsplit {
    env: Env,
    frame: Glitch,
    ch_a: Ink,
    ch_b: Ink,
    core: Ink,
    t: f32,
    hover: f32,
    flash: f32,
}

impl Neonsplit {
    fn new(env: &Env, rng: Rng) -> Self {
        let ch_a = env.color(0);
        let ch_b = env.color(1);
        // A faint core leaves the two channels as the frame; a third spec colour replaces it.
        let core = if env.custom_colors && env.colors.len() > 2 {
            env.colors[2]
        } else {
            Ink { a: 0.22, ..Ink::WHITE }
        };
        let mut frame_env = env.clone();
        frame_env.colors = vec![ch_a, ch_b, core];
        Self { env: env.clone(), frame: Glitch::new(&frame_env, rng, Some(core)), ch_a, ch_b, core, t: 0.0, hover: 0.0, flash: 0.0 }
    }
}

impl Effect for Neonsplit {
    fn update(&mut self, dt: f32, pointer: Option<Pos2>) {
        self.frame.update(dt, pointer);
        ease_hover(&mut self.hover, pointer, dt);
        self.flash = (self.flash - dt / 700.0).max(0.0);
        self.t += (dt / 1000.0) * (0.12 + 0.2 * self.env.intensity) * (1.0 + self.hover * 2.0);
    }

    fn draw(&self, cv: &Canvas) {
        let e = &self.env;
        let c = e.extent / 2.0;
        let radius = e.size / 2.0 - (2.5 * e.scale).max(2.0);
        let tube = ((2.0 + e.intensity) * e.scale).max(2.0);
        const GAP: f32 = 0.2;
        let arc = FRAC_PI_2 - GAP;
        let angle = self.t * TAU;
        let glow = (8.0 + 10.0 * e.intensity + 8.0 * self.hover + 16.0 * self.flash) * e.scale;
        let stops = [(0.0, self.ch_a), (0.38, self.ch_a), (0.62, self.ch_b), (0.88, self.ch_b), (1.0, self.ch_a)];
        let arcs: Vec<Vec<Pos2>> = (0..4)
            .map(|i| {
                let from = angle + i as f32 * FRAC_PI_2 + GAP / 2.0;
                (0..=24)
                    .map(|k| {
                        let a = from + arc * k as f32 / 24.0;
                        Pos2::new(c + a.cos() * radius, c + a.sin() * radius)
                    })
                    .collect()
            })
            .collect();
        for pts in &arcs {
            let shaded: Vec<(Pos2, Ink)> = pts.iter().map(|p| (*p, conic(&stops, c, angle, *p))).collect();
            cv.stroke_shaded(&shaded, false, tube, 1.0, Blend::Over, Some((self.ch_a, glow)));
        }
        // A white-hot core along the tube.
        let alpha = 0.12 + 0.2 * self.hover + 0.5 * self.flash;
        for pts in &arcs {
            cv.stroke(pts, false, (tube * 0.35).max(0.6), self.core, alpha, Blend::Over, Some((self.core, glow * 0.4)));
        }
        self.frame.draw(cv);
    }

    fn press(&mut self, at: Pos2) {
        self.flash = 1.0;
        self.frame.press(at);
    }
}

// --------------------------------------------------------------------------- //
// signal
// --------------------------------------------------------------------------- //

#[derive(Clone)]
struct SignalBand {
    y: f32,
    h: f32,
    dx: f32,
}

/// A bad signal inside the picture: in bursts the face is redrawn torn into
/// bands, red split from cyan, rolling, with noise and the odd dropout.
struct Signal {
    env: Env,
    rng: Rng,
    hover: bool,
    burst_left: f32,
    next_burst: f32,
    reroll_in: f32,
    strength: f32,
    bands: Vec<SignalBand>,
    split: f32,
    roll: f32,
    dropout: Option<(f32, f32, usize)>,
    noise_seed: f64,
}

impl Signal {
    fn new(env: &Env, mut rng: Rng) -> Self {
        let next_burst = 700.0 + rng.unit() * 1500.0;
        Self {
            env: env.clone(),
            rng,
            hover: false,
            burst_left: 0.0,
            next_burst,
            reroll_in: 0.0,
            strength: 0.0,
            bands: Vec::new(),
            split: 0.0,
            roll: 0.0,
            dropout: None,
            noise_seed: 0.0,
        }
    }

    fn reroll(&mut self, s: f32) {
        let scale = self.env.scale;
        let rng = &mut self.rng;
        self.strength = s;
        self.split = (1.0 + rng.unit() * 4.0 * s) * scale;
        self.roll = if rng.unit() < 0.25 * s { (rng.unit() - 0.5) * 0.3 } else { 0.0 };
        let n = 3 + (rng.unit() * 6.0).floor() as usize;
        let mut cuts: Vec<f32> = (0..n - 1).map(|_| rng.unit()).collect();
        cuts.sort_by(f32::total_cmp);
        let mut edges = vec![0.0];
        edges.extend(cuts);
        edges.push(1.0);
        self.bands = edges
            .windows(2)
            .map(|w| SignalBand {
                y: w[0],
                h: w[1] - w[0],
                dx: if rng.unit() < 0.55 { (rng.unit() - 0.5) * 16.0 * s * scale } else { 0.0 },
            })
            .collect();
        let colors = self.env.colors.len();
        self.dropout = if rng.unit() < 0.2 * s {
            Some((rng.unit() * 0.9, 0.03 + rng.unit() * 0.08, (rng.unit() * colors as f32).floor() as usize % colors))
        } else {
            None
        };
        self.noise_seed = rng.unit() as f64;
    }
}

impl Effect for Signal {
    fn update(&mut self, dt: f32, pointer: Option<Pos2>) {
        self.hover = pointer.is_some();
        self.next_burst -= dt;
        if self.next_burst <= 0.0 {
            self.burst_left = 120.0 + self.rng.unit() * 380.0;
            self.next_burst = (900.0 + self.rng.unit() * 2600.0) * (1.3 - self.env.intensity * 0.7);
        }
        self.burst_left = (self.burst_left - dt).max(0.0);
        if !(self.hover || self.burst_left > 0.0) {
            self.strength = 0.0;
            return;
        }
        self.reroll_in -= dt;
        if self.reroll_in <= 0.0 {
            // The picture jumps between distortions rather than easing: that's the look.
            self.reroll_in = 40.0 + self.rng.unit() * 70.0;
            let s = (if self.hover { 0.5 } else { 0.7 }) * (0.5 + self.env.intensity) * (0.6 + self.rng.unit() * 0.6);
            self.reroll(s);
        }
    }

    fn draw(&self, cv: &Canvas) {
        let e = &self.env;
        let size = e.size;
        let x0 = e.extent / 2.0 - size / 2.0;
        let y0 = x0;
        let cv = cv.clipped_to_face();
        let has_face = !matches!(cv.face, Face::None);
        if self.strength > 0.0 && has_face {
            // Cover the real face, then rebuild it additively from two channels.
            cv.fill_rect(x0, y0, size, size, Ink::BLACK, 1.0, Blend::Over);
            let red = Ink::rgb(255, 0, 0);
            let cyan = Ink::rgb(0, 255, 255);
            for b in &self.bands {
                let sy = (b.y + self.roll).rem_euclid(1.0);
                let sh = b.h.min(1.0 - sy);
                let dy = y0 + b.y * size;
                for (tint, ox) in [(red, -self.split), (cyan, self.split)] {
                    let x = x0 + b.dx + ox;
                    let parts = [(sy, sh, dy), (0.0, b.h - sh, dy + sh * size)];
                    for (uy, uh, top) in parts {
                        if uh <= 0.0 {
                            continue;
                        }
                        let dest = Rect::from_min_size(Pos2::new(x, top), Vec2::new(size, uh * size));
                        match &cv.face {
                            Face::Picture { .. } => cv.face_part(
                                false,
                                Rect::from_min_max(Pos2::new(0.0, uy), Pos2::new(1.0, uy + uh)),
                                dest,
                                tint.lighter(1.0),
                            ),
                            Face::Initials(text) => cv.initials_part(text, uy, dest, tint, Blend::Add),
                            Face::None => {}
                        }
                    }
                }
            }
            // Noise specks and a dropout band.
            let mut n = self.noise_seed;
            let mut next = || {
                n = (n * 9301.0 + 49297.0) % 233280.0;
                (n / 233280.0) as f32
            };
            let specks = (20.0 * self.strength).floor() as usize;
            for _ in 0..specks {
                let alpha = 0.25 + next() * 0.5;
                let ink = if next() < 0.5 { Ink::WHITE } else { e.color((next() * e.colors.len() as f32) as usize) };
                let (x, y, w) = (x0 + next() * size, y0 + next() * size, (1.0 + next() * 6.0) * e.scale);
                cv.fill_rect(x, y, w, (0.8 * e.scale).max(1.0), ink, alpha, Blend::Over);
            }
            if let Some((y, h, color)) = self.dropout {
                cv.fill_rect(x0, y0 + y * size, size, h * size, e.color(color), 0.55, Blend::Over);
            }
        }
        // Scanlines: faint at rest, stronger while distorted.
        let alpha = 0.1 + 0.2 * (self.strength * 1.5).min(1.0);
        let step = (2.0 * e.scale).max(2.0);
        let mut y = y0;
        while y < y0 + size {
            cv.fill_rect(x0, y, size, step / 2.0, Ink::BLACK, alpha, Blend::Over);
            y += step;
        }
    }

    fn press(&mut self, _at: Pos2) {
        self.burst_left = 600.0;
        self.reroll_in = 0.0;
    }
}

// --------------------------------------------------------------------------- //
// old film, brackets (tracking, blobtrack)
// --------------------------------------------------------------------------- //

/// One frame of film, ~24 fps.
const FILM_MS: f32 = 42.0;

/// An old-film layer over the face: a faded print with grain, dust specks and
/// scratches, re-rolled ~24 times a second, and a slight flicker.
struct OldFilm {
    size: f32,
    scale: f32,
    intensity: f32,
    film_in: f32,
    grain: Vec<[f32; 4]>,
    dust: Vec<[f32; 4]>,
    scratches: Vec<[f32; 6]>,
    flicker: f32,
}

impl OldFilm {
    fn new(env: &Env) -> Self {
        Self {
            size: env.size,
            scale: env.scale,
            intensity: env.intensity,
            film_in: 0.0,
            grain: Vec::new(),
            dust: Vec::new(),
            scratches: Vec::new(),
            flicker: 0.0,
        }
    }

    /// `rough` > 1: more grain, dirt and flicker.
    fn update(&mut self, dt: f32, rough: f32, rng: &mut Rng) {
        self.film_in -= dt;
        if self.film_in > 0.0 {
            return;
        }
        self.film_in = FILM_MS;
        let n = ((self.size * self.size / 160.0) * (0.5 + self.intensity) * rough).round() as usize;
        self.grain = (0..n)
            .map(|_| [rng.unit(), rng.unit(), 0.15 + rng.unit() * 0.35, if rng.unit() < 0.6 { 1.0 } else { 0.0 }])
            .collect();
        let specks = (rng.unit() * (3.0 + 4.0 * self.intensity) * rough).floor() as usize;
        self.dust = (0..specks)
            .map(|_| [rng.unit(), rng.unit(), (0.4 + rng.unit() * 1.3) * self.scale, 0.5 + rng.unit() * 0.5])
            .collect();
        // Scratches stay put for a few frames now and then, like a real one.
        if self.scratches.is_empty() || rng.unit() < 0.6 {
            self.scratches.clear();
            let lines = if rng.unit() < 0.45 * rough { 1 + (rng.unit() * 3.0).floor() as usize } else { 0 };
            for _ in 0..lines {
                let kind = rng.unit();
                let angle = if kind < 0.45 {
                    FRAC_PI_2 + (rng.unit() - 0.5) * 0.25
                } else if kind < 0.8 {
                    rng.unit() * PI
                } else {
                    (rng.unit() - 0.5) * 0.35
                };
                let length = if kind < 0.45 {
                    0.4 + rng.unit() * 0.6
                } else if kind < 0.8 {
                    0.2 + rng.unit() * 0.5
                } else {
                    0.08 + rng.unit() * 0.25
                };
                let bend = if rng.unit() < 0.4 { (rng.unit() - 0.5) * 0.25 } else { 0.0 };
                self.scratches.push([rng.unit(), rng.unit(), angle, length, bend, 0.35 + rng.unit() * 0.45]);
            }
        } else {
            // A held scratch drifts a little, like the film moving.
            for s in &mut self.scratches {
                s[0] += (rng.unit() - 0.5) * 0.02;
                s[2] += (rng.unit() - 0.5) * 0.03;
            }
        }
        self.flicker = (rng.unit() - 0.5) * 0.12 * (0.5 + self.intensity) * rough;
    }

    /// Over the face at (fx, fy); the caller clips to it.
    fn draw(&self, cv: &Canvas, fx: f32, fy: f32) {
        let size = self.size;
        cv.fill_rect(fx, fy, size, size, Ink::BLACK, (0.25 - self.flicker).clamp(0.0, 0.6), Blend::Over);
        let g = (self.scale * 0.9).max(1.0);
        for [x, y, a, white] in &self.grain {
            let ink = if *white > 0.0 { Ink::WHITE } else { Ink::BLACK };
            cv.fill_rect(fx + x * size, fy + y * size, g, g, ink, *a, Blend::Over);
        }
        for [x, y, r, a] in &self.dust {
            cv.fill_circle(fx + x * size, fy + y * size, *r, Ink::WHITE, *a);
        }
        let width = (0.55 * self.scale).max(0.6);
        for [x, y, a, len, bend, alpha] in &self.scratches {
            let (cx, cy) = (fx + x * size, fy + y * size);
            let half = len * size / 2.0;
            let (dx, dy) = (a.cos() * half, a.sin() * half);
            let b = bend * size;
            // A quadratic curve whose bend pushes the middle sideways.
            let (p0, p1, p2) = (Pos2::new(cx - dx, cy - dy), Pos2::new(cx - a.sin() * b, cy + a.cos() * b), Pos2::new(cx + dx, cy + dy));
            let pts: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    let u = 1.0 - t;
                    Pos2::new(u * u * p0.x + 2.0 * u * t * p1.x + t * t * p2.x, u * u * p0.y + 2.0 * u * t * p1.y + t * t * p2.y)
                })
                .collect();
            cv.stroke(&pts, false, width, Ink::WHITE, *alpha, Blend::Over, None);
        }
    }
}

/// The [ ] camera brackets either side of the face.
#[derive(Clone, Copy)]
struct Brackets {
    half: f32,
    arm: f32,
    thick: f32,
}

impl Brackets {
    fn new(size: f32, scale: f32) -> Self {
        let half = size / 2.0 + 6.0 * scale;
        Self { half, arm: half * 2.0 * 0.22, thick: (1.4 * scale).max(1.0) }
    }

    /// [ ] around c with half-size h, shifted by (sx, sy).
    #[allow(clippy::too_many_arguments)]
    fn draw(&self, cv: &Canvas, c: f32, h: f32, sx: f32, sy: f32, ink: Ink, alpha: f32) {
        let (top, bottom) = (c - h + sy, c + h + sy);
        let (left, right) = (c - h + sx, c + h - self.thick + sx);
        let t = self.thick;
        let fill = |x, y, w, h| cv.fill_rect(x, y, w, h, ink, alpha, Blend::Over);
        fill(left, top, t, bottom - top);
        fill(left, top, self.arm, t);
        fill(left, bottom - t, self.arm, t);
        fill(right, top, t, bottom - top);
        fill(right - self.arm + t, top, self.arm, t);
        fill(right - self.arm + t, bottom - t, self.arm, t);
    }
}

// --------------------------------------------------------------------------- //
// tracking
// --------------------------------------------------------------------------- //

const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// A camera-tracking HUD: brackets, a figure label and X/Y/Z read-outs down
/// the right edge, over an old-film face.
struct Tracking {
    env: Env,
    rng: Rng,
    frame: Brackets,
    film: OldFilm,
    font_px: f32,
    label_text: String,
    hunt_left: f32,
    next_hunt: f32,
    jitter_in: f32,
    shake: (f32, f32),
    breath_target: f32,
    breath: f32,
    zoom: f32,
    scramble_in: f32,
    coords: String,
    label_in: f32,
    reveal: f32,
    label: String,
}

impl Tracking {
    fn new(env: &Env, mut rng: Rng) -> Self {
        let frame = Brackets::new(env.size, env.scale);
        // Label (6) + a gap (3) + read-out (17) ≈ 26 monospace cells fit the bracket height.
        let font_px = (frame.half * 2.0 / (26.0 * 0.62)).max(4.0);
        let label_text = format!("FIG {:02}", 1 + (rng.unit() * 99.0).floor() as u32);
        let next_hunt = 800.0 + rng.unit() * 1500.0;
        let coords = readout(&mut rng);
        let label_in = 1500.0 + rng.unit() * 2000.0;
        Self {
            env: env.clone(),
            frame,
            film: OldFilm::new(env),
            font_px,
            label: label_text.clone(),
            label_text,
            hunt_left: 0.0,
            next_hunt,
            jitter_in: 0.0,
            shake: (0.0, 0.0),
            breath_target: 0.0,
            breath: 0.0,
            zoom: 0.0,
            scramble_in: 0.0,
            coords,
            label_in,
            reveal: 1.0,
            rng,
        }
    }
}

fn readout(rng: &mut Rng) -> String {
    let mut axis = |name: char| {
        let sign = if rng.unit() < 0.5 { '-' } else { '+' };
        let whole = (rng.unit() * 100.0).floor() as u32;
        let tenth = (rng.unit() * 10.0).floor() as u32;
        format!("{name}{sign}{whole:02}.{tenth}")
    };
    let (x, y, z) = (axis('X'), axis('Y'), axis('Z'));
    format!("{x} {y} {z}")
}

fn scrambled(text: &str, shown: usize, rng: &mut Rng) -> String {
    text.chars()
        .enumerate()
        .map(|(i, ch)| {
            if i < shown || ch == ' ' {
                ch
            } else {
                LETTERS[((rng.unit() * LETTERS.len() as f32) as usize).min(LETTERS.len() - 1)] as char
            }
        })
        .collect()
}

impl Effect for Tracking {
    fn update(&mut self, dt: f32, pointer: Option<Pos2>) {
        let hover = pointer.is_some();
        let (intensity, scale) = (self.env.intensity, self.env.scale);
        // Focus hunts: every 1.5–4s at rest (sooner with intensity), always while hovered.
        self.next_hunt -= dt;
        if self.next_hunt <= 0.0 {
            self.hunt_left = 300.0 + self.rng.unit() * 400.0;
            self.next_hunt = (1500.0 + self.rng.unit() * 2500.0) * (1.3 - intensity * 0.6);
        }
        self.hunt_left = (self.hunt_left - dt).max(0.0);
        let hunting = hover || self.hunt_left > 0.0;
        self.jitter_in -= dt;
        if self.jitter_in <= 0.0 {
            self.jitter_in = if hunting { 45.0 } else { 140.0 };
            let amp = (if hunting { 1.0 + 2.0 * intensity } else { 0.3 }) * scale;
            self.shake = ((self.rng.unit() - 0.5) * 2.0 * amp, (self.rng.unit() - 0.5) * 2.0 * amp);
            self.breath_target = if hunting { (self.rng.unit() - 0.5) * 0.07 * (0.5 + intensity) } else { 0.0 };
        }
        self.breath += (self.breath_target - self.breath) * (dt / 70.0).min(1.0);
        self.zoom = (self.zoom - dt / 450.0).max(0.0);
        self.film.update(dt, if hunting { 1.6 } else { 1.0 }, &mut self.rng);
        // Read-outs never settle: search mode.
        self.scramble_in -= dt;
        if self.scramble_in <= 0.0 {
            self.scramble_in = if hunting { 45.0 } else { 90.0 };
            self.coords = readout(&mut self.rng);
        }
        // The label re-decodes itself every few seconds.
        self.label_in -= dt;
        if self.label_in <= 0.0 {
            self.label_in = 2500.0 + self.rng.unit() * 2500.0;
            self.reveal = 0.0;
        }
        if self.reveal < 1.0 {
            self.reveal = (self.reveal + dt / 700.0).min(1.0);
            let shown = (self.reveal * self.label_text.len() as f32).floor() as usize;
            self.label = scrambled(&self.label_text, shown, &mut self.rng);
        } else {
            self.label = self.label_text.clone();
        }
    }

    fn draw(&self, cv: &Canvas) {
        let e = &self.env;
        let c = e.extent / 2.0;
        let (sx, sy) = self.shake;
        // Click: out by 14%, then an overshoot back in as it settles.
        let k = 1.0 + self.breath + 0.14 * self.zoom * ((1.0 - self.zoom) * PI * 1.5).cos();
        let h = self.frame.half * k;
        let (top, bottom) = (c - h + sy, c + h + sy);
        let right = c + h - self.frame.thick + sx;
        let ink = e.colors[0];
        self.frame.draw(cv, c, h, sx, sy, ink, 0.92);
        // Old film over the face (clipped to it).
        let (fx, fy) = (c - e.size / 2.0, c - e.size / 2.0);
        self.film.draw(&cv.clipped_to_face(), fx, fy);
        // Vertical text down the right edge, glyph tops facing outwards.
        let x = right + self.frame.thick + 3.0 * e.scale;
        cv.text_down(x, top + self.frame.thick, &self.label, self.font_px, ink, 0.92);
        let width = cv.text_width(&self.coords, self.font_px);
        cv.text_down(x, bottom - width, &self.coords, self.font_px, ink, 0.92);
    }

    fn press(&mut self, _at: Pos2) {
        self.zoom = 1.0;
        self.hunt_left = 600.0;
        self.reveal = 0.0;
    }
}

// --------------------------------------------------------------------------- //
// blobtrack, blobinvert
// --------------------------------------------------------------------------- //

#[derive(Clone)]
struct Blob {
    uid: u64,
    id: u32,
    q: usize,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    age: f32,
    life: f32,
    value: f32,
}

#[derive(Clone)]
struct Link {
    a: u64,
    b: u64,
    age: f32,
    life: f32,
}

/// Blob tracking: in each quadrant of the face a box snaps onto a random spot
/// for a split second, with an id and a value; lines flicker between centres.
/// Marks are drawn in the inverse of the face beneath them.
struct Blobtrack {
    env: Env,
    rng: Rng,
    invert_inside: bool,
    film: OldFilm,
    frame: Brackets,
    jitter_in: f32,
    shake: (f32, f32),
    zoom: f32,
    wait: [f32; 4],
    link_in: f32,
    blobs: Vec<Blob>,
    links: Vec<Link>,
    next_uid: u64,
    next_id: u32,
}

impl Blobtrack {
    fn new(env: &Env, mut rng: Rng, invert_inside: bool) -> Self {
        let wait = [rng.unit() * 30.0, rng.unit() * 30.0, rng.unit() * 30.0, rng.unit() * 30.0];
        Self {
            env: env.clone(),
            invert_inside,
            film: OldFilm::new(env),
            frame: Brackets::new(env.size, env.scale),
            jitter_in: 0.0,
            shake: (0.0, 0.0),
            zoom: 0.0,
            wait,
            link_in: 0.0,
            blobs: Vec::new(),
            links: Vec::new(),
            next_uid: 1,
            next_id: 1,
            rng,
        }
    }

    /// A box at a random spot, wholly inside quadrant q.
    fn spawn(&mut self, q: usize, life: f32) -> Blob {
        let rng = &mut self.rng;
        let w = 0.12 + rng.unit() * 0.22;
        let h = (w * (0.7 + rng.unit() * 0.6)).min(0.46);
        let (qx, qy) = ((q % 2) as f32 * 0.5, (q / 2) as f32 * 0.5);
        let blob = Blob {
            uid: self.next_uid,
            id: self.next_id % 100,
            q,
            x: qx + w / 2.0 + rng.unit() * (0.5 - w),
            y: qy + h / 2.0 + rng.unit() * (0.5 - h),
            w,
            h,
            age: 0.0,
            life,
            value: rng.unit(),
        };
        self.next_uid += 1;
        self.next_id += 1;
        blob
    }

    fn link(&mut self, a: u64, b: u64, life: f32) {
        if a == b || self.links.iter().any(|l| (l.a == a && l.b == b) || (l.a == b && l.b == a)) {
            return;
        }
        self.links.push(Link { a, b, age: 0.0, life });
    }

    fn blob(&self, uid: u64) -> Option<&Blob> {
        self.blobs.iter().find(|b| b.uid == uid)
    }

    /// A box in canvas px.
    fn box_of(&self, b: &Blob, x0: f32, y0: f32) -> Rect {
        let (w, h) = (b.w * self.env.size, b.h * self.env.size);
        Rect::from_min_size(Pos2::new(x0 + b.x * self.env.size - w / 2.0, y0 + b.y * self.env.size - h / 2.0), Vec2::new(w, h))
    }
}

impl Effect for Blobtrack {
    fn update(&mut self, dt: f32, pointer: Option<Pos2>) {
        let hover = pointer.is_some();
        let (intensity, scale) = (self.env.intensity, self.env.scale);
        let step = dt * if hover { 1.8 } else { 1.0 };
        self.film.update(dt, if hover { 1.6 } else { 1.0 }, &mut self.rng);
        // Bracket micro-jitter, as in tracking; shakier while hovered.
        self.jitter_in -= dt;
        if self.jitter_in <= 0.0 {
            self.jitter_in = if hover { 45.0 } else { 140.0 };
            let amp = (if hover { 1.0 + 2.0 * intensity } else { 0.3 }) * scale;
            self.shake = ((self.rng.unit() - 0.5) * 2.0 * amp, (self.rng.unit() - 0.5) * 2.0 * amp);
        }
        self.zoom = (self.zoom - dt / 450.0).max(0.0);
        // Everything ages and simply vanishes at the end of its life.
        let rng = &mut self.rng;
        self.blobs.retain_mut(|b| {
            b.age += step;
            if b.age > b.life {
                return false;
            }
            if rng.unit() < 0.3 {
                b.value = rng.unit();
            }
            true
        });
        let alive: Vec<u64> = self.blobs.iter().map(|b| b.uid).collect();
        self.links.retain_mut(|l| {
            l.age += step;
            l.age <= l.life && alive.contains(&l.a) && alive.contains(&l.b)
        });
        // Each empty quadrant gets its next box after a short gap.
        for q in 0..4 {
            if self.blobs.iter().any(|b| b.q == q) {
                continue;
            }
            self.wait[q] -= step;
            if self.wait[q] <= 0.0 {
                let life = 25.0 + self.rng.unit() * 65.0;
                let blob = self.spawn(q, life);
                self.blobs.push(blob);
                self.wait[q] = (5.0 + self.rng.unit() * 30.0) * (1.4 - intensity);
            }
        }
        // Lines flicker between whichever boxes are up.
        self.link_in -= step;
        if self.link_in <= 0.0 && self.blobs.len() > 1 {
            self.link_in = 4.0 + self.rng.unit() * 14.0;
            let n = self.blobs.len() as f32;
            let a = self.blobs[((self.rng.unit() * n) as usize).min(self.blobs.len() - 1)].uid;
            let b = self.blobs[((self.rng.unit() * n) as usize).min(self.blobs.len() - 1)].uid;
            let life = 15.0 + self.rng.unit() * 35.0;
            self.link(a, b, life);
        }
    }

    fn draw(&self, cv: &Canvas) {
        let e = &self.env;
        let c = e.extent / 2.0;
        let (x0, y0) = (c - e.size / 2.0, c - e.size / 2.0);
        let line = (0.5 * e.scale).max(0.5);
        let font_px = (e.size * 0.065).max(5.0);
        let face = cv.clipped_to_face();
        let picture = matches!(face.face, Face::Picture { .. });
        // blobinvert: the face inverted inside each box, under the film.
        if self.invert_inside {
            for b in &self.blobs {
                let r = self.box_of(b, x0, y0);
                if picture {
                    face.face_fill(r.min.x, r.min.y, r.width(), r.height(), true);
                } else {
                    face.fill_rect(r.min.x, r.min.y, r.width(), r.height(), Ink::rgb(0xe0, 0xe9, 0xf2), 1.0, Blend::Over);
                }
            }
        }
        self.film.draw(&face, x0, y0);
        // Marks in the inverse of the face beneath them — unless the spec gave
        // a colour, or there is no picture to take the inverse of.
        let inverse = picture && !e.custom_colors;
        let ink = e.colors[0];
        // Inside a box blobinvert shows the face inverted, so a mark there takes
        // the original to stand out.
        let inside_inverted = !self.invert_inside;
        for l in &self.links {
            let (Some(p), Some(q)) = (self.blob(l.a), self.blob(l.b)) else { continue };
            let (a, b) = (Pos2::new(x0 + p.x * e.size, y0 + p.y * e.size), Pos2::new(x0 + q.x * e.size, y0 + q.y * e.size));
            if inverse {
                face.face_line(a, b, line, true, 0.85);
            } else {
                face.stroke(&[a, b], false, line, ink, 0.85, Blend::Over, None);
            }
        }
        for b in &self.blobs {
            let r = self.box_of(b, x0, y0);
            let (cx, cy) = (r.center().x, r.center().y);
            let t = 2.0 * e.scale;
            if inverse {
                let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()];
                for i in 0..4 {
                    face.face_line(corners[i], corners[(i + 1) % 4], line, inside_inverted, 1.0);
                }
                face.face_fill(cx - t, cy - line / 2.0, t * 2.0, line, inside_inverted);
                face.face_fill(cx - line / 2.0, cy - t, line, t * 2.0, inside_inverted);
            } else {
                face.stroke_rect(r.min.x, r.min.y, r.width(), r.height(), line, ink, 1.0);
                face.fill_rect(cx - t, cy - line / 2.0, t * 2.0, line, ink, 1.0, Blend::Over);
                face.fill_rect(cx - line / 2.0, cy - t, line, t * 2.0, ink, 1.0, Blend::Over);
            }
            let label = format!("{:02} {:.2}", b.id, b.value);
            let ly = if r.min.y - line > y0 + font_px { r.min.y - line } else { r.max.y + font_px + line };
            // Text cannot be cut out of a picture here: on a picture it is
            // white over a dark edge, readable on light and dark parts alike.
            if inverse {
                face.text_above(r.min.x + 0.6, ly + 0.6, &label, font_px, Ink::BLACK.over(0.8));
                face.text_above(r.min.x, ly, &label, font_px, Ink::WHITE.over(1.0));
            } else {
                face.text_above(r.min.x, ly, &label, font_px, ink.over(1.0));
            }
        }
        // [ ] outside the face. Click: out by 14%, then an overshoot back in.
        let k = 1.0 + 0.14 * self.zoom * ((1.0 - self.zoom) * PI * 1.5).cos();
        self.frame.draw(cv, c, self.frame.half * k, self.shake.0, self.shake.1, ink, 0.92);
    }

    fn press(&mut self, _at: Pos2) {
        self.zoom = 1.0;
        self.blobs.clear();
        self.links.clear();
        let burst: Vec<Blob> = (0..4)
            .map(|q| {
                let life = 200.0 + self.rng.unit() * 200.0;
                self.spawn(q, life)
            })
            .collect();
        let uids: Vec<u64> = burst.iter().map(|b| b.uid).collect();
        self.blobs.extend(burst);
        for i in 0..uids.len() {
            for j in i + 1..uids.len() {
                let life = 150.0 + self.rng.unit() * 150.0;
                self.link(uids[i], uids[j], life);
            }
        }
    }
}
