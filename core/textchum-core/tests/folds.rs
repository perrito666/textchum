//! What can be folded, and what is deliberately not offered.

fn folds(language: &str, source: &str) -> Vec<(usize, usize)> {
    let mut doc = textchum_core::Document::new();
    doc.replace_utf16(0, 0, source).unwrap();
    doc.set_language(Some(language));
    doc.fold_ranges()
}

#[test]
fn a_function_body_folds_from_the_line_that_opens_it() {
    let source = "fn main() {\n    let a = 1;\n    let b = 2;\n}\n";
    let ranges = folds("rust", source);
    // Line 0 opens it, line 3 closes it.
    assert!(
        ranges.contains(&(0, 3)),
        "the function is not foldable: {ranges:?}"
    );
}

#[test]
fn nested_blocks_each_get_a_fold() {
    let source = "\
impl Item {
    fn label(&self) -> String {
        format!(\"{}\", self.name)
    }
}
";
    let ranges = folds("rust", source);
    assert!(ranges.contains(&(0, 4)), "the impl: {ranges:?}");
    assert!(ranges.contains(&(1, 3)), "the method: {ranges:?}");
}

#[test]
fn several_blocks_opening_on_one_line_offer_the_widest() {
    // `impl Item {` opens both the impl and its body; folding the impl
    // is what was meant, and two folds on one line is one too many.
    let source = "impl Item {\n    fn a() {}\n    fn b() {}\n}\n";
    let ranges = folds("rust", source);
    let on_first: Vec<&(usize, usize)> =
        ranges.iter().filter(|(start, _)| *start == 0).collect();
    assert_eq!(on_first.len(), 1, "one fold per opening line: {ranges:?}");
    assert_eq!(on_first[0].1, 3);
}

#[test]
fn a_block_that_saves_one_line_is_not_offered() {
    // Folding this would hide the closing brace and save one line,
    // which is not worth an arrow in the gutter on every other line.
    let source = "fn main() {\n}\n";
    let ranges = folds("rust", source);
    assert!(
        !ranges.iter().any(|(start, _)| *start == 0),
        "a two-line block was offered: {ranges:?}"
    );
}

#[test]
fn python_folds_by_its_blocks() {
    let source = "\
def greet(name):
    if name:
        return name
    return \"\"
";
    let ranges = folds("python", source);
    assert!(ranges.contains(&(0, 3)), "the function: {ranges:?}");
    // Its body, from the first statement — folding that leaves the
    // `if` visible and hides what follows.
    assert!(ranges.contains(&(1, 3)), "the body: {ranges:?}");
    // The `if` itself hides one line, which is not a fold worth an
    // arrow in the gutter.
    assert!(!ranges.contains(&(1, 2)), "a one-line fold was offered: {ranges:?}");
}

#[test]
fn plain_text_has_nothing_to_fold() {
    let mut doc = textchum_core::Document::new();
    doc.replace_utf16(0, 0, "one\ntwo\nthree\n").unwrap();
    assert!(doc.fold_ranges().is_empty());
}

#[test]
fn the_json_carries_the_ranges() {
    let mut doc = textchum_core::Document::new();
    doc.replace_utf16(0, 0, "fn main() {\n    let a = 1;\n}\n").unwrap();
    doc.set_language(Some("rust"));
    assert!(doc.fold_ranges_json().contains(r#"{"end":2,"start":0}"#));
}
