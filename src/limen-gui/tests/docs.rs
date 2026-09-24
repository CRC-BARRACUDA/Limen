//! The handbook: what it carries, and what the search does with it.

use eframe::egui;
use limen_gui::app::*;
use limen_gui::i18n::{self, Lang};

/// The language is one global, so no two tests here may hold an opinion about
/// it at once.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Every article exists in both languages, and opens with the heading its entry
/// in the contents is taken from.
///
/// A missing translation here is not a missing string — it is a blank page where
/// the help should be, which is the one moment a user has nothing else to fall
/// back on.
#[test]
fn every_article_is_written_in_both_languages() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    assert!(!TOPICS.is_empty(), "the handbook is not empty");
    for lang in [Lang::En, Lang::Uk] {
        i18n::set_locale(lang);
        for topic in TOPICS {
            let body = topic.body();
            assert!(
                body.starts_with("# "),
                "{:?}/{} must open with its heading",
                lang,
                topic.id
            );
            assert!(
                body.len() > 400,
                "{:?}/{} is {} bytes — a stub, not an article",
                lang,
                topic.id,
                body.len()
            );
            let title = topic.title();
            assert!(!title.is_empty() && !title.contains('#'), "{title:?}");
        }
    }
    i18n::set_locale(Lang::En);
}

/// The Ukrainian handbook is Ukrainian, not the English one copied across.
#[test]
fn the_translation_is_a_translation() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for topic in TOPICS {
        i18n::set_locale(Lang::En);
        let en = topic.body();
        i18n::set_locale(Lang::Uk);
        let uk = topic.body();
        assert_ne!(en, uk, "{} is the same text twice", topic.id);
        assert!(
            uk.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)),
            "{} has no Cyrillic in it",
            topic.id
        );
    }
    i18n::set_locale(Lang::En);
}

/// Every article's id is distinct, and so is every title — two identical entries
/// in the contents would be two rows nobody can tell apart.
#[test]
fn nothing_is_listed_twice() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for lang in [Lang::En, Lang::Uk] {
        i18n::set_locale(lang);
        let mut titles: Vec<&str> = TOPICS.iter().map(|t| t.title()).collect();
        titles.sort_unstable();
        let before = titles.len();
        titles.dedup();
        assert_eq!(before, titles.len(), "{lang:?}: two articles share a title");
    }
    let mut ids: Vec<&str> = TOPICS.iter().map(|t| t.id).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "two articles share an id");
    i18n::set_locale(Lang::En);
}

/// The search reads the whole article, not its title.
///
/// Somebody looking for help types the word that is bothering them — "digest",
/// "offline", "portable" — and that word is in the body of the page that
/// answers it, usually under a heading that says something else entirely.
#[test]
fn the_search_looks_inside_the_articles() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    i18n::set_locale(Lang::En);

    // Empty query: everything, in order.
    assert_eq!(matching(""), (0..TOPICS.len()).collect::<Vec<_>>());
    assert_eq!(matching("   "), (0..TOPICS.len()).collect::<Vec<_>>());

    let found = |q: &str| -> Vec<&str> { matching(q).iter().map(|&i| TOPICS[i].id).collect() };
    // A word that appears in one article only, under a heading that does not
    // contain it.
    assert_eq!(found("digest"), vec!["trust"]);
    assert!(found("python").contains(&"modules"), "{:?}", found("python"));
    // Case does not matter, to the query or to the text.
    assert_eq!(found("DIGEST"), found("digest"));
    // And a word that is nowhere is nowhere, rather than everywhere.
    assert!(found("kubernetes").is_empty(), "{:?}", found("kubernetes"));
}

/// The search works in Ukrainian too — the articles it reads are the translated
/// ones, so a Ukrainian word must find a page and an English one need not.
#[test]
fn the_search_reads_the_language_on_screen() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    i18n::set_locale(Lang::Uk);
    let found = |q: &str| -> Vec<&str> { matching(q).iter().map(|&i| TOPICS[i].id).collect() };
    assert!(
        found("відбиток").contains(&"trust"),
        "the Ukrainian word for digest finds the trust page: {:?}",
        found("відбиток")
    );
    assert!(!found("").is_empty());
    i18n::set_locale(Lang::En);
}

/// The nav chip and the tab title are translated, and the tab knows its own
/// name — a tab labelled `tab.docs` is what a missing key looks like on screen.
#[test]
fn the_tab_is_named_in_both_languages() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for lang in [Lang::En, Lang::Uk] {
        i18n::set_locale(lang);
        for key in ["nav.docs", "tab.docs", "docs.title", "docs.subtitle",
                    "docs.search_hint", "docs.no_match",
                    "docs.hide_contents", "docs.show_contents"] {
            assert_ne!(i18n::t(key), key, "{lang:?}: {key} is not translated");
        }
        assert_eq!(Tab::Docs.title(), i18n::t("tab.docs"));
    }
    i18n::set_locale(Lang::En);
}


