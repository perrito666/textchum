//! Several carets at once.
//!
//! One caret is the special case of a list with one entry, so the list
//! is what everything works over. Adding it late would mean teaching
//! every command that reads "the caret" to read "each caret", which is
//! why the model lives here rather than in a shell.
//!
//! Neither toolkit helps. AppKit keeps several selected ranges but
//! `insertText:` edits only the primary and drops the rest, and it
//! collapses several empty ranges into one — so an extra caret cannot
//! even be expressed as an empty selection. GtkTextView has a single
//! insertion mark. Both shells therefore hold the list, draw the extra
//! carets, and apply the edits themselves; this module says what those
//! edits are.
//!
//! Offsets are UTF-16 code units, the currency the rest of the core
//! speaks to the shells in.

/// One caret, with its selection: `anchor` is where the selection
/// started and `head` is where the caret is. They are equal for a caret
/// with nothing selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub anchor: usize,
    pub head: usize,
}

impl Caret {
    pub fn at(offset: usize) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// The selection, low end first.
    pub fn range(self) -> (usize, usize) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

/// One edit to apply: replace `[start, end)` with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// The carets, in document order, none overlapping another, one of them
/// primary.
///
/// The primary is the one commands that can only ask about one place
/// use — go to definition, hover, the language server in general. A
/// question about a symbol has one answer, and asking it five times is
/// five answers nobody wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carets {
    carets: Vec<Caret>,
    primary: usize,
}

impl Carets {
    /// One caret, which is where every document starts.
    pub fn one(caret: Caret) -> Self {
        Self {
            carets: vec![caret],
            primary: 0,
        }
    }

    /// Adds a caret, keeping the list ordered and non-overlapping.
    ///
    /// A caret that lands inside an existing one merges into it rather
    /// than becoming a second caret at the same place: two carets in
    /// one spot type every character twice.
    pub fn add(&mut self, caret: Caret) {
        let primary_before = self.carets[self.primary];
        self.carets.push(caret);
        self.normalize(primary_before);
    }

    /// Back to one caret — the primary. Escape does this.
    pub fn collapse(&mut self) {
        let primary = self.carets[self.primary];
        self.carets = vec![primary];
        self.primary = 0;
    }

    pub fn all(&self) -> &[Caret] {
        &self.carets
    }

    pub fn primary(&self) -> Caret {
        self.carets[self.primary]
    }

