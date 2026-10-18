//! Where the text was changed: the trail Go to Last Edit walks.
//!
//! Each document keeps the places it was edited, newest last, and moves
//! them with every later edit the way it moves its findings. A burst of
//! typing is one place, not one per keystroke: an edit on or next to a
//! line already in the trail replaces that entry. Every entry carries a
//! stamp from one clock shared by all documents, so a shell holding
//! several can tell which of them was changed last.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

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

// MARK: Recent places

/// How many places Recent Places lists, and how many visits are kept.
const RECENT_PLACES: usize = 50;

/// The places jumped away from, oldest first: the other half of where
/// the user has been, beside where they edited. One list for the app,
/// as the clock is one.
static VISITS: Mutex<Vec<(String, usize, u64)>> = Mutex::new(Vec::new());

/// Notes that the caret was at `line` of `path` when a jump left it.
pub fn note_visit(path: &str, line: usize) {
    let stamp = CLOCK.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut visits) = VISITS.lock() {
        visits.retain(|(known, at, _)| known != path || at.abs_diff(line) > 1);
        visits.push((path.to_owned(), line, stamp));
        if visits.len() > RECENT_PLACES {
            visits.remove(0);
        }
    }
}

/// Recent Places: where the user has been lately, most recent first —
/// the places jumped away from and the places edited, which the shell
/// passes in as (path, line, stamp) from its open documents. Places on
/// the same line or the next one either way in one file are one place,
/// listed at its latest.
pub fn recent_places(edits: &[(String, usize, u64)]) -> Vec<(String, usize)> {
    let mut all: Vec<(String, usize, u64)> = edits.to_vec();
    if let Ok(visits) = VISITS.lock() {
        all.extend(visits.iter().cloned());
    }
    all.sort_by(|a, b| b.2.cmp(&a.2));
    let mut places: Vec<(String, usize)> = Vec::new();
    for (path, line, _) in all {
        if !places.iter().any(|(known, at)| *known == path && at.abs_diff(line) <= 1) {
            places.push((path, line));
        }
        if places.len() == RECENT_PLACES {
            break;
        }
    }
    places
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
    fn recent_places_mix_visits_and_edits_newest_first_once_each() {
        let edit_stamp = CLOCK.fetch_add(1, Ordering::Relaxed);
        note_visit("/test-visits/a.rs", 10);
        note_visit("/test-visits/b.rs", 4);
        let later_edit = CLOCK.fetch_add(1, Ordering::Relaxed);
        let edits = vec![
            ("/test-visits/a.rs".to_owned(), 40, edit_stamp),
            ("/test-visits/b.rs".to_owned(), 5, later_edit),
        ];
        let places: Vec<(String, usize)> = recent_places(&edits)
            .into_iter()
            .filter(|(path, _)| path.starts_with("/test-visits/"))
            .collect();
        assert_eq!(
            places,
            vec![
                ("/test-visits/b.rs".to_owned(), 5),
                ("/test-visits/a.rs".to_owned(), 10),
                ("/test-visits/a.rs".to_owned(), 40),
            ],
            "the edit beside a visit is one place, at its latest"
        );
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
