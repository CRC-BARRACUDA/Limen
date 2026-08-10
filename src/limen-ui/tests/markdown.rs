//! The small Markdown renderer.

use limen_ui::*;
#[test]
fn markdown_inline_tokenizes_bold_code_and_links() {
let segs = inline_segments("**UI widgets** — a `table` and a [link](http://x)");
assert_eq!(
    segs,
    vec![
        Md::Bold("UI widgets".into()),
        Md::Text(" — a ".into()),
        Md::Code("table".into()),
        Md::Text(" and a ".into()),
        Md::Link {
            label: "link".into(),
            url: "http://x".into()
        },
    ]
);
// Unmatched markers stay literal.
assert_eq!(
    inline_segments("a ** b `c"),
    vec![Md::Text("a ** b `c".into())]
);
// Plain text is a single span.
assert_eq!(
    inline_segments("just text"),
    vec![Md::Text("just text".into())]
);
}