    pub fn len(&self) -> usize {
        self.carets.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Whether there is more than one, which is what the shells check
    /// before doing any of the extra work.
    pub fn is_multiple(&self) -> bool {
        self.carets.len() > 1
    }

    /// Sorts, merges overlaps, and keeps the primary pointing at the
    /// caret it pointed at — or the one that swallowed it.
    fn normalize(&mut self, primary: Caret) {
        self.carets.sort_by_key(|caret| caret.range());
        let mut merged: Vec<Caret> = Vec::with_capacity(self.carets.len());
        for caret in self.carets.drain(..) {
            let (start, end) = caret.range();
            match merged.last_mut() {
                // Touching counts as overlapping: two carets at the same
                // offset are one caret.
                Some(last) if last.range().1 >= start => {
                    let (last_start, last_end) = last.range();
                    let forward = last.head >= last.anchor;
                    let (low, high) = (last_start.min(start), last_end.max(end));
                    *last = if forward {
                        Caret::new(low, high)
                    } else {
                        Caret::new(high, low)
                    };
                }
                _ => merged.push(caret),
            }
        }
        self.carets = merged;
        let (primary_start, primary_end) = primary.range();
        self.primary = self
            .carets
            .iter()
            .position(|caret| {
                let (start, end) = caret.range();
                start <= primary_start && end >= primary_end
            })
            .unwrap_or(0);
    }
}

/// The edits typing `text` at every caret amounts to.
///
/// Back to front, so applying them in order leaves the offsets of the
/// ones not yet applied untouched.
pub fn insert(carets: &Carets, text: &str) -> Vec<Edit> {
    let mut edits: Vec<Edit> = carets
        .all()
        .iter()
        .map(|caret| {
            let (start, end) = caret.range();
            Edit {
                start,
                end,
                text: text.to_string(),
            }
        })
        .collect();
    edits.reverse();
    edits
}

/// The edits backspace amounts to.
///
/// A caret with a selection deletes it; one without deletes the
/// character before it — and nothing at all at the start of the
/// document.
pub fn delete_backward(carets: &Carets) -> Vec<Edit> {
    let mut edits: Vec<Edit> = carets
        .all()
        .iter()
        .filter_map(|caret| {
            let (start, end) = caret.range();
            if start != end {
                return Some(Edit {
                    start,
                    end,
                    text: String::new(),
                });
            }
            (start > 0).then(|| Edit {
                start: start - 1,
                end: start,
                text: String::new(),
            })
        })
        .collect();
    edits.reverse();
    edits
}

/// The edits forward delete amounts to.
pub fn delete_forward(carets: &Carets, length: usize) -> Vec<Edit> {
    let mut edits: Vec<Edit> = carets
        .all()
        .iter()
        .filter_map(|caret| {
            let (start, end) = caret.range();
            if start != end {
                return Some(Edit {
                    start,
                    end,
                    text: String::new(),
                });
            }
            (end < length).then(|| Edit {
                start: end,
                end: end + 1,
                text: String::new(),
            })
        })
        .collect();
    edits.reverse();
    edits
}

/// Where the carets end up once `edits` have been applied.
///
/// Each caret lands at the end of what replaced it, and every caret
/// after it shifts by what the edits before it added or took away.
pub fn after(edits: &[Edit]) -> Carets {
    // The edits arrive back to front; walking them forwards is what
    // accumulates the shift.
    let mut forward: Vec<&Edit> = edits.iter().collect();
    forward.reverse();
    let mut shift: isize = 0;
    let mut carets: Vec<Caret> = Vec::with_capacity(forward.len());
    for edit in forward {
        let length = edit.text.encode_utf16().count();
        let at = (edit.start as isize + shift) as usize + length;
        carets.push(Caret::at(at));
        shift += length as isize - (edit.end - edit.start) as isize;
    }
    if carets.is_empty() {
        return Carets::one(Caret::at(0));
    }
    let primary = carets.len() - 1;
    Carets {
        carets,
        primary,
    }
}

/// A caret one line above the topmost one, at the same column, if there
/// is a line above.
pub fn above(text: &str, carets: &Carets) -> Option<Caret> {
    let topmost = carets.all().first()?.range().0;
    let (line, column) = line_and_column(text, topmost);
    if line == 0 {
        return None;
    }
    Some(Caret::at(offset_of(text, line - 1, column)))
}

/// A caret one line below the bottommost one, at the same column.
pub fn below(text: &str, carets: &Carets) -> Option<Caret> {
    let bottom = carets.all().last()?.range().1;
    let (line, column) = line_and_column(text, bottom);
    let lines = text.split('\n').count();
    if line + 1 >= lines {
        return None;
    }
    Some(Caret::at(offset_of(text, line + 1, column)))
}

/// The next occurrence of what the primary caret has selected, wrapping
/// to the start, skipping the places already carrying a caret.
///
/// `None` when nothing is selected — there is no word to look for — or
/// when every occurrence already has one.
pub fn next_occurrence(text: &str, carets: &Carets) -> Option<Caret> {
    let (start, end) = carets.primary().range();
    if start == end {
        return None;
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let needle = &units.get(start..end)?;
    if needle.is_empty() || needle.len() > units.len() {
        return None;
    }
    let taken: Vec<(usize, usize)> = carets.all().iter().map(|caret| caret.range()).collect();
    // From just after the primary, round to it again.
    let total = units.len() + 1 - needle.len();
    for step in 1..=total {
        let at = (start + step) % total;
        if units[at..at + needle.len()] != **needle {
            continue;
        }
        let found = (at, at + needle.len());
        if taken.contains(&found) {
            continue;
        }
        return Some(Caret::new(found.0, found.1));
    }
    None
}

/// The line a UTF-16 offset is on, and how far into it, in UTF-16
/// units.
fn line_and_column(text: &str, offset: usize) -> (usize, usize) {
    let mut line = 0;
    let mut line_start = 0;
    let mut at = 0;
    for character in text.chars() {
        if at >= offset {
            break;
        }
        at += character.len_utf16();
        if character == '\n' {
            line += 1;
            line_start = at;
        }
    }
    (line, offset.saturating_sub(line_start))
}

/// The offset of a column on a line, clamped to the line's end — a
/// short line does not grow to meet a caret coming down from a long
/// one.
fn offset_of(text: &str, line: usize, column: usize) -> usize {
    let mut current = 0;
    let mut start = 0;
    let mut at = 0;
    for character in text.chars() {
        if current == line {
            break;
        }
        at += character.len_utf16();
        if character == '\n' {
            current += 1;
            start = at;
        }
    }
    if current != line {
        return at;
    }
    // Walk the line, stopping at the column or the newline.
    let mut units = 0;
    let mut offset = start;
    for character in text[byte_of(text, start)..].chars() {
        if character == '\n' || units >= column {
            break;
        }
        units += character.len_utf16();
        offset += character.len_utf16();
    }
    offset
}

/// The byte offset of a UTF-16 offset.
fn byte_of(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        if units >= offset {
            return byte;
        }
        units += character.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn made(list: &[(usize, usize)]) -> Carets {
        let mut carets = Carets::one(Caret::new(list[0].0, list[0].1));
        for (anchor, head) in &list[1..] {
            carets.add(Caret::new(*anchor, *head));
        }
        carets
    }

    #[test]
    fn carets_stay_ordered_and_apart() {
        let carets = made(&[(10, 12), (2, 4), (20, 20)]);
        assert_eq!(
            carets.all().iter().map(|c| c.range()).collect::<Vec<_>>(),
            vec![(2, 4), (10, 12), (20, 20)]
        );
    }

    #[test]
    fn two_carets_in_one_place_are_one_caret() {
        // Typing with two carets at the same offset types everything
        // twice, so they merge.
        let carets = made(&[(5, 5), (5, 5)]);
        assert_eq!(carets.len(), 1);
        // And overlapping selections become the one they cover.
        let carets = made(&[(2, 8), (5, 12)]);
        assert_eq!(carets.len(), 1);
        assert_eq!(carets.all()[0].range(), (2, 12));
    }

    #[test]
    fn the_primary_survives_a_caret_being_added_before_it() {
        let mut carets = Carets::one(Caret::at(10));
        carets.add(Caret::at(2));
        assert_eq!(carets.primary().range(), (10, 10));
        carets.collapse();
        assert_eq!(carets.len(), 1);
        assert_eq!(carets.primary().range(), (10, 10));
    }

    #[test]
    fn typing_edits_every_caret_back_to_front() {
        let edits = insert(&made(&[(2, 2), (10, 10)]), "x");
        // Back to front, so applying them in order keeps the offsets of
        // the ones not yet applied.
        assert_eq!(edits[0].start, 10);
        assert_eq!(edits[1].start, 2);
        assert!(edits.iter().all(|edit| edit.text == "x"));
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let edits = insert(&made(&[(2, 6)]), "x");
        assert_eq!(edits[0], Edit { start: 2, end: 6, text: "x".into() });
    }

    #[test]
    fn backspace_takes_a_character_or_the_selection() {
        let edits = delete_backward(&made(&[(5, 5), (10, 14)]));
        assert_eq!(edits[0], Edit { start: 10, end: 14, text: String::new() });
        assert_eq!(edits[1], Edit { start: 4, end: 5, text: String::new() });
        // And nothing at the very start.
        assert!(delete_backward(&made(&[(0, 0)])).is_empty());
    }

    #[test]
    fn forward_delete_stops_at_the_end() {
        let edits = delete_forward(&made(&[(5, 5)]), 10);
        assert_eq!(edits[0], Edit { start: 5, end: 6, text: String::new() });
        assert!(delete_forward(&made(&[(10, 10)]), 10).is_empty());
    }

    #[test]
    fn the_carets_land_after_what_replaced_them() {
        // "abcdefghij" with carets at 2 and 6, typing "XY" at each.
        let edits = insert(&made(&[(2, 2), (6, 6)]), "XY");
        let after = after(&edits);
        // The first lands after its own insertion; the second lands
        // after its own, shifted by the first.
        assert_eq!(
            after.all().iter().map(|c| c.range()).collect::<Vec<_>>(),
            vec![(4, 4), (10, 10)]
        );
    }

    #[test]
    fn deleting_moves_the_carets_back() {
        let edits = delete_backward(&made(&[(3, 3), (7, 7)]));
        let after = after(&edits);
        assert_eq!(
            after.all().iter().map(|c| c.range()).collect::<Vec<_>>(),
            vec![(2, 2), (5, 5)]
        );
    }

    #[test]
    fn a_caret_goes_above_and_below_at_the_same_column() {
        let text = "one\ntwo\nthree\n";
        let carets = Carets::one(Caret::at(5)); // line 1, column 1
        assert_eq!(above(text, &carets), Some(Caret::at(1)));
        assert_eq!(below(text, &carets), Some(Caret::at(9)));
        // Nothing above the first line.
        assert_eq!(above(text, &Carets::one(Caret::at(1))), None);
        // A trailing newline makes an empty last line, and a caret can
        // go there; below that one there is nothing.
        assert_eq!(below(text, &Carets::one(Caret::at(13))), Some(Caret::at(14)));
        assert_eq!(below(text, &Carets::one(Caret::at(14))), None);
    }

    #[test]
    fn a_short_line_does_not_grow_to_meet_the_caret() {
        let text = "a\nlonger line\n";
        // Column 8 of line 1, going up to a line with one character.
        let carets = Carets::one(Caret::at(10));
        assert_eq!(above(text, &carets), Some(Caret::at(1)));
    }

    #[test]
    fn the_next_occurrence_skips_the_ones_already_taken() {
        let text = "item = item + items";
        let carets = Carets::one(Caret::new(0, 4)); // the first "item"
        let next = next_occurrence(text, &carets).unwrap();
        assert_eq!(next.range(), (7, 11));

        let mut carets = carets;
        carets.add(next);
        // The next one is inside "items", which is still an occurrence.
        assert_eq!(next_occurrence(text, &carets).unwrap().range(), (14, 18));
    }

    #[test]
    fn the_next_occurrence_wraps_and_gives_up() {
        let text = "item item";
        let mut carets = Carets::one(Caret::new(5, 9));
        // From the second, the next is the first: it wraps.
        assert_eq!(next_occurrence(text, &carets).unwrap().range(), (0, 4));
        carets.add(Caret::new(0, 4));
        // Both taken, so there is nowhere left to go.
        assert_eq!(next_occurrence(text, &carets), None);
        // And nothing selected is no word to look for.
        assert_eq!(next_occurrence(text, &Carets::one(Caret::at(2))), None);
    }

    #[test]
    fn offsets_are_utf16_units() {
        let text = "🌱\nab\n";
        // The emoji is two units, so line 1 starts at 3 and offset 4 is
        // its column 1. Column 1 of line 0 lands inside the emoji, and
        // a caret cannot go there — the line is two units long, so it
        // clamps past it.
        assert_eq!(above(text, &Carets::one(Caret::at(4))), Some(Caret::at(2)));
        // And from further along line 1, the same clamp.
        assert_eq!(above(text, &Carets::one(Caret::at(5))), Some(Caret::at(2)));
    }
}
