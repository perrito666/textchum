//! Brackets as a structure: which one closes which, and how deep each
//! sits. What the editor shows as a matching pair when the caret is on
//! one, and as colours by depth when asked to.
//!
//! The scan is over the whole text with the strings and comments taken
//! out first — a `(` inside a string opens nothing — which is why the
//! document, which knows where those are, owns the cache of it.
//! Offsets are UTF-16 code units, the shells' native currency.

/// One bracket in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bracket {
    /// Where it is.
    pub offset: usize,
    /// True for `(`, `[` and `{`.
    pub open: bool,
    /// How many pairs enclose it, its own not counted: the outermost
    /// pair is depth 0. Meaningless when there is no partner.
    pub depth: u32,
    /// The offset of the bracket that closes (or opens) it, when the
    /// text has one.
    pub partner: Option<usize>,
}

fn opens(character: char) -> Option<char> {
    match character {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn closes(character: char) -> bool {
    matches!(character, ')' | ']' | '}')
}

/// Every bracket in `text`, in order, with `excluded` — the UTF-16
/// ranges of strings, comments and characters — left out of the
/// account entirely. `excluded` must be sorted.
///
/// A closer whose kind does not match the innermost open pair looks
/// for its opener further out: `([)]` pairs the parentheses, and the
/// `[` is left without a partner rather than closed by a `)`. A closer
/// with no opener at all, and an opener never closed, have no partner.
pub fn scan(text: &str, excluded: &[(usize, usize)]) -> Vec<Bracket> {
    let mut brackets: Vec<Bracket> = Vec::new();
    // Indices into `brackets` of the openers not yet closed.
    let mut open: Vec<usize> = Vec::new();
    let mut skip = excluded.iter().peekable();
    let mut offset = 0usize;
    for character in text.chars() {
        let here = offset;
        offset += character.len_utf16();
        while skip.peek().is_some_and(|(_, end)| *end <= here) {
            skip.next();
        }
        if skip.peek().is_some_and(|(start, end)| *start <= here && here < *end) {
            continue;
        }
        if let Some(closer) = opens(character) {
            let _ = closer;
            open.push(brackets.len());
            brackets.push(Bracket { offset: here, open: true, depth: 0, partner: None });
        } else if closes(character) {
            let wanted = match character {
                ')' => '(',
                ']' => '[',
                _ => '{',
            };
            let found = open.iter().rposition(|&index| {
                text_char(text, brackets[index].offset) == Some(wanted)
            });
            let Some(position) = found else {
                brackets.push(Bracket { offset: here, open: false, depth: 0, partner: None });
                continue;
            };
            // Openers inside the matched one that never closed are
            // abandoned along with it.
            let opener = open[position];
            open.truncate(position);
            let depth = position as u32;
            brackets[opener].depth = depth;
            brackets[opener].partner = Some(here);
            brackets.push(Bracket {
                offset: here,
                open: false,
                depth,
                partner: Some(brackets[opener].offset),
            });
        }
    }
    brackets
}

/// The character at a UTF-16 offset, for a bracket the scan already
/// placed there.
fn text_char(text: &str, offset: usize) -> Option<char> {
    let mut at = 0usize;
    for character in text.chars() {
        if at == offset {
            return Some(character);
        }
        at += character.len_utf16();
        if at > offset {
            return None;
        }
    }
    None
}

/// The bracket the caret is on and its partner: the one just before
/// the caret first — the one just typed, or just passed — else the one
/// at it. None when neither is a bracket, or the one there has no
/// partner. `brackets` is what [`scan`] answered.
pub fn matching(brackets: &[Bracket], caret: usize) -> Option<(usize, usize)> {
    let at = |offset: usize| {
        brackets
            .binary_search_by_key(&offset, |bracket| bracket.offset)
            .ok()
            .map(|index| brackets[index])
    };
    let candidate = caret
        .checked_sub(1)
        .and_then(at)
        .or_else(|| at(caret))?;
    Some((candidate.offset, candidate.partner?))
}

/// The brackets that sit within `start..end`, for painting.
pub fn within(brackets: &[Bracket], start: usize, end: usize) -> &[Bracket] {
    let from = brackets.partition_point(|bracket| bracket.offset < start);
    let to = brackets.partition_point(|bracket| bracket.offset < end);
    &brackets[from..to]
}

/// The colours pairs are painted with by depth, 0xRRGGBBAA, cycling
/// past the last; one set on light, one on dark. Hues far enough apart
/// to be told apart at a glance, none the colours the default theme
/// gives keywords and strings.
pub const RAINBOW_LIGHT: [u32; 6] =
    [0xB58900FF, 0xD33682FF, 0x268BD2FF, 0x2AA198FF, 0x6C71C4FF, 0xCB4B16FF];
pub const RAINBOW_DARK: [u32; 6] =
    [0xFFD866FF, 0xFF6188FF, 0x78DCE8FF, 0xA9DC76FF, 0xAB9DF2FF, 0xFC9867FF];

/// The colour for a depth.
pub fn rainbow(depth: u32, dark: bool) -> u32 {
    let table = if dark { &RAINBOW_DARK } else { &RAINBOW_LIGHT };
    table[depth as usize % table.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(text: &str) -> Vec<(usize, Option<usize>, u32)> {
        scan(text, &[])
            .into_iter()
            .map(|bracket| (bracket.offset, bracket.partner, bracket.depth))
            .collect()
    }

    #[test]
    fn pairs_nest_and_count_their_depth() {
        assert_eq!(
            pairs("f(a[b]{c})"),
            vec![
                (1, Some(9), 0),
                (3, Some(5), 1),
                (5, Some(3), 1),
                (6, Some(8), 1),
                (8, Some(6), 1),
                (9, Some(1), 0),
            ]
        );
    }

    #[test]
    fn a_closer_of_the_wrong_kind_looks_further_out() {
        // The parentheses pair; the bracket in between is abandoned.
        assert_eq!(
            pairs("([)]"),
            vec![(0, Some(2), 0), (1, None, 0), (2, Some(0), 0), (3, None, 0)]
        );
        assert_eq!(pairs(")("), vec![(0, None, 0), (1, None, 0)]);
    }

    #[test]
    fn brackets_in_strings_open_nothing() {
        let text = r#"f(")")"#;
        // The `)` inside the string is at 3; without the exclusion it
        // would close the call and leave the real closer orphaned.
        assert_eq!(pairs(text), vec![(1, Some(3), 0), (3, Some(1), 0), (5, None, 0)]);
        assert_eq!(
            scan(text, &[(2, 5)])
                .into_iter()
                .map(|b| (b.offset, b.partner))
                .collect::<Vec<_>>(),
            vec![(1, Some(5)), (5, Some(1))]
        );
    }

    #[test]
    fn the_caret_finds_the_bracket_before_it_first() {
        let brackets = scan("(a)(b)", &[]);
        // After the first `)`: that one, not the `(` at the caret.
        assert_eq!(matching(&brackets, 3), Some((2, 0)));
        assert_eq!(matching(&brackets, 0), Some((0, 2)));
        assert_eq!(matching(&brackets, 1), Some((0, 2)));
        assert_eq!(matching(&brackets, 2), Some((2, 0)));
        assert_eq!(matching(&scan("a", &[]), 1), None);
        assert_eq!(matching(&scan("(", &[]), 1), None, "no partner, nothing to show");
    }

    #[test]
    fn offsets_are_utf16() {
        // 😀 is two units; the brackets follow it.
        assert_eq!(pairs("😀()"), vec![(2, Some(3), 0), (3, Some(2), 0)]);
        let brackets = scan("x(😀)", &[]);
        assert_eq!(within(&brackets, 2, 10).len(), 1);
        assert_eq!(within(&brackets, 0, 10).len(), 2);
    }

    #[test]
    fn the_rainbow_cycles() {
        assert_eq!(rainbow(0, false), RAINBOW_LIGHT[0]);
        assert_eq!(rainbow(6, true), RAINBOW_DARK[0]);
        assert_eq!(rainbow(7, false), RAINBOW_LIGHT[1]);
    }
}
