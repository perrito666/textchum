//! Highlighting behavior tests, through the public Document API.

use textchum_core::{theme, Document, HighlightSpan};

fn doc_with(language: &str, text: &str) -> Document {
    let mut doc = Document::new();
    doc.replace_utf16(0, 0, text).unwrap();
    assert!(doc.set_language(Some(language)), "language {language} known");
    doc
}

fn styles_at(spans: &[HighlightSpan], position: usize) -> Vec<u32> {
    spans
        .iter()
        .filter(|s| s.start_utf16 <= position && position < s.end_utf16)
        .map(|s| s.style)
        .collect()
}

fn style(name: &str) -> u32 {
    theme::resolve(name).unwrap_or_else(|| panic!("{name} must be a styled capture"))
}

#[test]
fn rust_keywords_strings_and_comments_are_styled() {
    let source = "// greet\nfn main() { let s = \"hi\"; }\n";
    let doc = doc_with("rust", source);
    let spans = doc.highlights(0, doc.len_utf16()).unwrap();
    assert!(!spans.is_empty());

    let comment_pos = source.find("greet").unwrap();
    assert!(styles_at(&spans, comment_pos).contains(&style("comment")));
    let fn_pos = source.find("fn").unwrap();
    assert!(styles_at(&spans, fn_pos).contains(&style("keyword")));
    let string_pos = source.find("\"hi\"").unwrap() + 1;
    assert!(styles_at(&spans, string_pos).contains(&style("string")));
}

#[test]
fn plain_text_documents_have_no_spans() {
    let mut doc = Document::new();
    doc.replace_utf16(0, 0, "fn main() {}").unwrap();
    assert!(doc.highlights(0, doc.len_utf16()).unwrap().is_empty());
    assert_eq!(doc.language_name(), None);
}

#[test]
fn unknown_language_is_rejected() {
    let mut doc = Document::new();
    assert!(!doc.set_language(Some("clearly-not-a-language")));
    assert!(doc.set_language(None));
}

#[test]
fn incremental_edits_match_a_fresh_parse() {
    let mut doc = doc_with("rust", "fn a() {}\nfn b() {}\n");
    // A series of edits that reshape the tree.
    doc.replace_utf16(3, 4, "renamed").unwrap();
    doc.replace_utf16(0, 0, "// header\n").unwrap();
    let len = doc.len_utf16();
    doc.replace_utf16(len, len, "struct S { field: u32 }\n").unwrap();

    let incremental = doc.highlights(0, doc.len_utf16()).unwrap();
    let fresh = doc_with("rust", &doc.text());
    let reparsed = fresh.highlights(0, fresh.len_utf16()).unwrap();
    assert_eq!(
        incremental, reparsed,
        "incrementally maintained tree must highlight identically to a fresh parse"
    );
}

#[test]
fn undo_keeps_the_tree_in_sync() {
    let mut doc = doc_with("rust", "fn main() {}\n");
    doc.break_undo_group();
    let len = doc.len_utf16();
    doc.replace_utf16(len, len, "const X: u32 = 1;\n").unwrap();
    doc.undo();

    let after_undo = doc.highlights(0, doc.len_utf16()).unwrap();
    let fresh = doc_with("rust", &doc.text());
    assert_eq!(after_undo, fresh.highlights(0, fresh.len_utf16()).unwrap());
}

#[test]
fn markdown_injects_inline_and_fence_languages() {
    let source = "# Title\n\nsome *emphasis* here\n\n```rust\nfn main() {}\n```\n";
    let doc = doc_with("markdown", source);
    let spans = doc.highlights(0, doc.len_utf16()).unwrap();

    let title_pos = source.find("Title").unwrap();
    assert!(
        styles_at(&spans, title_pos).contains(&style("text.title")),
        "heading styled by the block grammar"
    );
    let emphasis_pos = source.find("emphasis").unwrap();
    assert!(
        styles_at(&spans, emphasis_pos).contains(&style("text.emphasis")),
        "emphasis styled via the injected inline grammar"
    );
    let fn_pos = source.find("fn main").unwrap();
    assert!(
        styles_at(&spans, fn_pos).contains(&style("keyword")),
        "fenced rust styled via the injected rust grammar"
    );
}

#[test]
fn range_queries_only_return_overlapping_spans() {
    let source = "fn a() {}\nfn b() {}\n";
    let doc = doc_with("rust", source);
    let second_fn = source.rfind("fn").unwrap();
    let spans = doc.highlights(second_fn, doc.len_utf16()).unwrap();
    assert!(!spans.is_empty());
    assert!(
        spans.iter().all(|s| s.end_utf16 > second_fn),
        "no span may end before the requested range starts"
    );
}

#[test]
fn language_detected_from_extension_on_open() {
    let dir = std::env::temp_dir().join(format!("textchum-syn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hello.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let doc = Document::open(&path).unwrap();
    assert_eq!(doc.language_name(), Some("rust"));
    assert!(!doc.highlights(0, doc.len_utf16()).unwrap().is_empty());
}
