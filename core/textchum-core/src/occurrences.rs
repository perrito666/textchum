//! The other places the selected word appears.
//!
//! Selecting a word and then typing it into the find bar to see where
//! else it is asks twice for one thing. Selecting it is the question;
//! marking the rest of them is the answer.
//!
//! Only a selection that is exactly one word asks it. A partial word
//! and a stretch spanning several are selections made for some other
//! reason, and marking anything for them would be noise.
//!
//! Offsets are UTF-16 code units, the same currency the rest of the
//! core speaks to the shells in.

/// How occurrences are matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Whether `Item` counts as `item`.
    pub case_sensitive: bool,
    /// Whether `item` inside `items` counts. False marks it.
    pub whole_word: bool,
}

impl Default for Options {
    fn default() -> Self {
        // The word was selected as a word, so the other words are what
        // is being asked about — and a name that differs in case is a
        // different name in every language here.
        Self {
            case_sensitive: true,
            whole_word: true,
        }
    }
}

/// A span, in UTF-16 code units, of the document the text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// The word a selection is, when the selection is exactly one.
///
/// `start` and `end` are UTF-16 offsets into `text`. `None` for an
/// empty selection, a partial word, and anything spanning a boundary.
pub fn selected_word(text: &str, start: usize, end: usize) -> Option<String> {
    if end <= start {
        return None;
    }
    let start_byte = utf16_to_byte(text, start)?;
    let end_byte = utf16_to_byte(text, end)?;
    let word = text.get(start_byte..end_byte)?;
    if word.is_empty() || !word.chars().all(is_word_char) {
        return None;
    }
    // Both ends have to be boundaries, or this is part of a longer
    // name and the selection was about something else.
    if text[..start_byte].chars().next_back().is_some_and(is_word_char)
        || text[end_byte..].chars().next().is_some_and(is_word_char)
    {
        return None;
    }
    Some(word.to_owned())
}

/// Every occurrence of `word` in `text`, as spans of the document.
///
/// `text` is the stretch to search — the visible one, so a long file
/// costs what a short one does — and `base` is its UTF-16 offset in the
/// document, which the spans are returned in.
pub fn occurrences(text: &str, word: &str, base: usize, options: Options) -> Vec<Span> {
    if word.is_empty() {
        return Vec::new();
    }
    let haystack = if options.case_sensitive {
        text.to_owned()
    } else {
        text.to_lowercase()
    };
    let needle = if options.case_sensitive {
        word.to_owned()
    } else {
        word.to_lowercase()
    };
    // Lowercasing can change a string's length (İ becomes two chars),
    // which would put every offset after it out by that much. Case
    // folding is only offered for text where it does not.
    if haystack.len() != text.len() || needle.len() != word.len() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut at = 0;
    while let Some(found) = haystack[at..].find(&needle) {
        let start = at + found;
        let end = start + needle.len();
        at = start + needle.len().max(1);
        if options.whole_word {
            let before = text[..start].chars().next_back();
            let after = text[end..].chars().next();
            if before.is_some_and(is_word_char) || after.is_some_and(is_word_char) {
                continue;
            }
        }
        spans.push(Span {
            start: base + utf16_len(&text[..start]),
            end: base + utf16_len(&text[..end]),
        });
    }
    spans
}

/// The spans as JSON — `[{"start": 12, "end": 16}, …]` — for shells
/// that reach the core through the C ABI.
pub fn to_json(spans: &[Span]) -> String {
    let items: Vec<serde_json::Value> = spans
        .iter()
        .map(|span| serde_json::json!({"start": span.start, "end": span.end}))
        .collect();
    serde_json::Value::Array(items).to_string()
}

/// Letters, digits and underscore: what a name is made of in every
/// language here.
fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// A UTF-16 offset as a byte offset, or `None` when it lands inside a
/// character — which a shell's own offsets never do, but a stale one
/// might.
fn utf16_to_byte(text: &str, offset: usize) -> Option<usize> {
    if offset == 0 {
        return Some(0);
    }
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += character.len_utf16();
    }
    (units == offset).then_some(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_word_is_the_question() {
        let text = "let item = items.first();";
        assert_eq!(selected_word(text, 4, 8).as_deref(), Some("item"));
    }

    #[test]
    fn a_partial_word_asks_nothing() {
        let text = "let item = items.first();";
        // "ite" inside "item".
        assert_eq!(selected_word(text, 4, 7), None);
        // "tem" inside "item".
        assert_eq!(selected_word(text, 5, 8), None);
        // Empty.
        assert_eq!(selected_word(text, 4, 4), None);
    }

    #[test]
    fn several_words_ask_nothing() {
        let text = "let item = items.first();";
        assert_eq!(selected_word(text, 0, 8), None);
        assert_eq!(selected_word(text, 4, 10), None);
    }

    #[test]
    fn the_other_words_are_marked() {
        let text = "item = item + items";
        let found = occurrences(text, "item", 0, Options::default());
        assert_eq!(
            found,
            vec![Span { start: 0, end: 4 }, Span { start: 7, end: 11 }]
        );
    }

    #[test]
    fn a_word_inside_a_longer_one_counts_when_asked() {
        let text = "item = items";
        let options = Options {
            whole_word: false,
            ..Options::default()
        };
        let found = occurrences(text, "item", 0, options);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1], Span { start: 7, end: 11 });
    }

    #[test]
    fn case_is_respected_unless_it_is_not() {
        let text = "item Item ITEM";
        assert_eq!(occurrences(text, "item", 0, Options::default()).len(), 1);
        let options = Options {
            case_sensitive: false,
            ..Options::default()
        };
        assert_eq!(occurrences(text, "item", 0, options).len(), 3);
    }

    #[test]
    fn spans_are_offset_by_where_the_visible_text_starts() {
        let found = occurrences("item", "item", 1000, Options::default());
        assert_eq!(found, vec![Span { start: 1000, end: 1004 }]);
    }

    #[test]
    fn offsets_are_utf16_units() {
        // Each emoji is one char and two UTF-16 units.
        let text = "🌱🌱 item";
        let found = occurrences(text, "item", 0, Options::default());
        assert_eq!(found, vec![Span { start: 5, end: 9 }]);
        assert_eq!(selected_word(text, 5, 9).as_deref(), Some("item"));
    }

    #[test]
    fn nothing_is_marked_for_nothing() {
        assert!(occurrences("item", "", 0, Options::default()).is_empty());
        assert!(occurrences("", "item", 0, Options::default()).is_empty());
    }

    #[test]
    fn the_json_carries_the_spans() {
        let found = occurrences("item item", "item", 0, Options::default());
        assert_eq!(
            to_json(&found),
            r#"[{"end":4,"start":0},{"end":9,"start":5}]"#
        );
    }
}
