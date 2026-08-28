//! Python highlighting beyond what the grammar's own query says.
//!
//! `self` and `cls` are the receiver and the class, not locals, and the
//! grammar marks neither.

use textchum_core::theme;

fn spans(source: &str) -> Vec<(String, u32)> {
    let mut doc = textchum_core::Document::new();
    doc.replace_utf16(0, 0, source).unwrap();
    doc.set_language(Some("python"));
    assert_eq!(doc.language_name(), Some("python"));
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

/// Spans arrive in application order and a later one wins, so the last
/// match for a piece of text is the style it paints with.
fn style_of(found: &[(String, u32)], text: &str) -> Option<u32> {
    found
        .iter()
        .filter(|(t, _)| t == text)
        .map(|(_, s)| *s)
        .next_back()
}

const SOURCE: &str = "class Berth:\n\
                      \x20   def __init__(self, name):\n\
                      \x20       self.name = name\n\
                      \x20       self.free = True\n\
                      \n\
                      \x20   @classmethod\n\
                      \x20   def empty(cls):\n\
                      \x20       return cls(\"\")\n";

#[test]
fn the_receiver_and_the_class_are_builtin_variables() {
    let found = spans(SOURCE);
    let builtin = theme::resolve("variable.builtin").expect("variable.builtin is a capture");
    assert_eq!(style_of(&found, "self"), Some(builtin), "self: {found:?}");
    assert_eq!(style_of(&found, "cls"), Some(builtin), "cls: {found:?}");
}

#[test]
fn an_ordinary_name_is_left_alone() {
    let found = spans(SOURCE);
    let builtin = theme::resolve("variable.builtin").expect("variable.builtin is a capture");
    // The parameter shares a line with `self` and must not be dragged
    // along by a pattern that matches too much.
    assert_ne!(style_of(&found, "name"), Some(builtin), "name: {found:?}");
}

#[test]
fn a_name_that_merely_contains_self_is_not_the_receiver() {
    let found = spans("selfish = 1\nmyself = 2\ncls_name = 3\n");
    let builtin = theme::resolve("variable.builtin").expect("variable.builtin is a capture");
    for name in ["selfish", "myself", "cls_name"] {
        assert_ne!(style_of(&found, name), Some(builtin), "{name}: {found:?}");
    }
}
