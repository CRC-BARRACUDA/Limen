//! Marking a row by severity.

use limen_ui::*;

fn table_with(levels: &str) -> View {
    let json = format!(
        r#"{{"title":"t","widgets":[{{
            "kind":"table",
            "columns":["Level","Name"],
            "rows":[["error","a"],["warning","b"],["","c"]],
            "row_levels":{levels}
        }}]}}"#
    );
    serde_json::from_str(&json).expect("the view deserialises")
}

/// The field is optional. Every table written before it existed, and every one
/// that does not mark anything, must go on deserialising.
#[test]
fn a_table_without_levels_is_still_a_table() {
    let v: View = serde_json::from_str(
        r#"{"title":"t","widgets":[{"kind":"table","columns":["A"],"rows":[["1"]]}]}"#,
    )
    .expect("no row_levels is fine");
    match &v.widgets[0] {
        Widget::Table { row_levels, rows, .. } => {
            assert!(row_levels.is_empty(), "nothing marked");
            assert_eq!(rows.len(), 1);
        }
        other => panic!("expected a table, got {other:?}"),
    }
}

/// What a module sends arrives in order, alongside its rows.
#[test]
fn the_levels_arrive_beside_the_rows_they_mark() {
    let v = table_with(r#"["error","warning",""]"#);
    match &v.widgets[0] {
        Widget::Table { row_levels, rows, .. } => {
            assert_eq!(row_levels.len(), rows.len());
            assert_eq!(row_levels[0], "error");
            assert_eq!(row_levels[1], "warning");
            assert!(row_levels[2].is_empty());
        }
        other => panic!("expected a table, got {other:?}"),
    }
}

/// A short list must not cost the rows past it. A module that marks the first
/// row and stops — or gets the length wrong — should still get its whole table
/// drawn, with the rest of the rows ordinary.
#[test]
fn a_short_or_long_list_does_not_lose_a_row() {
    for levels in [r#"["error"]"#, r#"["error","warning","","ok","error"]"#] {
        let v = table_with(levels);
        match &v.widgets[0] {
            Widget::Table { rows, .. } => assert_eq!(rows.len(), 3, "{levels}"),
            other => panic!("expected a table, got {other:?}"),
        }
    }
}

/// The words are the notice's words. A module that says severity one way for a
/// toast should not have to learn a second vocabulary for a table.
#[test]
fn the_levels_are_the_words_a_notice_takes() {
    // `notice` accepts these, and so must a row.
    for level in ["ok", "warning", "error"] {
        let v = table_with(&format!(r#"["{level}","",""]"#));
        match &v.widgets[0] {
            Widget::Table { row_levels, .. } => assert_eq!(row_levels[0], level),
            other => panic!("expected a table, got {other:?}"),
        }
    }
    // And an unknown word is not an error — it is an ordinary row. A module
    // from a later version naming a level this build has never heard of must
    // still get its table.
    let v = table_with(r#"["banana","",""]"#);
    match &v.widgets[0] {
        Widget::Table { row_levels, rows, .. } => {
            assert_eq!(row_levels[0], "banana");
            assert_eq!(rows.len(), 3, "the table is drawn regardless");
        }
        other => panic!("expected a table, got {other:?}"),
    }
}

/// The gallery shows it, because the gallery is where a module author finds out
/// the thing exists at all.
#[test]
fn the_ui_kit_marks_a_row() {
    let shown = demo_view().to_string();
    assert!(shown.contains("row_levels"), "the UI Kit does not mark a row");
    assert!(shown.contains(r#""error""#));
}