/// Every piece of text the page drew, with where it landed.
fn drawn_shapes(lang: Lang, selected: usize, contents_open: bool) -> Vec<(egui::Rect, String)> {
    let ctx = egui::Context::default();
    limen_gui::ui::apply_theme(&ctx);
    i18n::set_locale(lang);
    let mut out = Vec::new();
    let (mut sel, mut query) = (selected, String::new());
    for _ in 0..2 {
        out.clear();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let frame = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                docs_page(
                    ui,
                    &mut DocsPage {
                        selected: &mut sel,
                        search: &mut query,
                        contents_open: &mut { contents_open },
                        reveal_at: 0.0,
                        now: 100.0,
                        animate: false,
                    },
                );
            });
        });
        for shape in frame.shapes {
            if let egui::epaint::Shape::Text(t) = shape.shape {
                out.push((t.galley.rect.translate(t.pos.to_vec2()), t.galley.text().to_owned()));
            }
        }
    }
    out
}

/// The article reads as paragraphs down the page, not as strips across it.
///
/// A `ScrollArea` inherits its parent's layout, and this one's parent is a
/// `horizontal_top` holding the contents beside it. Without a top-down `Ui` of
/// its own, every label the Markdown renderer emits is laid out as the *next
/// column*: the page came out as columns of one-character-wide vertical text,
/// which is how it first shipped.
#[test]
fn the_article_is_laid_out_in_rows() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let shapes = drawn_shapes(Lang::En, 0, true);
    // Right of the contents column and its divider: the article pane.
    let article: Vec<&(egui::Rect, String)> =
        shapes.iter().filter(|(r, _)| r.min.x > 300.0).collect();
    assert!(article.len() > 5, "the article drew almost nothing: {article:?}");

    // Wrapped prose starts at the same left edge every line — a handful of
    // values at most, allowing for the bullet indent. Laid out as columns it is
    // one distinct x per paragraph, and there are dozens.
    let mut lefts: Vec<i32> = article.iter().map(|(r, _)| r.min.x.round() as i32).collect();
    lefts.sort_unstable();
    lefts.dedup();
    assert!(
        lefts.len() <= 4,
        "the article starts at {} different x positions — it is in columns: {lefts:?}",
        lefts.len()
    );

    // And a line of prose is a line, not a three-character strip.
    let widest = article.iter().map(|(r, _)| r.width()).fold(0.0_f32, f32::max);
    assert!(widest > 300.0, "the widest line drawn is {widest}px");
}

/// The page draws, in both languages, and what it draws is the article.
///
/// Headless: egui lays the whole page out with no window and no GPU, and the
/// shapes it produced are the text that would have been on screen.
fn drawn(lang: Lang, selected: usize, search: &str) -> String {
    drawn_open(lang, selected, search, true)
}

fn drawn_open(lang: Lang, selected: usize, search: &str, contents_open: bool) -> String {
    let ctx = egui::Context::default();
    limen_gui::ui::apply_theme(&ctx);
    i18n::set_locale(lang);
    let mut text = String::new();
    let (mut sel, mut query) = (selected, search.to_string());
    // Two frames: egui settles sizes it had to measure on the first.
    for _ in 0..2 {
        text.clear();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                docs_page(
                    ui,
                    &mut DocsPage {
                        selected: &mut sel,
                        search: &mut query,
                        contents_open: &mut { contents_open },
                        reveal_at: 0.0,
                        now: 100.0,
                        animate: false,
                    },
                );
            });
        });
        for shape in out.shapes {
            if let egui::epaint::Shape::Text(t) = shape.shape {
                text.push_str(t.galley.text());
                text.push('\n');
            }
        }
    }
    text
}

#[test]
fn the_page_draws_the_article_it_is_told_to() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let text = drawn(Lang::En, 0, "");
    // The contents lists every article…
    i18n::set_locale(Lang::En);
    for topic in TOPICS {
        assert!(
            text.contains(topic.title()),
            "{} is missing from the contents",
            topic.id
        );
    }
    // …and the body of the selected one is on the page, in prose rather than as
    // Markdown: the heading marker is consumed by the renderer.
    assert!(text.contains("Limen is a console for tools"), "{text}");
    assert!(!text.contains("# Getting started"), "raw Markdown leaked through");

    // A different selection shows a different article. Asserted on its opening
    // line, not on a heading further down: the pane is a scroll area, and only
    // what fits above the fold is drawn at all.
    let trust = drawn(Lang::En, 2, "");
    assert!(trust.contains("A module is code somebody else wrote"), "{trust}");
    assert!(!trust.contains("Limen is a console for tools"), "two articles at once");
    i18n::set_locale(Lang::En);
}

/// Two scroll areas on one page: the contents and the article. Given the same
/// id they collide, and egui paints a "🔥 First use of ScrollArea ID…" warning
/// over the page — which is what shipped the last time two tables shared one id.
#[test]
fn the_two_panes_do_not_collide() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let text = drawn(Lang::En, 0, "");
    assert!(!text.contains('🔥'), "an egui id clash was painted: {text}");
    i18n::set_locale(Lang::En);
}

