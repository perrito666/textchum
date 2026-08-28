//! Reading a place in a file out of whatever was typed or pasted.
//!
//! "Go to line" is asked for in more shapes than a number. A compiler
//! prints `src/main.rs:412:8`. A stack trace prints `main.rs, line 412`.
//! A colleague says `412`. Someone selects the whole of a build log
//! line and pastes it. All of them name the same place, and refusing
//! any of them teaches nothing except to retype.
//!
//! So: take the text, find the first number that could be a line,
//! and take a second one after a colon as a column. Anything before,
//! after or between is ignored — including a Windows path's drive
//! colon, which is a colon that is not a separator.

/// A place in a file: a one-based line, and a one-based column when one
/// was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub line: usize,
    /// One-based. `1` when the text named no column.
    pub column: usize,
}

/// Reads a target out of `text`. `None` when there is no number in it
/// at all, which is the only input that names no line.
pub fn parse(text: &str) -> Option<Target> {
    let bytes: Vec<char> = text.chars().collect();
    let (line, after) = first_number(&bytes, 0)?;
    if line == 0 {
        // `0` is what a zero-based tool prints; the first line is what
        // it meant.
        return Some(Target { line: 1, column: 1 });
    }

    // A column only counts when a colon joins it to the line, with
    // nothing but spaces between: `412:8` and `412: 8`, not `412 8`,
    // which is two numbers and no claim about which is which.
    let mut at = after;
    while bytes.get(at).is_some_and(|c| *c == ' ') {
        at += 1;
    }
    let column = if bytes.get(at) == Some(&':') {
        first_number(&bytes, at + 1)
            .map(|(column, _)| column.max(1))
            .unwrap_or(1)
    } else {
        1
    };
    Some(Target { line, column })
}

/// The first run of digits at or after `from`, and where it ended.
///
/// A digit run that a letter or a path separator runs straight into is
/// part of a name — `utf8.rs` names no line, and neither does the `8`
/// in `main2.rs`. A drive letter's colon (`C:\...`) is skipped for the
/// same reason: what follows it is a path, not a column.
fn first_number(chars: &[char], from: usize) -> Option<(usize, usize)> {
    let mut at = from;
    while at < chars.len() {
        if !chars[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while chars.get(at).is_some_and(char::is_ascii_digit) {
            at += 1;
        }
        let preceded_by_name = start > 0
            && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_');
        let followed_by_name = chars
            .get(at)
            .is_some_and(|c| c.is_alphanumeric() || *c == '_');
        if preceded_by_name || followed_by_name {
            continue;
        }
        let value: usize = chars[start..at]
            .iter()
            .collect::<String>()
            .parse()
            .unwrap_or(usize::MAX);
        return Some((value, at));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: usize, column: usize) -> Option<Target> {
        Some(Target { line, column })
    }

    #[test]
    fn a_bare_number_is_a_line() {
        assert_eq!(parse("412"), at(412, 1));
        assert_eq!(parse("  412  "), at(412, 1));
    }

    #[test]
    fn a_colon_joins_a_column() {
        assert_eq!(parse("412:8"), at(412, 8));
        assert_eq!(parse("412: 8"), at(412, 8));
        // Two numbers with no colon claim nothing about the second.
        assert_eq!(parse("412 8"), at(412, 1));
    }

    #[test]
    fn a_compilers_whole_line_names_the_same_place() {
        assert_eq!(parse("src/main.rs:412:8"), at(412, 8));
        assert_eq!(parse("main.rs:412:8: error: nope"), at(412, 8));
        assert_eq!(parse("  --> core/src/lib.rs:31:5"), at(31, 5));
    }

    #[test]
    fn a_drive_letter_is_not_a_column() {
        assert_eq!(parse(r"C:\src\main.rs:412:8"), at(412, 8));
    }

    #[test]
    fn a_number_inside_a_name_is_part_of_the_name() {
        assert_eq!(parse("utf8.rs:12"), at(12, 1));
        assert_eq!(parse("main2.rs:412:8"), at(412, 8));
        assert_eq!(parse("base64.py"), None);
    }

    #[test]
    fn other_shapes_people_paste() {
        assert_eq!(parse("main.rs, line 412"), at(412, 1));
        assert_eq!(parse("line 412, column 8"), at(412, 1));
        assert_eq!(parse("#412"), at(412, 1));
        assert_eq!(parse(":412"), at(412, 1));
    }

    #[test]
    fn nothing_that_could_be_a_line_is_nothing() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("   "), None);
        assert_eq!(parse("nowhere"), None);
    }

    #[test]
    fn zero_means_the_first_line() {
        assert_eq!(parse("0"), at(1, 1));
        assert_eq!(parse("0:0"), at(1, 1));
        assert_eq!(parse("412:0"), at(412, 1));
    }
}
