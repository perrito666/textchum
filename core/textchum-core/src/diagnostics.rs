//! A server's findings, kept on the text they are about.
//!
//! A language server reports a finding as a (line, column) pair against
//! the text it last saw, and the text keeps changing under it: cargo's
//! lints only run again on a save, so a "never constructed" warning can
//! be several edits old by the time the next one arrives. Resolving the
//! pair against the current text on every paint puts the mark on
//! whatever line the number now names, which is not the struct. Held as
//! UTF-16 ranges and shifted by every edit in
//! [`crate::Document::mutate_buffer`], a finding follows its code
//! until the server says otherwise, the way a snippet's placeholders do.

/// One finding, as a range of the current text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// UTF-16 offsets into the text.
    pub start: usize,
    pub end: usize,
    /// 1 = error, 2 = warning, 3 = information, 4 = hint.
    pub severity: u8,
    pub message: String,
}

/// Where a finding's range ends up after `start..end` of the text was
/// replaced by `new_len` units.
///
/// A finding entirely before the edit stays; one entirely after it
/// moves by the change in length. One the edit touches keeps whatever
/// of it survives: an edit inside it stretches or shrinks it, an edit
/// across one of its edges trims it to the other edge, and an edit
/// swallowing it whole leaves an empty range where it was, which the
/// shells still paint as a point. An insertion exactly at its end
/// stays outside it (typing after a word is not typing in it), and one
/// exactly at its start pushes it along.
pub fn shifted(range: (usize, usize), start: usize, end: usize, new_len: usize) -> (usize, usize) {
    let (from, to) = range;
    let delta = new_len as isize - (end - start) as isize;
    let moved = |offset: usize| (offset as isize + delta).max(0) as usize;
    if to <= start {
        (from, to)
    } else if from >= end {
        (moved(from), moved(to))
    } else {
        let new_from = from.min(start);
        let new_to = if to >= end { moved(to) } else { start + new_len };
        (new_from, new_to.max(new_from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finding_before_the_edit_stays_put() {
        assert_eq!(shifted((2, 5), 8, 8, 3), (2, 5));
        assert_eq!(shifted((2, 5), 5, 9, 0), (2, 5));
    }

    #[test]
    fn a_finding_after_the_edit_moves_with_the_text() {
        assert_eq!(shifted((10, 14), 0, 0, 3), (13, 17));
        assert_eq!(shifted((10, 14), 2, 6, 0), (6, 10));
        assert_eq!(shifted((10, 14), 10, 10, 2), (12, 16));
    }

    #[test]
    fn typing_inside_a_finding_stretches_it_and_deleting_shrinks_it() {
        assert_eq!(shifted((10, 14), 12, 12, 3), (10, 17));
        assert_eq!(shifted((10, 14), 11, 13, 0), (10, 12));
    }

    #[test]
    fn an_edit_across_an_edge_trims_the_finding_to_the_other_edge() {
        assert_eq!(shifted((10, 14), 8, 12, 1), (8, 11));
        assert_eq!(shifted((10, 14), 12, 20, 1), (10, 13));
    }

    #[test]
    fn an_edit_swallowing_the_finding_leaves_a_point_where_it_was() {
        assert_eq!(shifted((10, 14), 8, 20, 0), (8, 8));
        assert_eq!(shifted((10, 14), 8, 20, 5), (8, 13));
    }

    #[test]
    fn an_insertion_at_the_end_stays_outside() {
        assert_eq!(shifted((10, 14), 14, 14, 3), (10, 14));
    }
}
