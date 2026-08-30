//! The delimiters that come in pairs, and what wrapping does.
//!
//! Typing an opening delimiter with text selected wraps the selection
//! in the pair instead of replacing it, which is what every editor in
//! this class does. Both shells funnel typing through one place and
//! ask here, so they wrap the same things in the same way.
//!
//! `<` is left out on purpose. It opens a bracket in a handful of
//! languages and compares two numbers in most of them, and wrapping a
//! selection in `<>` when someone meant `a < b` is worse than typing
//! the closing bracket by hand.

/// The closing half of `open`, when `open` is a delimiter that wraps.
pub fn closing(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '\'' => Some('\''),
        '"' => Some('"'),
        '`' => Some('`'),
        _ => None,
    }
}

/// Whether typing `text` over a selection should wrap it: one
/// character, and that character a delimiter. A paste of several
/// characters replaces the selection, as it always did.
pub fn wraps(text: &str) -> Option<(char, char)> {
    let mut characters = text.chars();
    let open = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    closing(open).map(|close| (open, close))
}

/// `selection` wrapped in the pair `open` belongs to, or None when
/// `open` is not a delimiter or there is nothing selected.
///
/// The caller keeps the selection on what was wrapped rather than on
/// the whole, which is what lets a second delimiter nest inside the
/// first: `[`, then `(`, gives `[(hello)]`.
pub fn wrap(selection: &str, open: &str) -> Option<String> {
    if selection.is_empty() {
        return None;
    }
    let (open, close) = wraps(open)?;
    Some(format!("{open}{selection}{close}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delimiter_wraps_the_selection() {
        assert_eq!(wrap("hello", "[").as_deref(), Some("[hello]"));
        assert_eq!(wrap("hello", "(").as_deref(), Some("(hello)"));
        assert_eq!(wrap("hello", "{").as_deref(), Some("{hello}"));
        assert_eq!(wrap("hello", "\"").as_deref(), Some("\"hello\""));
        assert_eq!(wrap("hello", "'").as_deref(), Some("'hello'"));
        assert_eq!(wrap("hello", "`").as_deref(), Some("`hello`"));
    }

    #[test]
    fn one_wrap_nests_inside_another() {
        // What the editor does with the selection kept on the inside:
        // three delimiters in a row give three pairs.
        let once = wrap("hello", "[").unwrap();
        assert_eq!(once, "[hello]");
        let twice = wrap("hello", "(").unwrap();
        assert_eq!(format!("[{twice}]"), "[(hello)]");
        let thrice = wrap("hello", "{").unwrap();
        assert_eq!(format!("[({thrice})]"), "[({hello})]");
    }

    #[test]
    fn anything_else_replaces_the_selection() {
        assert_eq!(wrap("hello", "x"), None);
        assert_eq!(wrap("hello", "<"), None);
        assert_eq!(wrap("hello", ")"), None);
        // A paste is not a keystroke.
        assert_eq!(wrap("hello", "[]"), None);
        assert_eq!(wrap("", "["), None);
    }

    #[test]
    fn a_closing_half_is_known_for_each_opening_one() {
        for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
            assert_eq!(closing(open), Some(close));
        }
        assert_eq!(closing('<'), None);
    }
}
