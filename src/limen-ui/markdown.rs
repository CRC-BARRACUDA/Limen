//! The small Markdown renderer release notes are shown with.

use crate::*;

/// One inline span of a Markdown line.
#[derive(Debug, PartialEq)]
pub enum Md {
    Text(String),
    Bold(String),
    Code(String),
    Link { label: String, url: String },
}

/// Split a line into inline spans, handling `**bold**`, `` `code` `` and
/// `[label](url)`. Unmatched markers stay literal.
pub fn inline_segments(line: &str) -> Vec<Md> {
    let mut out: Vec<Md> = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut Vec<Md>| {
        if !buf.is_empty() {
            out.push(Md::Text(std::mem::take(buf)));
        }
    };
    let mut rest = line;
    while !rest.is_empty() {
        // Index of the next markup opener, whichever comes first.
        let next = ["**", "`", "["]
            .iter()
            .filter_map(|m| rest.find(m).map(|i| (i, *m)))
            .min_by_key(|(i, _)| *i);
        let Some((i, marker)) = next else {
            buf.push_str(rest);
            break;
        };
        buf.push_str(&rest[..i]);
        let after = &rest[i + marker.len()..];
        match marker {
            "**" => match after.find("**") {
                Some(j) => {
                    flush(&mut buf, &mut out);
                    out.push(Md::Bold(after[..j].to_string()));
                    rest = &after[j + 2..];
                }
                None => {
                    buf.push_str("**");
                    rest = after;
                }
            },
            "`" => match after.find('`') {
                Some(j) => {
                    flush(&mut buf, &mut out);
                    out.push(Md::Code(after[..j].to_string()));
                    rest = &after[j + 1..];
                }
                None => {
                    buf.push('`');
                    rest = after;
                }
            },
            _ => match parse_link(&rest[i..]) {
                Some((label, url, consumed)) => {
                    flush(&mut buf, &mut out);
                    out.push(Md::Link { label, url });
                    rest = &rest[i + consumed..];
                }
                None => {
                    buf.push('[');
                    rest = after;
                }
            },
        }
    }
    flush(&mut buf, &mut out);
    out
}

/// Parse `[label](url)` at the start of `s`; returns (label, url, bytes consumed).
pub fn parse_link(s: &str) -> Option<(String, String, usize)> {
    let close = s.find(']')?;
    let tail = &s[close + 1..];
    if !tail.starts_with('(') {
        return None;
    }
    let end = tail.find(')')?;
    Some((
        s[1..close].to_string(),
        tail[1..end].to_string(),
        close + 1 + end + 1,
    ))
}

/// Render a line's inline spans into the current (wrapped) row.
pub fn render_spans(ui: &mut egui::Ui, spans: Vec<Md>) {
    for span in spans {
        match span {
            Md::Text(t) => {
                ui.label(egui::RichText::new(t).color(color::TEXT_MUTED));
            }
            Md::Bold(t) => {
                ui.label(egui::RichText::new(t).strong().color(color::TEXT));
            }
            Md::Code(t) => {
                ui.label(egui::RichText::new(t).monospace().color(color::ACCENT));
            }
            Md::Link { label, url } => {
                ui.hyperlink_to(label, url);
            }
        }
    }
}

/// Render a small subset of Markdown (headings, bold, inline code, bullets,
/// links) — enough for release notes.
pub fn markdown(ui: &mut egui::Ui, md: &str) {
    for raw in md.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            ui.add_space(6.0);
        } else if let Some(rest) = trimmed.strip_prefix("### ") {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(rest)
                    .strong()
                    .size(15.0)
                    .color(color::TEXT),
            );
        } else if let Some(rest) = trimmed.strip_prefix("## ") {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(rest)
                    .strong()
                    .size(17.0)
                    .color(color::TEXT),
            );
        } else if let Some(rest) = trimmed.strip_prefix("# ") {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(rest).heading().color(color::TEXT));
        } else if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(egui::RichText::new("  •  ").color(color::TEXT_MUTED));
                render_spans(ui, inline_segments(rest));
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                render_spans(ui, inline_segments(line));
            });
        }
    }
}
