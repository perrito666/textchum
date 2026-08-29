//! Transformations over a stretch of text.
//!
//! Sort these lines, drop the duplicates, make this upper case, take the
//! trailing whitespace off, convert the line endings. Every editor in
//! this class has some of these and they are all the same shape: text
//! in, text out, no cursor and no document.
//!
//! Which means the rules live here, once, rather than twice in two
//! shells — including the one rule that is not obvious: an operation
//! over lines is given whole lines, so sorting a selection that starts
//! mid-line sorts that line too rather than the fragment of it.

/// What to do to the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    UpperCase,
    LowerCase,
    /// The first letter of each word up, the rest down.
    TitleCase,
    /// Each letter the other way round.
    InvertCase,
    SortLines,
    SortLinesReversed,
    /// Keeps the first of each run of equal lines, in place.
    RemoveDuplicateLines,
    /// One line, single-spaced, with each line's own indentation gone.
    JoinLines,
    TrimTrailingWhitespace,
    ToUnixLineEndings,
    ToWindowsLineEndings,
}

impl Transform {
    /// The name the configuration and the C ABI use.
    pub fn id(self) -> &'static str {
        match self {
            Self::UpperCase => "upper",
            Self::LowerCase => "lower",
            Self::TitleCase => "title",
            Self::InvertCase => "invert",
            Self::SortLines => "sort",
            Self::SortLinesReversed => "sort-reversed",
            Self::RemoveDuplicateLines => "dedupe",
            Self::JoinLines => "join",
            Self::TrimTrailingWhitespace => "trim",
            Self::ToUnixLineEndings => "lf",
            Self::ToWindowsLineEndings => "crlf",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [
            Self::UpperCase,
            Self::LowerCase,
            Self::TitleCase,
            Self::InvertCase,
            Self::SortLines,
            Self::SortLinesReversed,
            Self::RemoveDuplicateLines,
            Self::JoinLines,
            Self::TrimTrailingWhitespace,
            Self::ToUnixLineEndings,
            Self::ToWindowsLineEndings,
        ]
        .into_iter()
        .find(|transform| transform.id() == id)
    }

    /// Whether this one is about lines rather than characters.
    ///
    /// A line-wise transformation is given whole lines: the shell grows
    /// the selection to the line boundaries around it first, because
    /// sorting half a line is not something anyone asked for.
    pub fn is_line_wise(self) -> bool {
        matches!(
            self,
            Self::SortLines
                | Self::SortLinesReversed
                | Self::RemoveDuplicateLines
                | Self::JoinLines
                | Self::TrimTrailingWhitespace
        )
    }
}

/// Applies a transformation. Line endings are kept: text that came in
/// with CRLF goes out with CRLF, unless the transformation is about
/// line endings.
pub fn apply(transform: Transform, text: &str) -> String {
    use Transform::*;
    match transform {
        UpperCase => text.to_uppercase(),
        LowerCase => text.to_lowercase(),
        TitleCase => title_case(text),
        InvertCase => text.chars().flat_map(invert_char).collect(),
        SortLines => over_lines(text, |lines| lines.sort()),
        SortLinesReversed => over_lines(text, |lines| {
            lines.sort();
            lines.reverse();
        }),
        RemoveDuplicateLines => over_lines(text, |lines| {
            let mut seen = std::collections::HashSet::new();
            lines.retain(|line| seen.insert(line.to_string()));
        }),
        JoinLines => join_lines(text),
        TrimTrailingWhitespace => over_lines(text, |lines| {
            for line in lines.iter_mut() {
                *line = line.trim_end();
            }
        }),
        ToUnixLineEndings => text.replace("\r\n", "\n").replace('\r', "\n"),
        ToWindowsLineEndings => text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', "\r\n"),
    }
}

/// Runs `change` over the lines, and puts them back the way they came:
/// same line ending, and a trailing newline only if there was one.
fn over_lines(text: &str, change: impl FnOnce(&mut Vec<&str>)) -> String {
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let trailing = text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(text);
    let body = body.strip_suffix('\r').unwrap_or(body);
    let mut lines: Vec<&str> = body.split('\n').map(|line| line.trim_end_matches('\r')).collect();
    change(&mut lines);
    let mut out = lines.join(ending);
    if trailing {
        out.push_str(ending);
    }
    out
}

