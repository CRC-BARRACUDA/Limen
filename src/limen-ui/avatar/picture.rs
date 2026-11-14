//! A picture for an avatar — the user's icon or a decoration's frame — decoded
//! off the GUI thread and then uploaded as textures.
//!
//! PNG, APNG, GIF and WebP (animated or not), and JPEG for icons. Frames are
//! scaled down to what an avatar can show, and an animation that would take
//! too much memory keeps its first frame only: nothing from the network is
//! decoded without a limit.

use std::io::Cursor;
use std::sync::Arc;

use eframe::egui;
use image::{AnimationDecoder, ImageFormat};

/// The largest side a decoded frame keeps: the hover card's 72px avatar, at
/// 2x, with a frame's 1.2 — rounded up.
pub const MAX_SIDE: u32 = 192;
/// At most this many frames of an animation.
const MAX_FRAMES: usize = 400;
/// At most this many decoded bytes for all frames together.
const MAX_DECODED: usize = 48 << 20;

/// Decoded frames, each with how long it shows, in ms. One frame: a still.
pub struct Decoded {
    pub frames: Vec<(egui::ColorImage, u32)>,
}

/// Browsers show a GIF frame of 0–10ms for 100ms; so does this.
fn delay_ms(frame: &image::Frame) -> u32 {
    let (n, d) = frame.delay().numer_denom_ms();
    let ms = n.checked_div(d).unwrap_or(0);
    if ms <= 10 { 100 } else { ms }
}

fn to_color_image(img: image::RgbaImage, max_side: u32) -> egui::ColorImage {
    let (w, h) = img.dimensions();
    let img = if w.max(h) > max_side {
        let k = max_side as f32 / w.max(h) as f32;
        let (nw, nh) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
        image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let size = [img.width() as usize, img.height() as usize];
    egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw())
}

/// Decode a picture's bytes into its frames.
pub fn decode(bytes: &[u8], max_side: u32) -> Result<Decoded, String> {
    let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
    let frames: Option<Vec<image::Frame>> = match format {
        ImageFormat::Gif => image::codecs::gif::GifDecoder::new(Cursor::new(bytes))
            .and_then(|d| d.into_frames().take(MAX_FRAMES).collect::<Result<Vec<_>, _>>())
            .ok(),
        ImageFormat::WebP => match image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)) {
            Ok(d) if d.has_animation() => {
                d.into_frames().take(MAX_FRAMES).collect::<Result<Vec<_>, _>>().ok()
            }
            _ => None,
        },
        ImageFormat::Png => match image::codecs::png::PngDecoder::new(Cursor::new(bytes)) {
            Ok(d) if d.is_apng().unwrap_or(false) => d
                .apng()
                .and_then(|a| a.into_frames().take(MAX_FRAMES).collect::<Result<Vec<_>, _>>())
                .ok(),
            _ => None,
        },
        _ => None,
    };
    if let Some(frames) = frames.filter(|f| f.len() > 1) {
        let bytes_needed: usize = frames
            .iter()
            .map(|f| {
                let (w, h) = f.buffer().dimensions();
                let k = (max_side as f32 / w.max(h) as f32).min(1.0);
                ((w as f32 * k) * (h as f32 * k) * 4.0) as usize
            })
            .sum();
        if bytes_needed <= MAX_DECODED {
            let out = frames
                .into_iter()
                .map(|f| {
                    let ms = delay_ms(&f);
                    (to_color_image(f.into_buffer(), max_side), ms)
                })
                .collect();
            return Ok(Decoded { frames: out });
        }
    }
    // A still, or an animation kept to its first frame.
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    Ok(Decoded { frames: vec![(to_color_image(img, max_side), 0)] })
}

/// A picture on the GPU: its frames, and for a face, the still and its
/// colour-inverted copy that effects which work on the picture draw from.
pub struct Picture {
    frames: Vec<(egui::TextureHandle, u32)>,
    total_ms: u32,
    /// Width / height of the picture.
    pub aspect: f32,
    /// The first frame, cropped square: what effects distort.
    pub still: egui::TextureHandle,
    /// The same, colour-inverted.
    pub inverted: egui::TextureHandle,
}

impl Picture {
    /// Upload decoded frames. On the GUI thread.
    pub fn upload(ctx: &egui::Context, name: &str, decoded: Decoded) -> Arc<Self> {
        let opts = egui::TextureOptions::LINEAR;
        let first = decoded.frames.first().map(|(img, _)| img.clone()).unwrap_or_else(|| {
            egui::ColorImage::new([1, 1], egui::Color32::TRANSPARENT)
        });
        let [w, h] = first.size;
        let aspect = w as f32 / h.max(1) as f32;
        let square = crop_square(&first);
        let inverted = invert(&square);
        let still = ctx.load_texture(format!("{name}:still"), square, opts);
        let inverted = ctx.load_texture(format!("{name}:inverted"), inverted, opts);
        let total_ms = decoded.frames.iter().map(|(_, ms)| *ms).sum();
        let frames = decoded
            .frames
            .into_iter()
            .enumerate()
            .map(|(i, (img, ms))| (ctx.load_texture(format!("{name}:{i}"), img, opts), ms))
            .collect();
        Arc::new(Self { frames, total_ms, aspect, still, inverted })
    }

    pub fn animated(&self) -> bool {
        self.frames.len() > 1 && self.total_ms > 0
    }

    /// The frame showing `ms` into the animation (looping), and how long until
    /// the next one. The first frame, and no next, for a still — or when
    /// animations are off.
    pub fn frame_at(&self, ms: f64, animate: bool) -> (&egui::TextureHandle, Option<f64>) {
        if !animate || !self.animated() {
            return (&self.frames[0].0, None);
        }
        let mut t = ms.rem_euclid(self.total_ms as f64);
        for (tex, d) in &self.frames {
            if t < *d as f64 {
                return (tex, Some(*d as f64 - t));
            }
            t -= *d as f64;
        }
        (&self.frames[0].0, Some(1.0))
    }
}

/// The face tile's colour, under a picture with transparent parts.
pub const FACE_BG: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x16, 0x0d);

/// The middle square of an image ("object-fit: cover" for a square face), on
/// the opaque face tile — effects composite it per channel, and transparent
/// pixels would otherwise come out as solid colour.
fn crop_square(img: &egui::ColorImage) -> egui::ColorImage {
    let [w, h] = img.size;
    let s = w.min(h);
    let (x0, y0) = ((w - s) / 2, (h - s) / 2);
    let [br, bg, bb, _] = FACE_BG.to_array();
    let mut px = Vec::with_capacity(s * s);
    for y in 0..s {
        for c in &img.pixels[(y0 + y) * w + x0..(y0 + y) * w + x0 + s] {
            // Premultiplied: the picture, plus the tile where it lets it through.
            let [r, g, b, a] = c.to_array();
            let k = 255 - a as u16;
            let over = |src: u8, tile: u8| (src as u16 + tile as u16 * k / 255).min(255) as u8;
            px.push(egui::Color32::from_rgb(over(r, br), over(g, bg), over(b, bb)));
        }
    }
    egui::ColorImage { size: [s, s], pixels: px }
}

/// Each colour turned into its opposite, as `difference` with white does.
fn invert(img: &egui::ColorImage) -> egui::ColorImage {
    let pixels = img
        .pixels
        .iter()
        .map(|c| {
            let [r, g, b, a] = c.to_srgba_unmultiplied();
            egui::Color32::from_rgba_unmultiplied(255 - r, 255 - g, 255 - b, a)
        })
        .collect();
    egui::ColorImage { size: img.size, pixels }
}
