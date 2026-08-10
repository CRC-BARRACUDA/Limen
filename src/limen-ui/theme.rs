//! The palette, the fonts, and the egui style everything else is drawn against.

use crate::*;

// --------------------------------------------------------------------------- //
// Barracuda "EVE-Frontier" amber-HUD palette
//
// Mirrors the platform ui-kit design tokens (Libraries/ui-kit `src/tokens.ts` +
// `theme.css`): a warm amber accent over deep warm-black backgrounds, orange CTA,
// and warm brown-grey neutrals (not a cool slate). Keep these in sync with the
// kit so Limen and the web platform read as one product.
// --------------------------------------------------------------------------- //
pub mod color {
    use eframe::egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x08, 0x06, 0x04); // deep warm-black page
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(0x0d, 0x0a, 0x06); // inputs/code, near-black
    pub const BG_WIDGET: Color32 = Color32::from_rgb(0x18, 0x11, 0x09); // buttons/rows
    pub const BG_HOVER: Color32 = Color32::from_rgb(0x2a, 0x1d, 0x0f); // amber-tinted hover
    pub const BORDER: Color32 = Color32::from_rgb(0x3a, 0x2c, 0x1c); // warm amber-dim border
    pub const TEXT: Color32 = Color32::from_rgb(0xc4, 0xb4, 0x98); // warm body text
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x8a, 0x78, 0x68); // warm grey
    pub const ACCENT: Color32 = Color32::from_rgb(0xc8, 0x78, 0x30); // amber — primary accent
    pub const ACCENT_BRIGHT: Color32 = Color32::from_rgb(0xf0, 0xa8, 0x50); // amber, hovered
    pub const ORANGE: Color32 = Color32::from_rgb(0xf9, 0x73, 0x16); // orange — CTA fill
    pub const ORANGE_BRIGHT: Color32 = Color32::from_rgb(0xfb, 0x92, 0x3c); // orange, hovered
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0xff, 0xf5, 0xe8); // text on amber/orange
    pub const SUCCESS: Color32 = Color32::from_rgb(0x4a, 0xde, 0x80); // done / OK green
    pub const WARNING: Color32 = Color32::from_rgb(0xfb, 0xbf, 0x24); // warning amber-yellow
    pub const ERROR: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71); // error red
    // The destructive outline's accent. Same red the error text already uses, so
    // "this destroys something" reads the same wherever it appears.
    pub const DANGER_BRIGHT: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71); // red — destructive
}

// --------------------------------------------------------------------------- //
// Animation toolkit — reusable primitives + widgets built on them.
//
// Every animated widget is assembled from a few pieces: the global on/off switch
// ([`animations_enabled`]), the easing/colour helpers, and [`interact`] — which
// turns a [`Response`](egui::Response) into eased hover/press factors. A new
// animated widget just allocates a rect and paints from those factors; entrance
// animations use [`reveal_t`]. Nothing here is Modules-specific, so it can be
// reused anywhere.
// --------------------------------------------------------------------------- //


/// Channel-wise linear interpolation between two opaque colours.
pub fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let c = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    egui::Color32::from_rgb(c(a.r(), b.r()), c(a.g(), b.g()), c(a.b(), b.b()))
}

/// `c` with its alpha scaled by `t` (0 = transparent, 1 = opaque) — to fade a
/// fill or stroke in.
pub fn with_alpha(c: egui::Color32, t: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (255.0 * t.clamp(0.0, 1.0)) as u8)
}

// ---- interaction ----------------------------------------------------------- //

/// Install **JetBrains Mono** as the app's single face, for both the
/// proportional and the monospace family.
///
/// It carries Latin *and* Cyrillic in one file (1363 codepoints), so English and
/// Ukrainian render in the same typeface. The previous brand pair — Orbitron and
/// Share Tech Mono — had **zero** Cyrillic glyphs between them, so every
/// Ukrainian string silently dropped through to egui's bundled Ubuntu-Light,
/// leaving mixed typefaces on any line that mixed the two alphabets.
///
/// It's inserted at the *front* of each family, with egui's bundled faces kept
/// behind it as fallback — emoji (and the `⚙` it lacks) still resolve there.
pub fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "JetBrainsMono".to_owned(),
        FontData::from_static(include_bytes!(
            "../../resources/fonts/JetBrainsMono-Regular.ttf"
        )),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "JetBrainsMono".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Apply the Barracuda amber-HUD theme (palette + fonts + sharp chrome) to the
/// whole context.
pub fn apply_theme(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, Rounding, Stroke, TextStyle};

    install_fonts(ctx);

    let mut style = (*ctx.style()).clone();

    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(18.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(12.0);

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = Some(color::TEXT);
    v.panel_fill = color::BG;
    v.window_fill = color::BG;
    v.extreme_bg_color = color::BG_ELEVATED;
    v.faint_bg_color = color::BG_ELEVATED;
    v.window_stroke = Stroke::new(1.0_f32, color::BORDER);
    v.window_rounding = Rounding::ZERO;
    v.hyperlink_color = color::ACCENT_BRIGHT;
    v.selection.bg_fill = color::ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0_f32, color::ACCENT);

    // Sharp, zero-radius HUD chrome — the ui-kit uses `border-radius: 0` throughout.
    let rounding = Rounding::ZERO;
    v.widgets.noninteractive.bg_fill = color::BG;
    v.widgets.noninteractive.weak_bg_fill = color::BG;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, color::BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, color::TEXT);
    v.widgets.noninteractive.rounding = rounding;

    v.widgets.inactive.bg_fill = color::BG_WIDGET;
    v.widgets.inactive.weak_bg_fill = color::BG_WIDGET;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, color::BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, color::TEXT);
    v.widgets.inactive.rounding = rounding;

    v.widgets.hovered.bg_fill = color::BG_HOVER;
    v.widgets.hovered.weak_bg_fill = color::BG_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, color::ACCENT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, color::TEXT);
    v.widgets.hovered.rounding = rounding;

    v.widgets.active.bg_fill = color::ACCENT;
    v.widgets.active.weak_bg_fill = color::ACCENT;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, color::ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, color::ON_ACCENT);
    v.widgets.active.rounding = rounding;

    v.widgets.open.bg_fill = color::BG_ELEVATED;
    v.widgets.open.weak_bg_fill = color::BG_ELEVATED;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, color::BORDER);
    v.widgets.open.rounding = rounding;

    ctx.set_style(style);
}