/// Searching down to nothing says so, rather than showing an empty page beside
/// an empty list.
#[test]
fn a_search_that_finds_nothing_says_so() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    i18n::set_locale(Lang::En);
    let text = drawn(Lang::En, 0, "kubernetes");
    assert!(text.contains(&i18n::t("docs.no_match")), "{text}");
    i18n::set_locale(Lang::En);
}

/// A search that filters out what you were reading moves you to the first thing
/// it did find — rather than leaving the pane showing a page the list no longer
/// offers.
#[test]
fn a_search_moves_you_to_what_it_found() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    i18n::set_locale(Lang::En);
    // "digest" is in the trust article only; start on another one.
    let text = drawn(Lang::En, 0, "digest");
    assert!(text.contains("A module is code somebody else wrote"), "{text}");
    i18n::set_locale(Lang::En);
}

/// Ukrainian draws Ukrainian, chrome and article alike.
#[test]
fn the_page_follows_the_language() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let text = drawn(Lang::Uk, 0, "");
    assert!(text.contains("Початок роботи"), "{text}");
    assert!(text.contains(&{
        i18n::set_locale(Lang::Uk);
        i18n::t("docs.search_hint")
    }));
    assert!(!text.contains("Getting started"));
    i18n::set_locale(Lang::En);
}


/// The contents column can be put away, and the article takes the width it
/// leaves.
///
/// Both halves matter. A column that hides but leaves its gap behind is a
/// blank stripe down the page; one that hides without releasing its clicks is
/// a row of invisible buttons.
#[test]
fn the_contents_can_be_put_away() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    i18n::set_locale(Lang::En);

    let open = drawn_shapes(Lang::En, 0, true);
    let closed = drawn_shapes(Lang::En, 0, false);
    let title_of = |i: usize| TOPICS[i].title();

    // Out: every entry is on the page. Away: none of them are.
    assert!(open.iter().any(|(_, s)| s == title_of(3)), "the contents are out");
    assert!(
        !closed.iter().any(|(_, s)| s == title_of(3)),
        "an entry is still drawn with the contents away"
    );

    // The article is still there either way…
    let body = |v: &[(egui::Rect, String)]| -> Option<egui::Rect> {
        v.iter()
            .find(|(_, s)| s.contains("Limen is a console for tools"))
            .map(|(r, _)| *r)
    };
    let (a, b) = (body(&open), body(&closed));
    assert!(a.is_some() && b.is_some(), "the article must survive the toggle");
    // …and with the column away it starts where the column used to be, rather
    // than leaving a blank stripe behind.
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(
        b.min.x < a.min.x - 100.0,
        "the article did not move left: {} → {}",
        a.min.x,
        b.min.x
    );
    i18n::set_locale(Lang::En);
}


/// Where the hide button is, found by its 26×26 outlined box.
fn toggle_rect(contents_open: bool) -> egui::Rect {
    let ctx = egui::Context::default();
    limen_gui::ui::apply_theme(&ctx);
    i18n::set_locale(Lang::En);
    let (mut sel, mut query, mut open) = (0usize, String::new(), contents_open);
    let mut found = egui::Rect::NOTHING;
    for _ in 0..2 {
        found = egui::Rect::NOTHING;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let frame = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                docs_page(
                    ui,
                    &mut DocsPage {
                        selected: &mut sel,
                        search: &mut query,
                        contents_open: &mut open,
                        reveal_at: 0.0,
                        now: 100.0,
                        animate: false,
                    },
                );
            });
        });
        for shape in frame.shapes {
            if let egui::epaint::Shape::Rect(r) = shape.shape
                && (r.rect.width() - 26.0).abs() < 0.5
                && (r.rect.height() - 26.0).abs() < 0.5
            {
                found = r.rect;
            }
        }
    }
    found
}

/// The hide button sits at the top of the contents column — at its right edge
/// while the column is out, and where the column was once it is away.
///
/// Both positions are the point. A button that lives beside the page title is
/// not beside the thing it hides; one that disappears with the column leaves no
/// way to bring it back.
#[test]
fn the_hide_button_rides_with_the_column() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());

    let out = toggle_rect(true);
    let away = toggle_rect(false);
    assert!(out.is_positive(), "the button is not drawn with the contents out");
    assert!(away.is_positive(), "the button vanished with the contents");

    // Out: at the column's right edge, which is ~230px in.
    assert!(
        out.max.x > 180.0 && out.max.x < 280.0,
        "the button is at x={} — not at the column's edge",
        out.max.x
    );
    // Away: back at the left, where the column was.
    assert!(
        away.min.x < 60.0,
        "with the contents away the button is at x={}",
        away.min.x
    );
    // And it is above the list either way, not floating in the middle of it.
    let first_entry = drawn_shapes(Lang::En, 0, true)
        .into_iter()
        .find(|(_, s)| s == TOPICS[0].title())
        .expect("the first entry is drawn");
    assert!(
        out.max.y <= first_entry.0.min.y + 1.0,
        "the button ({}) is not above the first entry ({})",
        out.max.y,
        first_entry.0.min.y
    );
    i18n::set_locale(Lang::En);
}
