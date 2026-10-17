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

/// Where Next Finding goes from `offset`: the start of the first
/// finding that starts after it, wrapping round to the first one; going
/// back, the last that starts before it, wrapping round to the last.
/// Strictly after and before, so pressing again moves on even from
/// inside a finding. None when there are none.
pub fn neighbour(findings: &[Finding], offset: usize, forward: bool) -> Option<usize> {
    let mut starts: Vec<usize> = findings.iter().map(|finding| finding.start).collect();
    starts.sort_unstable();
    starts.dedup();
    if forward {
        starts.iter().copied().find(|start| *start > offset).or_else(|| starts.first().copied())
    } else {
        starts.iter().rev().copied().find(|start| *start < offset).or_else(|| starts.last().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(start: usize) -> Finding {
        Finding { start, end: start + 2, severity: 1, message: String::new() }
    }

    #[test]
    fn next_and_previous_move_on_and_wrap_round() {
        let findings = vec![finding(40), finding(10), finding(25), finding(25)];
        assert_eq!(neighbour(&findings, 0, true), Some(10));
        assert_eq!(neighbour(&findings, 10, true), Some(25), "from a finding's start, on to the next");
        assert_eq!(neighbour(&findings, 30, true), Some(40));
        assert_eq!(neighbour(&findings, 40, true), Some(10), "past the last, round to the first");
        assert_eq!(neighbour(&findings, 25, false), Some(10));
        assert_eq!(neighbour(&findings, 5, false), Some(40), "before the first, round to the last");
        assert_eq!(neighbour(&[], 5, true), None);
    }

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
