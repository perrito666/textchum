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

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// A language's own table of pairs, as the configuration spells it:
/// each entry the opening half and the closing one.
pub type Table = Vec<(char, char)>;

/// `entries` read as a table — `"()"`, `"''"`, `"<>"`. An entry that
/// is not exactly two characters is not a pair and is left out rather
/// than guessed at.
pub fn table<I, S>(entries: I) -> Table
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    entries
        .into_iter()
        .filter_map(|entry| {
            let mut characters = entry.as_ref().chars();
            let pair = (characters.next()?, characters.next()?);
            characters.next().is_none().then_some(pair)
        })
        .collect()
}

/// The closing half of `open` in `table`, or None when no pair there
/// opens with it.
pub fn closing_in(table: &[(char, char)], open: char) -> Option<char> {
    table.iter().find(|(o, _)| *o == open).map(|(_, close)| *close)
}

/// [`auto_close`] with a language's own table in place of the built-in
/// rule. The table says what pairs; the rest still holds: no pair
/// opens into a word, and a pair whose halves are the same character
/// is a quote, not opened right after a word nor before another of
/// itself.
pub fn auto_close_in(
    table: &[(char, char)],
    typed: char,
    before: Option<char>,
    after: Option<char>,
) -> Option<char> {
    let close = closing_in(table, typed)?;
    if is_word(after) {
        return None;
    }
    if close == typed && (is_word(before) || after == Some(typed)) {
        return None;
    }
    Some(close)
}

/// [`skips_closer`] against a language's own table.
pub fn skips_closer_in(table: &[(char, char)], typed: char, after: Option<char>) -> bool {
    after == Some(typed) && table.iter().any(|(_, close)| *close == typed)
}

/// [`deletes_pair`] against a language's own table.
pub fn deletes_pair_in(table: &[(char, char)], before: Option<char>, after: Option<char>) -> bool {
    match (before, after) {
        (Some(open), Some(close)) => table.contains(&(open, close)),
        _ => false,
    }
}

/// [`closing`] under a language's own table when it has one, the
/// built-in pairs otherwise.
pub fn closing_with(table: Option<&[(char, char)]>, open: char) -> Option<char> {
    match table {
        Some(table) => closing_in(table, open),
        None => closing(open),
    }
}

/// [`auto_close`] under a language's own table when it has one, the
/// built-in rule by language otherwise.
pub fn auto_close_with(
    table: Option<&[(char, char)]>,
    language: Option<&str>,
    typed: char,
    before: Option<char>,
    after: Option<char>,
) -> Option<char> {
    match table {
        Some(table) => auto_close_in(table, typed, before, after),
        None => auto_close(language, typed, before, after),
    }
}

/// [`skips_closer`] under a language's own table when it has one.
pub fn skips_closer_with(table: Option<&[(char, char)]>, typed: char, after: Option<char>) -> bool {
    match table {
        Some(table) => skips_closer_in(table, typed, after),
        None => skips_closer(typed, after),
    }
}

/// [`deletes_pair`] under a language's own table when it has one.
pub fn deletes_pair_with(
    table: Option<&[(char, char)]>,
    before: Option<char>,
    after: Option<char>,
) -> bool {
    match table {
        Some(table) => deletes_pair_in(table, before, after),
        None => deletes_pair(before, after),
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
    fn a_language_s_own_table_is_the_whole_answer() {
        let table = table(["()", "<>", "''", "x", "abc", ""]);
        assert_eq!(table, vec![('(', ')'), ('<', '>'), ('\'', '\'')]);
        assert_eq!(auto_close_in(&table, '<', None, None), Some('>'));
        assert_eq!(auto_close_in(&table, '[', None, None), None, "not in the table");
        assert_eq!(auto_close_in(&table, '(', None, Some('x')), None, "still not into a word");
        assert_eq!(auto_close_in(&table, '\'', Some('t'), None), None, "a quote, still not after a word");
        assert_eq!(auto_close_in(&table, '\'', None, Some('\'')), None);
        assert!(skips_closer_in(&table, '>', Some('>')));
        assert!(!skips_closer_in(&table, ']', Some(']')));
        assert!(deletes_pair_in(&table, Some('<'), Some('>')));
        assert!(!deletes_pair_in(&table, Some('['), Some(']')));
        assert_eq!(closing_in(&table, '<'), Some('>'));
        assert_eq!(closing_in(&table, '['), None);
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