/// Every line on one, single-spaced. Each line's own indentation goes:
/// it was there to sit under the line above, and there is no line above
/// any more.
fn join_lines(text: &str) -> String {
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let trailing = text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(text);
    let body = body.strip_suffix('\r').unwrap_or(body);
    let mut out = String::new();
    for (at, line) in body.split('\n').enumerate() {
        let line = line.trim_end_matches('\r');
        if at == 0 {
            out.push_str(line.trim_end());
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push_str(line);
    }
    if trailing {
        out.push_str(ending);
    }
    out
}

/// The first letter of each word up, the rest down. A word starts after
/// anything that is not a letter, a digit or an apostrophe — so
/// `don't` is one word and `well-known` is two.
fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut starting = true;
    for character in text.chars() {
        if starting {
            out.extend(character.to_uppercase());
        } else {
            out.extend(character.to_lowercase());
        }
        starting = !(character.is_alphanumeric() || character == '\'');
    }
    out
}

fn invert_char(character: char) -> Vec<char> {
    if character.is_lowercase() {
        character.to_uppercase().collect()
    } else if character.is_uppercase() {
        character.to_lowercase().collect()
    } else {
        vec![character]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, text: &str) -> String {
        apply(Transform::from_id(id).unwrap(), text)
    }

    #[test]
    fn case_goes_both_ways_and_back() {
        assert_eq!(run("upper", "hello, World"), "HELLO, WORLD");
        assert_eq!(run("lower", "HELLO, World"), "hello, world");
        assert_eq!(run("invert", "Hello, World"), "hELLO, wORLD");
        // Inverting twice is where it started.
        assert_eq!(run("invert", &run("invert", "Hello, World")), "Hello, World");
    }

    #[test]
    fn title_case_starts_each_word() {
        assert_eq!(run("title", "the quick brown fox"), "The Quick Brown Fox");
        // An apostrophe is part of the word; a hyphen is not.
        assert_eq!(run("title", "don't be well-known"), "Don't Be Well-Known");
        // And the rest of a shouted word comes down.
        assert_eq!(run("title", "HELLO there"), "Hello There");
    }

    #[test]
    fn lines_sort_and_keep_their_shape() {
        assert_eq!(run("sort", "pear\napple\ncherry"), "apple\ncherry\npear");
        // A trailing newline was there, so it stays there.
        assert_eq!(run("sort", "pear\napple\n"), "apple\npear\n");
        assert_eq!(run("sort-reversed", "apple\npear\ncherry"), "pear\ncherry\napple");
    }

    #[test]
    fn duplicates_go_and_the_order_stays() {
        assert_eq!(
            run("dedupe", "pear\napple\npear\ncherry\napple"),
            "pear\napple\ncherry"
        );
    }

    #[test]
    fn joining_takes_the_indentation_with_it() {
        assert_eq!(
            run("join", "the quick\n    brown fox\n    jumps"),
            "the quick brown fox jumps"
        );
        // A blank line joins nothing; it was spacing.
        assert_eq!(run("join", "one\n\ntwo"), "one two");
    }

    #[test]
    fn trailing_whitespace_goes_and_nothing_else_does() {
        assert_eq!(run("trim", "one   \n  two\t\nthree"), "one\n  two\nthree");
    }

    #[test]
    fn line_endings_convert_both_ways() {
        assert_eq!(run("lf", "one\r\ntwo\r\n"), "one\ntwo\n");
        assert_eq!(run("crlf", "one\ntwo\n"), "one\r\ntwo\r\n");
        // Already converted is a no-op, not a doubling.
        assert_eq!(run("crlf", "one\r\ntwo"), "one\r\ntwo");
        // A lone carriage return is a line ending too.
        assert_eq!(run("lf", "one\rtwo"), "one\ntwo");
    }

    #[test]
    fn crlf_text_keeps_its_endings_through_a_line_operation() {
        assert_eq!(run("sort", "pear\r\napple\r\n"), "apple\r\npear\r\n");
    }

    #[test]
    fn line_operations_say_they_are_about_lines() {
        assert!(Transform::SortLines.is_line_wise());
        assert!(Transform::TrimTrailingWhitespace.is_line_wise());
        assert!(!Transform::UpperCase.is_line_wise());
        assert!(!Transform::ToUnixLineEndings.is_line_wise());
    }

    #[test]
    fn an_unknown_name_is_no_transformation() {
        assert_eq!(Transform::from_id("nonexistent"), None);
        // And every one of them round-trips through its name.
        for id in [
            "upper", "lower", "title", "invert", "sort", "sort-reversed", "dedupe", "join",
            "trim", "lf", "crlf",
        ] {
            assert_eq!(Transform::from_id(id).map(Transform::id), Some(id));
        }
    }
}
