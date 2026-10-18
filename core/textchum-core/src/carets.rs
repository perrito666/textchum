//! Several carets: where they go and what an edit does at each.
//!
//! A shell draws the carets and routes keys here; what to select next,
//! where a caret above lands, and which text each keystroke replaces are
//! worked out once, for both. Selections are `(anchor, head)` pairs in
//! UTF-16 units, the head being where the caret is, listed in the order
//! they were made: the last is the newest, which Add Next Occurrence
//! searches on from. An edit comes back as the replacements to make,
//! back to front so none moves another, and the selections after them;
//! the shell makes them through its one edit path, as one undo group.

use ropey::Rope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Selection {
        Selection { anchor: at, head: at }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// A replacement of the UTF-16 range `start..end` by `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Insert(String),
    Backspace,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
}

fn to_char(rope: &Rope, utf16: usize) -> usize {
    rope.utf16_cu_to_char(utf16.min(rope.len_utf16_cu()))
}

fn to_utf16(rope: &Rope, char: usize) -> usize {
    rope.char_to_utf16_cu(char.min(rope.len_chars()))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The selections with duplicates and overlaps merged, in the order the
/// first of each was made. Touching ranges stay apart; two carets at
/// one place become one.
pub fn merged(selections: &[Selection]) -> Vec<Selection> {
    let mut kept: Vec<Selection> = Vec::new();
    for selection in selections {
        let overlapping = kept.iter().position(|other| {
            if selection.is_empty() || other.is_empty() {
                // A caret inside a range, or on another caret.
                (other.start() < selection.start() && selection.start() < other.end())
                    || (selection.start() < other.start() && other.start() < selection.end())
                    || (selection.is_empty() && other.is_empty() && selection.head == other.head)
            } else {
                selection.start() < other.end() && other.start() < selection.end()
            }
        });
        match overlapping {
            Some(at) => {
                let other = kept[at];
                let (start, end) = (other.start().min(selection.start()), other.end().max(selection.end()));
                kept[at] = if other.head < other.anchor {
                    Selection { anchor: end, head: start }
                } else {
                    Selection { anchor: start, head: end }
                };
            }
            None => kept.push(*selection),
        }
    }
    kept
}

/// The word around UTF-16 `at`, as a UTF-16 range; None off any word.
pub fn word_at(rope: &Rope, at: usize) -> Option<(usize, usize)> {
    let at = to_char(rope, at);
    let len = rope.len_chars();
    let mut start = at;
    while start > 0 && is_word(rope.char(start - 1)) {
        start -= 1;
    }
    let mut end = at;
    while end < len && is_word(rope.char(end)) {
        end += 1;
    }
    (start < end).then(|| (to_utf16(rope, start), to_utf16(rope, end)))
}

/// Add Next Occurrence. With the newest selection empty, the word around
/// it is selected; otherwise the next match of the newest selection's
/// text after it, round the end, not already selected, is added. A
/// selection that is a whole word matches whole words only. None when
/// there is nothing to add.
pub fn add_next_occurrence(rope: &Rope, selections: &[Selection]) -> Option<Vec<Selection>> {
    let newest = *selections.last()?;
    if newest.is_empty() {
        let (start, end) = word_at(rope, newest.head)?;
        let mut grown = selections.to_vec();
        *grown.last_mut()? = Selection { anchor: start, head: end };
        return Some(merged(&grown));
    }
    let (start, end) = (to_char(rope, newest.start()), to_char(rope, newest.end()));
    let needle: Vec<char> = rope.slice(start..end).chars().collect();
    let text: Vec<char> = rope.chars().collect();
    let bounded = |from: usize, to: usize| {
        (from == 0 || !is_word(text[from - 1])) && (to == text.len() || !is_word(text[to]))
    };
    let whole_word = needle.iter().all(|c| is_word(*c)) && bounded(start, end);
    let taken: Vec<(usize, usize)> =
        selections.iter().map(|selection| (to_char(rope, selection.start()), to_char(rope, selection.end()))).collect();
    let last_start = text.len().saturating_sub(needle.len());
    let candidates = (end..=last_start).chain(0..end.min(last_start + 1));
    for from in candidates {
        let to = from + needle.len();
        if text[from..to] != needle[..] || (whole_word && !bounded(from, to)) {
            continue;
        }
        if taken.iter().any(|(taken_start, taken_end)| from < *taken_end && *taken_start < to) {
            continue;
        }
        let mut grown = selections.to_vec();
        grown.push(Selection { anchor: to_utf16(rope, from), head: to_utf16(rope, to) });
        return Some(grown);
    }
    None
}

/// The zero-based line and character column of UTF-16 `at`.
fn line_column(rope: &Rope, at: usize) -> (usize, usize) {
    let at = to_char(rope, at);
    let line = rope.char_to_line(at);
    (line, at - rope.line_to_char(line))
}

/// The UTF-16 offset of `column` on `line`, kept on the line.
fn at_column(rope: &Rope, line: usize, column: usize) -> usize {
    let start = rope.line_to_char(line);
    let slice = rope.line(line);
    let mut length = slice.len_chars();
    // The line's own break is not a column.
    while length > 0 && matches!(slice.char(length - 1), '\n' | '\r') {
        length -= 1;
    }
    to_utf16(rope, start + column.min(length))
}

/// Add Caret Above (`up`) or Below: a caret on the line past the
/// highest (or lowest) one, in the first caret's column. None at the
/// first or last line.
pub fn add_caret_vertically(rope: &Rope, selections: &[Selection], up: bool) -> Option<Vec<Selection>> {
    let first = selections.first()?;
    let (_, column) = line_column(rope, first.head);
    let lines = selections.iter().map(|selection| line_column(rope, selection.head).0);
    let edge = if up { lines.min()? } else { lines.max()? };
    let line = if up { edge.checked_sub(1)? } else { edge + 1 };
    // A file ending in a line break has an empty last line ropey counts.
    if line >= rope.len_lines() {
        return None;
    }
    let mut grown = selections.to_vec();
    grown.push(Selection::caret(at_column(rope, line, column)));
    Some(merged(&grown))
}

/// What `op` replaces at each selection, back to front, and the carets
/// after: after the text typed, or where the deleted text was.
pub fn edit(rope: &Rope, selections: &[Selection], op: &Op) -> (Vec<Edit>, Vec<Selection>) {
    let selections = merged(selections);
    let total = rope.len_chars();
    let mut ranges: Vec<(usize, usize, usize)> = selections
        .iter()
        .enumerate()
        .map(|(index, selection)| {
            let (start, end) = (to_char(rope, selection.start()), to_char(rope, selection.end()));
            let (start, end) = match op {
                _ if start < end => (start, end),
                Op::Insert(_) => (start, end),
                Op::Backspace => (start.saturating_sub(1), end),
                Op::Delete => (start, (end + 1).min(total)),
            };
            (start, end, index)
        })
        .collect();
    ranges.sort_by_key(|(start, _, _)| *start);
    let inserted: Vec<char> = match op {
        Op::Insert(text) => text.chars().collect(),
        _ => Vec::new(),
    };
    let text: String = inserted.iter().collect();
    // Where each caret lands, in characters of the text after every
    // edit: the earlier edits have moved it by what they added.
    let mut carets = vec![0usize; selections.len()];
    let mut shift: isize = 0;
    let mut previous_end = 0;
    let mut edits = Vec::new();
    for (start, end, index) in ranges {
        // Two carets side by side deleting towards each other reach the
        // same character; it goes once.
        let start = start.max(previous_end);
        let end = end.max(start);
        carets[index] = (start as isize + shift) as usize + inserted.len();
        if start < end || !inserted.is_empty() {
            edits.push(Edit { start: to_utf16(rope, start), end: to_utf16(rope, end), text: text.clone() });
        }
        shift += inserted.len() as isize - (end - start) as isize;
        previous_end = end;
    }
    edits.reverse();
    // The carets in UTF-16 of the text after the edits.
    let mut after = rope.clone();
    for edit in &edits {
        let (start, end) = (to_char(&after, edit.start), to_char(&after, edit.end));
        after.remove(start..end);
        after.insert(start, &edit.text);
    }
    let carets: Vec<Selection> = carets.into_iter().map(|at| Selection::caret(to_utf16(&after, at))).collect();
    (edits, merged(&carets))
}

/// Every selection moved by `motion`, or with `extend` grown to where
/// its head moves. A range collapsed by Left or Right goes to its start
/// or end, as with one caret.
pub fn moved(rope: &Rope, selections: &[Selection], motion: Motion, extend: bool) -> Vec<Selection> {
    let total = rope.len_chars();
    let moved: Vec<Selection> = selections
        .iter()
        .map(|selection| {
            if !extend && !selection.is_empty() {
                match motion {
                    Motion::Left => return Selection::caret(selection.start()),
                    Motion::Right => return Selection::caret(selection.end()),
                    _ => {}
                }
            }
            let head = to_char(rope, selection.head);
            let (line, column) = line_column(rope, selection.head);
            let to = match motion {
                Motion::Left => to_utf16(rope, head.saturating_sub(1)),
                Motion::Right => to_utf16(rope, (head + 1).min(total)),
                Motion::Up if line == 0 => 0,
                Motion::Up => at_column(rope, line - 1, column),
                Motion::Down if line + 1 >= rope.len_lines() => to_utf16(rope, total),
                Motion::Down => at_column(rope, line + 1, column),
                Motion::LineStart => at_column(rope, line, 0),
                Motion::LineEnd => at_column(rope, line, usize::MAX),
            };
            if extend { Selection { anchor: selection.anchor, head: to } } else { Selection::caret(to) }
        })
        .collect();
    merged(&moved)
}

/// The JSON a shell sends: `{selections: [[anchor, head]], op}`, op one
/// of `next`, `above`, `below`, `insert` (with `text`), `backspace`,
/// `delete`, `left`, `right`, `up`, `down`, `home`, `end` (with
/// `extend`). The answer is `{selections, edits: [{start, end, text}]}`,
/// edits back to front; `null` when the command has nothing to do.
pub fn request_json(rope: &Rope, request: &str) -> String {
    let Ok(request) = serde_json::from_str::<serde_json::Value>(request) else { return "null".to_owned() };
    let selections: Vec<Selection> = request["selections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pair| Some(Selection { anchor: pair[0].as_u64()? as usize, head: pair[1].as_u64()? as usize }))
        .collect();
    if selections.is_empty() {
        return "null".to_owned();
    }
    let extend = request["extend"].as_bool().unwrap_or(false);
    let (selections, edits) = match request["op"].as_str().unwrap_or_default() {
        "next" => match add_next_occurrence(rope, &selections) {
            Some(grown) => (grown, Vec::new()),
            None => return "null".to_owned(),
        },
        "above" | "below" => match add_caret_vertically(rope, &selections, request["op"] == "above") {
            Some(grown) => (grown, Vec::new()),
            None => return "null".to_owned(),
        },
        "insert" => {
            let (edits, after) = edit(rope, &selections, &Op::Insert(request["text"].as_str().unwrap_or_default().to_owned()));
            (after, edits)
        }
        "backspace" => {
            let (edits, after) = edit(rope, &selections, &Op::Backspace);
            (after, edits)
        }
        "delete" => {
            let (edits, after) = edit(rope, &selections, &Op::Delete);
            (after, edits)
        }
        motion => {
            let motion = match motion {
                "left" => Motion::Left,
                "right" => Motion::Right,
                "up" => Motion::Up,
                "down" => Motion::Down,
                "home" => Motion::LineStart,
                "end" => Motion::LineEnd,
                _ => return "null".to_owned(),
            };
            (moved(rope, &selections, motion, extend), Vec::new())
        }
    };
    serde_json::json!({
        "selections": selections.iter().map(|selection| [selection.anchor, selection.head]).collect::<Vec<_>>(),
        "edits": edits
            .iter()
            .map(|edit| serde_json::json!({"start": edit.start, "end": edit.end, "text": edit.text}))
            .collect::<Vec<_>>(),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(selections: &[Selection]) -> Vec<(usize, usize)> {
        selections.iter().map(|selection| (selection.anchor, selection.head)).collect()
    }

    #[test]
    fn next_occurrence_takes_the_word_then_its_whole_word_matches() {
        let rope = Rope::from_str("let item = items[0];\nitem.go(item);\n");
        let word = add_next_occurrence(&rope, &[Selection::caret(5)]).unwrap();
        assert_eq!(pairs(&word), [(4, 8)]);
        let two = add_next_occurrence(&rope, &word).unwrap();
        assert_eq!(pairs(&two), [(4, 8), (21, 25)], "`items` is not a match for the word `item`");
        let three = add_next_occurrence(&rope, &two).unwrap();
        assert_eq!(pairs(&three)[2], (29, 33));
        assert_eq!(add_next_occurrence(&rope, &three), None, "every one is taken");
        // A selection that is not a whole word matches inside words.
        let part = add_next_occurrence(&rope, &[Selection { anchor: 4, head: 7 }]).unwrap();
        assert_eq!(pairs(&part)[1], (11, 14));
    }

    #[test]
    fn carets_are_added_above_and_below_in_the_first_ones_column() {
        let rope = Rope::from_str("abcdef\nab\nabcdef\n");
        let below = add_caret_vertically(&rope, &[Selection::caret(4)], false).unwrap();
        assert_eq!(pairs(&below), [(4, 4), (9, 9)], "kept on the short line");
        let further = add_caret_vertically(&rope, &below, false).unwrap();
        assert_eq!(pairs(&further)[2], (14, 14), "the first caret's column again");
        assert_eq!(add_caret_vertically(&rope, &[Selection::caret(2)], true), None);
    }

    #[test]
    fn typing_and_deleting_at_every_caret() {
        let rope = Rope::from_str("a1\nb2\nc3\n");
        let carets = [Selection::caret(1), Selection::caret(4), Selection::caret(7)];
        let (edits, after) = edit(&rope, &carets, &Op::Insert("é".into()));
        assert_eq!(edits[0], Edit { start: 7, end: 7, text: "é".into() }, "back to front");
        assert_eq!(pairs(&after), [(2, 2), (6, 6), (10, 10)]);
        let (edits, after) = edit(&rope, &carets, &Op::Backspace);
        assert_eq!(edits.len(), 3);
        assert_eq!(pairs(&after), [(0, 0), (2, 2), (4, 4)]);
        // A selection is replaced whole; carets side by side delete
        // towards each other once.
        let (edits, after) = edit(&rope, &[Selection { anchor: 0, head: 2 }, Selection::caret(4)], &Op::Insert("x".into()));
        assert_eq!(edits.last().unwrap(), &Edit { start: 0, end: 2, text: "x".into() });
        assert_eq!(pairs(&after), [(1, 1), (4, 4)]);
        let (edits, _) = edit(&Rope::from_str("abc"), &[Selection::caret(1), Selection::caret(2)], &Op::Delete);
        assert_eq!(edits, [Edit { start: 2, end: 3, text: String::new() }, Edit { start: 1, end: 2, text: String::new() }]);
    }

    #[test]
    fn moving_every_caret_and_merging_those_that_meet() {
        let rope = Rope::from_str("one\ntwo\n");
        let carets = [Selection::caret(1), Selection::caret(5)];
        assert_eq!(pairs(&moved(&rope, &carets, Motion::LineStart, false)), [(0, 0), (4, 4)]);
        assert_eq!(pairs(&moved(&rope, &carets, Motion::Up, false)), [(0, 0), (1, 1)]);
        assert_eq!(pairs(&moved(&rope, &carets, Motion::Right, true)), [(1, 2), (5, 6)]);
        let met = moved(&rope, &[Selection::caret(1), Selection::caret(2)], Motion::LineStart, false);
        assert_eq!(pairs(&met), [(0, 0)]);
    }

    #[test]
    fn a_request_answers_in_json() {
        let rope = Rope::from_str("ab ab");
        let answer: serde_json::Value =
            serde_json::from_str(&request_json(&rope, r#"{"selections": [[0, 2]], "op": "next"}"#)).unwrap();
        assert_eq!(answer["selections"], serde_json::json!([[0, 2], [3, 5]]));
        let typed: serde_json::Value = serde_json::from_str(&request_json(
            &rope,
            r#"{"selections": [[0, 2], [3, 5]], "op": "insert", "text": "x"}"#,
        ))
        .unwrap();
        assert_eq!(typed["edits"], serde_json::json!([{"start": 3, "end": 5, "text": "x"}, {"start": 0, "end": 2, "text": "x"}]));
        assert_eq!(typed["selections"], serde_json::json!([[1, 1], [3, 3]]));
        assert_eq!(request_json(&rope, r#"{"selections": [[0, 0]], "op": "above"}"#), "null");
    }
}
