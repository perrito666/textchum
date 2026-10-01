//! Inlay hints, gathered at the end of the line they are about.
//!
//! A server's inlay hints are meant to be drawn inside a line: the
//! inferred type after a `let`'s name, a parameter's name before its
//! argument. Neither shell's text view can place text that is not in
//! the document without laying the line out again around it. Text
//! *after* a line needs no such thing — it sits in the empty space past
//! the last character and moves nothing — so that is where the hints of
//! a line go, each type hint written with the name it belongs to:
//! `let d = Drinker::new()` is followed, dimmed, by `d: Drinker`.
//!
//! Parameter-name hints are left out. Their whole value is sitting next
//! to their argument, and a list of names at the end of a line says
//! little about which argument is which.
//!
//! The hints are held as positions in the document and moved with every
//! edit, the way findings are, so they stay on their lines while the
//! server is still to be asked again.

/// One hint the server gave, at a UTF-16 offset of the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub offset: usize,
    pub label: String,
    /// 1 = type, 2 = parameter, 0 = the server did not say.
    pub kind: u8,
}

/// The longest a line's hints are shown at; a type that is a screenful
/// of generics is cut rather than pushed off the window.
const WIDEST: usize = 96;

/// Where a hint's position ends up after `start..end` was replaced by
/// `new_len` units: before the edit it stays, after it moves, and
/// inside it the hint is gone with what it was about.
pub fn shifted(offset: usize, start: usize, end: usize, new_len: usize) -> Option<usize> {
    if offset <= start {
        Some(offset)
    } else if offset >= end {
        // `offset >= end >= end - start`, so this cannot go below zero.
        Some(offset - (end - start) + new_len)
    } else {
        None
    }
}

/// What a hint contributes to the end of its line, given the name
/// written just before it in the text. A type hint — a label that
/// opens with a colon — is joined to that name; anything else stands
/// as the server wrote it. Parameter hints contribute nothing.
pub fn piece(hint: &Hint, name_before: &str) -> Option<String> {
    if hint.kind == 2 {
        return None;
    }
    let label = hint.label.trim();
    if label.is_empty() {
        return None;
    }
    if label.starts_with(':') && !name_before.is_empty() {
        Some(format!("{name_before}{label}"))
    } else {
        Some(label.trim_start_matches(':').trim().to_owned())
    }
}

/// The pieces of one line, as the text shown after it.
pub fn joined(pieces: &[String]) -> String {
    let mut seen: Vec<&String> = Vec::new();
    for piece in pieces {
        if !seen.contains(&piece) {
            seen.push(piece);
        }
    }
    let text = seen.iter().map(|piece| piece.as_str()).collect::<Vec<_>>().join(", ");
    if text.chars().count() > WIDEST {
        let cut: String = text.chars().take(WIDEST - 1).collect();
        format!("{cut}…")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(label: &str, kind: u8) -> Hint {
        Hint { offset: 0, label: label.into(), kind }
    }

    #[test]
    fn a_position_rides_an_edit_before_it_and_goes_with_one_around_it() {
        assert_eq!(shifted(10, 12, 14, 0), Some(10), "an edit after it");
        assert_eq!(shifted(10, 10, 10, 3), Some(10), "typing at it leaves it before the typing");
        assert_eq!(shifted(10, 2, 2, 5), Some(15), "an insertion before it");
        assert_eq!(shifted(10, 2, 6, 0), Some(6), "a deletion before it");
        assert_eq!(shifted(10, 6, 10, 1), Some(7), "a replacement ending at it");
        assert_eq!(shifted(10, 8, 12, 0), None, "an edit that swallows it");
    }

    #[test]
    fn a_type_takes_its_name_and_a_parameter_is_left_out() {
        assert_eq!(piece(&hint(": Drinker", 1), "d").as_deref(), Some("d: Drinker"));
        assert_eq!(piece(&hint(": usize", 0), "").as_deref(), Some("usize"));
        assert_eq!(piece(&hint("impl Iterator<Item = usize>", 1), "x").as_deref(),
                   Some("impl Iterator<Item = usize>"));
        assert_eq!(piece(&hint("left:", 2), "add"), None);
        assert_eq!(piece(&hint("  ", 1), "d"), None);
    }

    #[test]
    fn a_lines_pieces_are_listed_once_and_cut_when_they_run_long() {
        let pieces = vec!["a: i32".to_owned(), "b: u8".to_owned(), "a: i32".to_owned()];
        assert_eq!(joined(&pieces), "a: i32, b: u8");
        let long = vec![format!("x: {}", "Vec<".repeat(40))];
        let shown = joined(&long);
        assert_eq!(shown.chars().count(), WIDEST);
        assert!(shown.ends_with('…'));
    }
}
