//! Moving through code by the shapes code has.
//!
//! The text system's idea of a word lumps a run of punctuation together
//! and reads across line breaks; an editor stops at every change of
//! character class — identifier characters, symbols, whitespace — and
//! at the end of a line. And a closing bracket typed first on a line
//! wants the indentation of the line that opened it.
//!
//! Offsets are UTF-16 code units, the shells' native currency.

/// The classes a boundary sits between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Symbol,
    Space,
    Newline,
}

fn class(character: char) -> Class {
    if character == '\n' || character == '\r' {
        Class::Newline
    } else if character.is_whitespace() {
        Class::Space
    } else if character.is_alphanumeric() || character == '_' {
        Class::Word
    } else {
        Class::Symbol
    }
}

/// The UTF-16 offset a word move lands on from `offset`: forward,
/// past any blanks and then one run of like characters; backward, the
/// mirror. A line break is a stop of its own, so a move never crosses
/// a line without landing on its edge first.
pub fn word_boundary(text: &str, offset: usize, forward: bool) -> usize {
    let units: Vec<(usize, char)> = utf16_indexed(text);
    let total = units.last().map_or(0, |(at, c)| at + c.len_utf16());
    let offset = offset.min(total);
    // The index of the first unit at or after `offset`.
    let index_at = |target: usize| units.partition_point(|(at, _)| *at < target);
    if forward {
        let mut index = index_at(offset);
        // Blanks first — but a line break is where a move stops.
        while index < units.len() && class(units[index].1) == Class::Space {
            index += 1;
        }
        if index < units.len() && class(units[index].1) == Class::Newline {
            // Land after the break: the start of the next line.
            let start = units[index].1.len_utf16();
            let mut end = index + 1;
            if units[index].1 == '\r' && end < units.len() && units[end].1 == '\n' {
                end += 1;
            }
            let _ = start;
            return units.get(end).map_or(total, |(at, _)| *at);
        }
        let Some(&(_, first)) = units.get(index) else { return total };
        let run = class(first);
        while index < units.len() && class(units[index].1) == run {
            index += 1;
        }
        units.get(index).map_or(total, |(at, _)| *at)
    } else {
        let mut index = index_at(offset);
        while index > 0 && class(units[index - 1].1) == Class::Space {
            index -= 1;
        }
        if index > 0 && class(units[index - 1].1) == Class::Newline {
            // Land before the break: the end of the previous line.
            let mut start = index - 1;
            if units[start].1 == '\n' && start > 0 && units[start - 1].1 == '\r' {
                start -= 1;
            }
            return units[start].0;
        }
        if index == 0 {
            return 0;
        }
        let run = class(units[index - 1].1);
        while index > 0 && class(units[index - 1].1) == run {
            index -= 1;
        }
        units[index].0
    }
}

/// What a closing bracket typed at `offset` asks of its line: when
/// everything before it on the line is blank and the matching opener
/// is found, the leading blanks' UTF-16 range and the indentation the
/// opener's line has, to put in their place. `None` when the line
/// already carries text, or no opener matches.
pub fn closing_bracket_indent(text: &str, offset: usize, closer: char) -> Option<(usize, usize, String)> {
    let opener = match closer {
        ')' => '(',
        ']' => '[',
        '}' => '{',
        _ => return None,
    };
    let units = utf16_indexed(text);
    let index = units.partition_point(|(at, _)| *at < offset);
    // The line the closer goes on, and whether it is blank before it.
    let mut line_start = index;
    while line_start > 0 && class(units[line_start - 1].1) != Class::Newline {
        line_start -= 1;
    }
    if units[line_start..index].iter().any(|(_, c)| !c.is_whitespace()) {
        return None;
    }
    // Backwards to the opener, counting nesting of the same pair.
    let mut depth = 0usize;
    let mut cursor = line_start;
    let mut opener_index = None;
    while cursor > 0 {
        cursor -= 1;
        let c = units[cursor].1;
        if c == closer {
            depth += 1;
        } else if c == opener {
            if depth == 0 {
                opener_index = Some(cursor);
                break;
            }
            depth -= 1;
        }
    }
    let opener_index = opener_index?;
    let mut opener_line = opener_index;
    while opener_line > 0 && class(units[opener_line - 1].1) != Class::Newline {
        opener_line -= 1;
    }
    let indent: String = units[opener_line..]
        .iter()
        .map(|(_, c)| *c)
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let blanks_start = units[line_start].0;
    let blanks_end = offset;
    Some((blanks_start, blanks_end, indent))
}

/// Every character with the UTF-16 offset it starts at.
fn utf16_indexed(text: &str) -> Vec<(usize, char)> {
    let mut at = 0;
    text.chars()
        .map(|c| {
            let here = at;
            at += c.len_utf16();
            (here, c)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_of_symbols_is_its_own_word() {
        let text = r#"key")} next"#;
        // From the start: over `key`, then over `")}`, then over ` next`.
        assert_eq!(word_boundary(text, 0, true), 3);
        assert_eq!(word_boundary(text, 3, true), 6);
        assert_eq!(word_boundary(text, 6, true), 11);
        // And back.
        assert_eq!(word_boundary(text, 11, false), 7);
        assert_eq!(word_boundary(text, 7, false), 3);
        assert_eq!(word_boundary(text, 3, false), 0);
    }

    #[test]
    fn a_line_break_is_a_stop_of_its_own() {
        let text = "a\n\n  b";
        assert_eq!(word_boundary(text, 0, true), 1);
        assert_eq!(word_boundary(text, 1, true), 2);
        assert_eq!(word_boundary(text, 2, true), 3);
        assert_eq!(word_boundary(text, 3, true), 6);
        // Back: over `b`, then the blanks up to the break — landing at
        // the end of the empty line — then that line's own break.
        assert_eq!(word_boundary(text, 6, false), 5);
        assert_eq!(word_boundary(text, 5, false), 2);
        assert_eq!(word_boundary(text, 2, false), 1);
        assert_eq!(word_boundary(text, 1, false), 0);
    }

    #[test]
    fn the_ends_hold() {
        assert_eq!(word_boundary("", 0, true), 0);
        assert_eq!(word_boundary("abc", 3, true), 3);
        assert_eq!(word_boundary("abc", 0, false), 0);
        assert_eq!(word_boundary("abc", 99, false), 0);
    }

    #[test]
    fn a_closer_first_on_its_line_takes_the_openers_indentation() {
        let text = "fn f() {\n    if x {\n        y();\n        ";
        let offset = text.encode_utf16().count();
        let (start, end, indent) = closing_bracket_indent(text, offset, '}').unwrap();
        assert_eq!(indent, "    ");
        assert_eq!(&text[start..end], "        ");
    }

    #[test]
    fn a_closer_after_text_or_without_an_opener_is_left_alone() {
        let text = "let a = [1, 2";
        assert!(closing_bracket_indent(text, text.len(), ']').is_none());
        let text = "x\n    ";
        assert!(closing_bracket_indent(text, text.len(), ')').is_none());
        // Nesting counts: the inner pair is skipped.
        let text = "{\n  {\n  }\n  ";
        let (_, _, indent) = closing_bracket_indent(text, text.len(), '}').unwrap();
        assert_eq!(indent, "");
    }
}
