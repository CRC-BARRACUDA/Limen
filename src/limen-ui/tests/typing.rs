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
