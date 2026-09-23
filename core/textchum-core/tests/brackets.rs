//! The document's bracket structure: strings and comments left out,
//! kept until an edit, gone past the ceiling.

use textchum_core::Document;

#[test]
fn a_bracket_in_a_string_or_comment_opens_nothing() {
    let mut doc = Document::new();
    doc.set_language(Some("rust"));
    let text = "fn f() { let s = \")\"; // (\n    g([1]);\n}\n";
    doc.replace_utf16(0, 0, text).unwrap();
    let brackets = doc.brackets();
    let unpaired: Vec<usize> =
        brackets.iter().filter(|b| b.partner.is_none()).map(|b| b.offset).collect();
    assert!(unpaired.is_empty(), "every real bracket has a partner: {unpaired:?}");
    // `(` at 4, `{` at 7, `(` after g, `[`: the string's `)` and the
    // comment's `(` are not in the list at all.
    let offsets: Vec<usize> = brackets.iter().map(|b| b.offset).collect();
    let string_paren = text.find("\")\"").unwrap() + 1;
    let comment_paren = text.find("// (").unwrap() + 3;
    assert!(!offsets.contains(&string_paren));
    assert!(!offsets.contains(&comment_paren));
    // Depths: the body's brace is 0, the call's parenthesis 1, the
    // slice's bracket 2.
    let depth_at = |needle: &str| {
        let at = text.find(needle).unwrap();
        brackets.iter().find(|b| b.offset == at).map(|b| b.depth)
    };
    assert_eq!(depth_at("{"), Some(0));
    assert_eq!(depth_at("([1]"), Some(1));
    assert_eq!(depth_at("[1]"), Some(2));
    // The caret after the slice's `]` finds the `[`.
    let close = text.find("]").unwrap();
    assert_eq!(doc.matching_bracket(close + 1), Some((close, close - 2)));
}

#[test]
fn the_structure_follows_edits() {
    let mut doc = Document::new();
    doc.replace_utf16(0, 0, "(a)").unwrap();
    assert_eq!(doc.matching_bracket(3), Some((2, 0)));
    doc.replace_utf16(1, 1, "[").unwrap();
    // Now `([a)`: the `)` has no partner of its kind... it looks past
    // the `[` and finds the `(`.
    assert_eq!(doc.matching_bracket(4), Some((3, 0)));
    assert_eq!(doc.bracket_depths(0, 4), vec![(0, 0), (3, 0)]);
    doc.undo();
    assert_eq!(doc.matching_bracket(3), Some((2, 0)));
}

#[test]
fn past_the_ceiling_there_is_no_structure() {
    let mut doc = Document::new();
    let big = "()".repeat(Document::BRACKETS_CEILING_UTF16 / 2 + 1);
    doc.replace_utf16(0, 0, &big).unwrap();
    assert!(doc.brackets().is_empty());
    assert_eq!(doc.matching_bracket(1), None);
}
