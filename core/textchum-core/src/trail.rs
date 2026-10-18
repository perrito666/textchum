//! Where the text was changed: the trail Go to Last Edit walks.
//!
//! Each document keeps the places it was edited, newest last, and moves
//! them with every later edit the way it moves its findings. A burst of
//! typing is one place, not one per keystroke: an edit on or next to a
//! line already in the trail replaces that entry. Every entry carries a
//! stamp from one clock shared by all documents, so a shell holding
//! several can tell which of them was changed last.

use std::sync::atomic::{AtomicU64, Ordering};

/// One clock for every document: a later edit anywhere has a larger
/// stamp. Starts at 1 so 0 can mean "none" across the C boundary.
static CLOCK: AtomicU64 = AtomicU64::new(1);

/// How many places a document keeps.
const CAP: usize = 100;

/// One place something was changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    /// UTF-16 offset into the current text: where the edit ended.
    pub offset: usize,
    /// When, on the shared clock.
    pub stamp: u64,
}

/// A document's edit places, oldest first.
#[derive(Debug, Clone, Default)]
pub struct Trail {
    places: Vec<Place>,
}

impl Trail {
    /// Records an edit that ended at `offset`. `line_of` gives the line
    /// of an offset in the text as it is now. Places on that line or
    /// the next one either way are the same place, typed in again.
    pub fn note(&mut self, offset: usize, line_of: impl Fn(usize) -> usize) {
        let line = line_of(offset);
        self.places.retain(|place| line_of(place.offset).abs_diff(line) > 1);
        self.places.push(Place { offset, stamp: CLOCK.fetch_add(1, Ordering::Relaxed) });
        if self.places.len() > CAP {
            self.places.remove(0);
        }
    }

    /// Moves the places for `start..end` of the text replaced by
    /// `new_len` units. A place inside what was removed goes to where
    /// the removal was: something was still changed there.
    pub fn shift(&mut self, start: usize, end: usize, new_len: usize) {
        for place in &mut self.places {
            place.offset = crate::diagnostics::shifted((place.offset, place.offset), start, end, new_len).0;
        }
    }

    /// The newest place older than `before`; with `before` None, the
    /// newest of all.
    pub fn latest_before(&self, before: Option<u64>) -> Option<Place> {
        self.places
            .iter()
            .rev()
            .find(|place| before.is_none_or(|before| place.stamp < before))
            .copied()
    }

    pub fn places(&self) -> &[Place] {
        &self.places
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines of ten units each.
    fn line_of(offset: usize) -> usize {
        offset / 10
    }

    #[test]
    fn a_burst_of_typing_is_one_place_and_a_return_moves_it_to_the_front() {
        let mut trail = Trail::default();
        trail.note(3, line_of);
        trail.note(4, line_of);
        trail.note(15, line_of); // the next line: still the same place
        trail.note(52, line_of);
        let offsets: Vec<usize> = trail.places().iter().map(|place| place.offset).collect();
        assert_eq!(offsets, [15, 52]);
        trail.note(16, line_of);
        let offsets: Vec<usize> = trail.places().iter().map(|place| place.offset).collect();
        assert_eq!(offsets, [52, 16], "back where it was edited before, that place is newest");
    }

    #[test]
    fn walking_back_goes_one_older_each_time() {
        let mut trail = Trail::default();
        for offset in [5, 35, 75] {
            trail.note(offset, line_of);
        }
        let newest = trail.latest_before(None).unwrap();
        assert_eq!(newest.offset, 75);
        let older = trail.latest_before(Some(newest.stamp)).unwrap();
        assert_eq!(older.offset, 35);
        let oldest = trail.latest_before(Some(older.stamp)).unwrap();
        assert_eq!(oldest.offset, 5);
        assert_eq!(trail.latest_before(Some(oldest.stamp)), None);
    }

    #[test]
    fn places_ride_later_edits() {
        let mut trail = Trail::default();
        trail.note(40, line_of);
        trail.shift(0, 0, 5);
        assert_eq!(trail.places()[0].offset, 45);
        trail.shift(40, 50, 0);
        assert_eq!(trail.places()[0].offset, 40, "what was removed leaves its place where it was");
    }
}
