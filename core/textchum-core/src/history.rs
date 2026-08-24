//! Linear undo/redo history with typing coalescing.
//!
//! Every applied edit is recorded as an invertible [`EditRecord`]. Undo pops
//! a record and applies its inverse; redo replays it. Two bookkeeping rules
//! make the rest of the editor simple:
//!
//! * **Coalescing.** Consecutive plain typing (contiguous insertions without
//!   a newline) and consecutive backspaces/forward-deletes merge into one
//!   record, so undo works in human-sized steps rather than per keystroke.
//! * **Save-point tracking.** The history remembers the stack depth at the
//!   last save; a document is dirty exactly when the current depth differs.
//!   Coalescing is suppressed at the save point so a record that predates a
//!   save is never mutated afterwards, and the save point is invalidated if
//!   new edits truncate the redo branch it lived on.

/// One invertible edit: at `start_byte`, `old` was replaced by `new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditRecord {
    pub start_byte: usize,
    pub old: String,
    pub new: String,
}

impl EditRecord {
    fn is_pure_insert(&self) -> bool {
        self.old.is_empty() && !self.new.is_empty()
    }

    fn is_pure_delete(&self) -> bool {
        self.new.is_empty() && !self.old.is_empty()
    }
}

#[derive(Debug, Default)]
pub(crate) struct History {
    undo: Vec<EditRecord>,
    redo: Vec<EditRecord>,
    /// `undo.len()` at the moment of the last save, if that state is still
    /// reachable through undo/redo alone.
    saved_at: Option<usize>,
    /// Set by [`History::break_group`] to force the next edit into a fresh
    /// record regardless of adjacency.
    group_broken: bool,
}

impl History {
    /// Records an applied edit, coalescing with the previous record when the
    /// coalescing rules allow it. No-op edits are ignored.
    pub fn record(&mut self, edit: EditRecord) {
        if edit.old.is_empty() && edit.new.is_empty() {
            return;
        }
        self.redo.clear();
        if let Some(saved_at) = self.saved_at {
            // The saved state lived down the redo branch we just discarded.
            if saved_at > self.undo.len() {
                self.saved_at = None;
            }
        }

        let broke = std::mem::take(&mut self.group_broken);
        // Never mutate the record that represents the saved state.
        let at_save_point = self.saved_at == Some(self.undo.len());
        if !broke && !at_save_point {
            if let Some(last) = self.undo.last_mut() {
                if try_coalesce(last, &edit) {
                    return;
                }
            }
        }
        self.undo.push(edit);
    }

    /// Ends the current coalescing run (e.g. because the caret moved).
    pub fn break_group(&mut self) {
        self.group_broken = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Pops the newest record for the caller to invert. The record moves to
    /// the redo stack.
    pub fn pop_undo(&mut self) -> Option<&EditRecord> {
        let record = self.undo.pop()?;
        self.group_broken = true;
        self.redo.push(record);
        self.redo.last()
    }

    /// Pops the newest undone record for the caller to replay. The record
    /// moves back to the undo stack.
    pub fn pop_redo(&mut self) -> Option<&EditRecord> {
        let record = self.redo.pop()?;
        self.group_broken = true;
        self.undo.push(record);
        self.undo.last()
    }

    /// Marks the current state as saved.
    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.undo.len());
        // A later coalesce into the newest record would silently change the
        // "saved" content; force the next edit into a fresh record instead.
        self.group_broken = true;
    }

    /// Whether the current state differs from the last saved one.
    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.undo.len())
    }
}

/// Merges `next` into `last` when they form one uninterrupted typing or
/// deletion run. Returns whether the merge happened.
fn try_coalesce(last: &mut EditRecord, next: &EditRecord) -> bool {
    // Typing: an insertion starting exactly where the previous one ended.
    // A newline on either side ends the run so undo stops at line
    // granularity.
    if last.is_pure_insert()
        && next.is_pure_insert()
        && !last.new.contains('\n')
        && !next.new.contains('\n')
        && next.start_byte == last.start_byte + last.new.len()
    {
        last.new.push_str(&next.new);
        return true;
    }
    if last.is_pure_delete() && next.is_pure_delete() && !next.old.contains('\n') {
        // Backspace run: each deletion ends where the previous one started.
        if next.start_byte + next.old.len() == last.start_byte {
            last.start_byte = next.start_byte;
            last.old = format!("{}{}", next.old, last.old);
            return true;
        }
        // Forward-delete run: repeated deletion at the same position.
        if next.start_byte == last.start_byte {
            last.old.push_str(&next.old);
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(at: usize, text: &str) -> EditRecord {
        EditRecord {
            start_byte: at,
            old: String::new(),
            new: text.into(),
        }
    }

    fn delete(at: usize, text: &str) -> EditRecord {
        EditRecord {
            start_byte: at,
            old: text.into(),
            new: String::new(),
        }
    }

    #[test]
    fn typing_coalesces_into_one_record() {
        let mut h = History::default();
        h.record(insert(0, "h"));
        h.record(insert(1, "e"));
        h.record(insert(2, "y"));
        assert_eq!(h.undo.len(), 1);
        assert_eq!(h.undo[0].new, "hey");
    }

    #[test]
    fn newline_breaks_the_run() {
        let mut h = History::default();
        h.record(insert(0, "a"));
        h.record(insert(1, "\n"));
        h.record(insert(2, "b"));
        assert_eq!(h.undo.len(), 3);
    }

    #[test]
    fn non_adjacent_insert_breaks_the_run() {
        let mut h = History::default();
        h.record(insert(0, "ab"));
        h.record(insert(0, "c"));
        assert_eq!(h.undo.len(), 2);
    }

    #[test]
    fn backspaces_coalesce_backwards() {
        let mut h = History::default();
        h.record(delete(2, "c"));
        h.record(delete(1, "b"));
        h.record(delete(0, "a"));
        assert_eq!(h.undo.len(), 1);
        assert_eq!(h.undo[0], delete(0, "abc"));
    }

    #[test]
    fn forward_deletes_coalesce_in_place() {
        let mut h = History::default();
        h.record(delete(3, "x"));
        h.record(delete(3, "y"));
        assert_eq!(h.undo.len(), 1);
        assert_eq!(h.undo[0], delete(3, "xy"));
    }

    #[test]
    fn break_group_forces_new_record() {
        let mut h = History::default();
        h.record(insert(0, "a"));
        h.break_group();
        h.record(insert(1, "b"));
        assert_eq!(h.undo.len(), 2);
    }

    #[test]
    fn dirty_tracking_across_undo_redo() {
        let mut h = History::default();
        assert!(h.is_dirty()); // never saved
        h.mark_saved();
        assert!(!h.is_dirty());
        h.record(insert(0, "a"));
        assert!(h.is_dirty());
        h.pop_undo();
        assert!(!h.is_dirty());
        h.pop_redo();
        assert!(h.is_dirty());
    }

    #[test]
    fn save_point_suppresses_coalescing() {
        let mut h = History::default();
        h.record(insert(0, "a"));
        h.mark_saved();
        h.record(insert(1, "b"));
        assert_eq!(h.undo.len(), 2, "post-save typing must not merge");
        h.pop_undo();
        assert!(!h.is_dirty());
    }

    #[test]
    fn save_point_dies_with_its_redo_branch() {
        let mut h = History::default();
        h.record(insert(0, "a"));
        h.mark_saved();
        h.pop_undo();
        assert!(h.is_dirty());
        h.record(insert(0, "z")); // discards the redo branch holding the save
        assert!(h.is_dirty());
        h.pop_undo();
        assert!(h.is_dirty(), "saved state is no longer reachable");
    }
}
