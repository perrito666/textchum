//! SQL highlighting. CASE/WHEN arrive as `conditional` and decimals as
//! `float`; both are captures of their own, so a theme decides how they
//! look.

use textchum_core::theme;

fn spans(source: &str) -> Vec<(String, u32)> {
    let mut doc = textchum_core::Document::new();
    doc.replace_utf16(0, 0, source).unwrap();
    doc.set_language(Some("sql"));
    assert_eq!(doc.language_name(), Some("sql"));
    let units: Vec<u16> = source.encode_utf16().collect();
    doc.highlights(0, units.len())
        .unwrap()
        .iter()
        .map(|s| {
            (
                String::from_utf16_lossy(&units[s.start_utf16..s.end_utf16]),
                s.style,
            )
        })
        .collect()
}

/// The style a span actually paints with: spans arrive in application
/// order and a later one wins, so the last match is the answer. A
/// narrowing pattern — `@number` over a broad `@string` — is later by
/// construction.
fn style_of(found: &[(String, u32)], text: &str) -> Option<u32> {
    found.iter().filter(|(t, _)| t == text).map(|(_, s)| *s).next_back()
}

#[test]
fn a_query_highlights_its_parts() {
    let found = spans(
        "-- berths in use\n\
         SELECT name, COUNT(*) AS n\n\
         FROM berths\n\
         WHERE port = 'harbor'\n",
    );
    assert_eq!(style_of(&found, "-- berths in use"), theme::resolve("comment"));
    assert_eq!(style_of(&found, "SELECT"), theme::resolve("keyword"));
    assert_eq!(style_of(&found, "FROM"), theme::resolve("keyword"));
    assert_eq!(style_of(&found, "'harbor'"), theme::resolve("string"));
}

#[test]
fn case_expressions_and_decimals_are_not_left_plain() {
    // The grammar captures every literal as a string first and narrows
    // numbers back out with predicates it wrote in Lua syntax, which
    // never hold here. Both numbers below were painted like 'deep'.
    let found = spans(
        "SELECT CASE WHEN depth > 4.5 THEN 'deep' ELSE 'shallow' END, 42\nFROM berths;\n",
    );
    // The grammar files these under `conditional`, which is a capture
    // of its own — a theme can colour a CASE apart from a SELECT.
    for word in ["CASE", "WHEN", "THEN", "ELSE"] {
        assert_eq!(
            style_of(&found, word),
            theme::resolve("conditional"),
            "{word} is left plain unless the theme knows `conditional`"
        );
    }
    assert_eq!(style_of(&found, "END"), theme::resolve("keyword"));
    assert_eq!(
        style_of(&found, "4.5"),
        theme::resolve("float"),
        "a decimal carries the grammar's own name for it"
    );
    assert_eq!(style_of(&found, "42"), theme::resolve("number"));
}

#[test]
fn sql_is_offered_and_detected() {
    use textchum_core::syntax::languages;
    assert!(languages::selectable_names().contains(&"sql"));
    assert_eq!(
        languages::by_path(std::path::Path::new("/tmp/report.sql")).map(|e| e.spec.name),
        Some("sql")
    );
}
