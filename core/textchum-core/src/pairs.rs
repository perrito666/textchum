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

/// Whether typing `typed` should put its closing half after the caret
/// as well, in `language`, with `before` and `after` the characters on
/// either side of the caret. The closer to insert, or None.
///
/// Brackets close everywhere. Quotes are the language's business: an
/// apostrophe in prose is not a quote, a `'` in Rust is a lifetime as
/// often as a character, and a backtick is code in Markdown and a
/// template in JavaScript but nothing in most languages. And a pair
/// is not opened into a word — `foo|bar` typing `"` is a `"`, not a
/// `""` — nor a quote right after one, since `it's` is not a string.
pub fn auto_close(language: Option<&str>, typed: char, before: Option<char>, after: Option<char>) -> Option<char> {
    let close = closing(typed)?;
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    if is_word(after) {
        return None;
    }
    let is_quote = typed == '\'' || typed == '"' || typed == '`';
    if is_quote {
        if is_word(before) || after == Some(typed) {
            return None;
        }
        let language = language.unwrap_or("");
        let prose = matches!(language, "" | "markdown" | "gitcommit" | "text");
        match typed {
            '\'' if prose || matches!(language, "rust" | "ocaml") => return None,
            '"' if language.is_empty() => return None,
            '`' if !matches!(
                language,
                "markdown" | "javascript" | "typescript" | "tsx" | "jsx" | "go" | "bash" | "shell"
                    | "sh" | "zsh" | "kotlin" | "sql" | "php" | "swift"
            ) =>
            {
                return None
            }
            _ => {}
        }
    }
    Some(close)
}

/// Whether typing `typed` with `after` already there should step over
/// it rather than insert a second: the closer the editor put there a
/// moment ago, typed by a hand that types closers.
pub fn skips_closer(typed: char, after: Option<char>) -> bool {
    after == Some(typed)
        && (matches!(typed, ')' | ']' | '}') || closing(typed) == Some(typed))
}

/// Whether Backspace between `before` and `after` should take both:
/// an empty pair the editor opened and nothing was typed into.
pub fn deletes_pair(before: Option<char>, after: Option<char>) -> bool {
    match (before, after) {
        (Some(open), Some(close)) => closing(open) == Some(close),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets_close_everywhere_but_not_into_a_word() {
        assert_eq!(auto_close(Some("rust"), '(', None, None), Some(')'));
        assert_eq!(auto_close(None, '[', Some('a'), Some(' ')), Some(']'));
        assert_eq!(auto_close(Some("python"), '{', None, Some(')')), Some('}'));
        assert_eq!(auto_close(Some("rust"), '(', None, Some('x')), None, "not into a word");
        assert_eq!(auto_close(Some("rust"), 'x', None, None), None);
        assert_eq!(auto_close(Some("rust"), '<', None, None), None);
    }

    #[test]
    fn quotes_follow_the_language() {
        assert_eq!(auto_close(Some("python"), '"', None, None), Some('"'));
        assert_eq!(auto_close(Some("python"), '\'', None, None), Some('\''));
        assert_eq!(auto_close(Some("rust"), '\'', None, None), None, "a lifetime as often as not");
        assert_eq!(auto_close(Some("rust"), '"', None, None), Some('"'));
        assert_eq!(auto_close(Some("markdown"), '\'', None, None), None, "an apostrophe in prose");
        assert_eq!(auto_close(Some("markdown"), '`', None, None), Some('`'));
        assert_eq!(auto_close(Some("javascript"), '`', None, None), Some('`'));
        assert_eq!(auto_close(Some("python"), '`', None, None), None);
        assert_eq!(auto_close(None, '"', None, None), None, "plain text has no strings");
        // Not after a word — it's — nor before the same quote.
        assert_eq!(auto_close(Some("python"), '\'', Some('t'), None), None);
        assert_eq!(auto_close(Some("python"), '"', None, Some('"')), None);
    }

    #[test]
    fn a_typed_closer_steps_over_the_one_there() {
        assert!(skips_closer(')', Some(')')));
        assert!(skips_closer('"', Some('"')));
        assert!(!skips_closer(')', Some(']')));
        assert!(!skips_closer('(', Some('(')));
        assert!(!skips_closer(')', None));
    }

    #[test]
    fn backspace_takes_an_empty_pair() {
        assert!(deletes_pair(Some('('), Some(')')));
        assert!(deletes_pair(Some('"'), Some('"')));
        assert!(!deletes_pair(Some('('), Some(']')));
        assert!(!deletes_pair(Some('a'), Some(')')));
        assert!(!deletes_pair(None, Some(')')));
    }

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
