//! What a frame of a scale change costs, measured rather than guessed.
//!
//! `cargo test -p limen-ui --test zoom_cost -- --ignored --nocapture`

use eframe::egui;
use limen_ui::*;

/// A screenful of text, laid out the way the host lays its pages out.
fn page(ctx: &egui::Context) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(980.0, 640.0),
        )),
        ..Default::default()
    };
    ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            for i in 0..24 {
                ui.horizontal(|ui| {
                    ui.label(format!("module-{i}"));
                    ui.label(egui::RichText::new("Searches file contents and archives").small());
                    let _ = ui.button("Run");
                });
            }
        });
    })
}

/// How many bytes of texture a frame hands the renderer.
fn uploaded(out: &egui::FullOutput) -> usize {
    out.textures_delta
        .set
        .iter()
        .map(|(_, img)| match &img.image {
            egui::ImageData::Font(f) => f.pixels.len() * 4,
            egui::ImageData::Color(c) => c.pixels.len() * 4,
        })
        .sum()
}

#[test]
#[ignore = "a measurement, not an assertion; run with --ignored --nocapture"]
fn what_a_frame_of_zoom_costs() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    ctx.set_zoom_factor(1.0);
    // Settle: build the atlas for this scale once.
    for _ in 0..4 {
        let _ = page(&ctx);
    }

    let out = page(&ctx);
    let start = std::time::Instant::now();
    let out2 = page(&ctx);
    println!(
        "  settled frame:      {:>7.2}ms   uploads {:>2} textures, {:>8} KiB",
        start.elapsed().as_secs_f64() * 1000.0,
        out2.textures_delta.set.len(),
        uploaded(&out2) / 1024
    );
    drop(out);

    // Now the same frame, but at a scale it has never seen — one step of the
    // glide.
    for (i, pct) in [102.0_f32, 104.0, 106.0, 108.0].iter().enumerate() {
        ctx.set_zoom_factor(pct / 100.0);
        let start = std::time::Instant::now();
        let out = page(&ctx);
        println!(
            "  frame at {pct:>5.1}% (#{i}): {:>7.2}ms   uploads {:>2} textures, {:>8} KiB",
            start.elapsed().as_secs_f64() * 1000.0,
            out.textures_delta.set.len(),
            uploaded(&out) / 1024
        );
    }

    // And going back to a scale it has already built, to separate "new scale"
    // from "new atlas".
    ctx.set_zoom_factor(1.0);
    let start = std::time::Instant::now();
    let out = page(&ctx);
    println!(
        "  back to 100%:       {:>7.2}ms   uploads {:>2} textures, {:>8} KiB",
        start.elapsed().as_secs_f64() * 1000.0,
        out.textures_delta.set.len(),
        uploaded(&out) / 1024
    );
}
