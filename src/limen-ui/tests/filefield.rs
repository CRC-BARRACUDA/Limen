//! The path field and what it will accept.

use limen_ui::*;
/// What a path field takes, and the fallback for modules built before
/// `accepts` existed.
#[test]
fn accepts_reads_the_spec_and_the_legacy_flag() {
// The new spelling.
assert!(Accepts::of("file", false) == Accepts::File);
assert!(Accepts::of("dir", false) == Accepts::Dir);
assert!(Accepts::of("file|dir", false) == Accepts::Either);
assert!(Accepts::of("dir|file", false) == Accepts::Either);
assert!(Accepts::of(" FILE|DIR ", false) == Accepts::Either);

// Absent: fall back to `directory`, so an older module is unchanged.
assert!(Accepts::of("", true) == Accepts::Dir);
assert!(Accepts::of("", false) == Accepts::File);
// `accepts` wins when both are given.
assert!(Accepts::of("file", true) == Accepts::File);

// Nonsense narrows rather than widens — a typo must not make a field
// swallow anything at all.
assert!(Accepts::of("folder", false) == Accepts::File);
assert!(Accepts::of("anything", false) == Accepts::File);
}
#[test]
fn a_dual_field_takes_both_and_the_others_take_one() {
for (a, file_ok, dir_ok) in [
    (Accepts::File, true, false),
    (Accepts::Dir, false, true),
    (Accepts::Either, true, true),
] {
    assert_eq!(a.takes(false), file_ok, "file into {a:?}", a = a as u8);
    assert_eq!(a.takes(true), dir_ok, "dir into {a:?}", a = a as u8);
}
}
