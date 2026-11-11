//! Text arriving a letter or a word at a time.

use limen_ui::*;
/// Slicing a string by a count of characters is the same as slicing it by
/// bytes only in ASCII, and this application ships in Ukrainian. Every
/// intermediate step of a Cyrillic reveal has to land on a boundary, or the
/// heading panics part way through typing itself.
#[test]
fn typing_cyrillic_never_splits_a_character() {
let uk = "Ліцензія — вільне програмне забезпечення";
assert!(uk.len() > uk.chars().count(), "the test needs multi-byte text");
for mode in [Reveal::Letters, Reveal::Words] {
    for step in 0..200 {
        let n = revealed_len(uk, mode, step as f64 * 0.01, 10.0);
        assert!(uk.is_char_boundary(n), "{mode:?} cut at byte {n}");
        let _ = &uk[..n]; // would panic on a bad boundary
    }
}
// And it does finish, rather than stopping one character short.
assert_eq!(revealed(uk, Reveal::Letters, 100.0, 10.0), uk);
assert_eq!(revealed(uk, Reveal::Words, 100.0, 10.0), uk);
}
#[test]
fn letters_arrive_one_at_a_time_and_words_whole() {
let t = "one two three";
// Ten a second: at 0.25s two letters, at 0.5s five.
assert_eq!(revealed(t, Reveal::Letters, 0.25, 10.0), "on");
assert_eq!(revealed(t, Reveal::Letters, 0.5, 10.0), "one t");
// Words land whole: never a fragment of one.
for step in 0..60 {
    let shown = revealed(t, Reveal::Words, step as f64 * 0.05, 4.0);
    assert!(
        shown.is_empty() || t[shown.len()..].starts_with(char::is_whitespace) || shown == t,
        "cut mid-word: {shown:?}"
    );
}
assert_eq!(revealed(t, Reveal::Words, 0.3, 4.0), "one");
assert_eq!(revealed(t, Reveal::Words, 0.6, 4.0), "one two");
}
/// Nothing has arrived before it starts, and a speed of zero does not divide
/// the reveal by nothing.
#[test]
fn typing_handles_the_edges() {
assert_eq!(revealed("abc", Reveal::Letters, 0.0, 10.0), "");
assert_eq!(revealed("abc", Reveal::Letters, -1.0, 10.0), "");
assert_eq!(revealed("", Reveal::Words, 5.0, 10.0), "");
assert_eq!(revealed("abc", Reveal::Letters, 99.0, 0.0), "");
// Newlines are whitespace, so a paragraph reveals across its lines.
let para = "first line\nsecond line";
assert_eq!(revealed(para, Reveal::Words, 0.75, 4.0), "first line\nsecond");
assert_eq!(revealed(para, Reveal::Words, 99.0, 4.0), para);
}

/// The same boundary rule as the letter-by-letter reveal, for the fraction of
/// a string a retype shows: a Ukrainian label cut part-way through a character
/// panics, and a language change cuts every label on screen at once.
#[test]
fn a_share_of_cyrillic_never_splits_a_character() {
    let uk = "Налаштування — мова інтерфейсу";
    assert!(
        uk.len() > uk.chars().count(),
        "the test needs multi-byte text"
    );
    for step in 0..=100 {
        let s = step as f32 / 100.0;
        let n = share_len(uk, s);
        assert!(uk.is_char_boundary(n), "cut at byte {n} for share {s}");
        let _ = &uk[..n]; // would panic on a bad boundary
    }
}

/// Both ends are exact: a share of 0 shows nothing and a share of 1 shows
/// everything. An entrance that stopped one character short, or a backspace
/// that left one behind, is what a reader notices.
#[test]
fn a_share_runs_from_nothing_to_the_whole_string() {
    let s = "Modules";
    assert_eq!(share(s, 0.0), "");
    assert_eq!(share(s, 1.0), s);
    assert_eq!(share(s, -1.0), "", "nonsense shares show nothing");
    assert_eq!(share(s, 2.0), s);
    assert_eq!(share(s, f32::NAN), "");
    // Monotonic, and it never overshoots the string.
    let mut last = 0;
    for step in 0..=20 {
        let n = share_len(s, step as f32 / 20.0);
        assert!(n >= last && n <= s.len(), "share {step}/20 gave {n}");
        last = n;
    }
    // Proportional: half of a long string and half of a short one are both
    // half, which is what makes every label finish together.
    assert_eq!(share("12345678", 0.5), "1234");
    assert_eq!(share("1234", 0.5), "12");
}
