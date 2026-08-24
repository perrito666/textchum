//! Linear undo/redo history with typing coalescing and edit groups.
//!
//! Every applied edit is recorded as an invertible [`EditRecord`]. The undo
//! stack holds *groups* of records; undo pops a group and applies the
//! inverses of its records (in reverse order), redo replays them forward.
//! Three bookkeeping rules make the rest of the editor simple:
//!
//! * **Coalescing.** Consecutive plain typing (contiguous insertions without
//!   a newline) and consecutive backspaces/forward-deletes merge into one
//!   record, so undo works in human-sized steps rather than per keystroke.
//! * **Explicit groups.** A caller can bracket several edits with
//!   [`History::begin_group`]/[`History::end_group`] so a compound operation
//!   — replace-all, a reload — undoes as a single step.
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

/// One undo step: the records of a single user-visible operation, in the
/// order they were applied.
pub(crate) type Group = Vec<EditRecord>;

#[derive(Debug, Default)]
pub(crate) struct History {
    undo: Vec<Group>,
    redo: Vec<Group>,
    /// `undo.len()` at the moment of the last save, if that state is still
    /// reachable through undo/redo alone.
    saved_at: Option<usize>,
    /// Set by [`History::break_group`] to force the next edit into a fresh
    /// group regardless of adjacency.
    group_broken: bool,
    /// Records accumulated between `begin_group` and `end_group`.
    open_group: Option<Group>,
}

impl History {
    /// Records an applied edit. Inside an explicit group it simply joins the
    /// group; otherwise it coalesces with the previous step when the typing
    /// rules allow. No-op edits are ignored.
    pub fn record(&mut self, edit: EditRecord) {
        if edit.old.is_empty() && edit.new.is_empty() {
            return;
        }
        self.on_new_edit();

        if let Some(group) = &mut self.open_group {
            group.push(edit);
            return;
        }

        let broke = std::mem::take(&mut self.group_broken);
        // Never mutate the group that represents the saved state.
        let at_save_point = self.saved_at == Some(self.undo.len());
        if !broke && !at_save_point {
            // Coalescing only ever targets a plain typing/deletion step,
            // which is by construction a single-record group.
            if let Some([last]) = self.undo.last_mut().map(Vec::as_mut_slice) {
                if try_coalesce(last, &edit) {
                    return;
                }
            }
        }
        self.undo.push(vec![edit]);
    }

    /// Every new edit discards the redo branch — and the save point with
    /// it, if that is where the save lived.
    fn on_new_edit(&mut self) {
        self.redo.clear();
        if let Some(saved_at) = self.saved_at {
            if saved_at > self.undo.len() {
                self.saved_at = None;
            }
        }
    }

    /// Starts an explicit group; edits recorded until [`Self::end_group`]
    /// form one undo step. Nested calls join the already-open group.
    pub fn begin_group(&mut self) {
        if self.open_group.is_none() {
            self.open_group = Some(Vec::new());
        }
    }

    /// Commits the open group as a single undo step. A group with no edits
    /// leaves no trace. Always ends the current coalescing run.
    pub fn end_group(&mut self) {
        if let Some(group) = self.open_group.take() {
            if !group.is_empty() {
                self.undo.push(group);
            }
        }
        self.group_broken = true;
    }

    /// Ends the current coalescing run (e.g. because the caret moved).
    pub fn break_group(&mut self) {
        self.group_broken = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.open_group.as_ref().is_some_and(|g| !g.is_empty())
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Pops the newest group for the caller to invert (its records in
    /// reverse order). The group moves to the redo stack. An open explicit
    /// group is committed first.
    pub fn pop_undo(&mut self) -> Option<&Group> {
        self.end_group();
        let group = self.undo.pop()?;
        self.redo.push(group);
        self.redo.last()
    }

    /// Pops the newest undone group for the caller to replay (its records
    /// in order). The group moves back to the undo stack.
    pub fn pop_redo(&mut self) -> Option<&Group> {
        let group = self.redo.pop()?;
        self.group_broken = true;
        self.undo.push(group);
        self.undo.last()
    }

    /// Marks the current state as saved.
    pub fn mark_saved(&mut self) {
        self.end_group();
        self.saved_at = Some(self.undo.len());
    }

    /// Whether the current state differs from the last saved one.
    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.undo.len())
            || self.open_group.as_ref().is_some_and(|g| !g.is_empty())
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
    fn typing_coalesces_into_one_step() {
        let mut h = History::default();
        h.record(insert(0, "h"));
        h.record(insert(1, "e"));
        h.record(insert(2, "y"));
        assert_eq!(h.undo.len(), 1);
        assert_eq!(h.undo[0], vec![insert(0, "hey")]);
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
        assert_eq!(h.undo[0], vec![delete(0, "abc")]);
    }

    #[test]
    fn forward_deletes_coalesce_in_place() {
        let mut h = History::default();
        h.record(delete(3, "x"));
        h.record(delete(3, "y"));
        assert_eq!(h.undo.len(), 1);
        assert_eq!(h.undo[0], vec![delete(3, "xy")]);
    }

    #[test]
    fn break_group_forces_new_step() {
        let mut h = History::default();
        h.record(insert(0, "a"));
        h.break_group();
        h.record(insert(1, "b"));
        assert_eq!(h.undo.len(), 2);
    }

    #[test]
    fn explicit_group_is_one_step() {
        let mut h = History::default();
        h.begin_group();
        h.record(delete(9, "x"));
        h.record(delete(4, "y"));
        h.end_group();
        assert_eq!(h.undo.len(), 1);
        assert_eq!(h.undo[0].len(), 2);
        // Grouped records never coalesce with what follows.
        h.record(insert(0, "z"));
        assert_eq!(h.undo.len(), 2);
    }

    #[test]
    fn empty_group_leaves_no_trace() {
        let mut h = History::default();
        h.record(insert(0, "a"));
        h.begin_group();
        h.end_group();
        assert!(h.pop_undo().is_some());
        assert!(h.pop_undo().is_none());
    }

    #[test]
    fn open_group_counts_as_dirty_and_undoable() {
        let mut h = History::default();
        h.mark_saved();
        h.begin_group();
        assert!(!h.is_dirty());
        h.record(insert(0, "a"));
        assert!(h.is_dirty());
        assert!(h.can_undo());
        let group = h.pop_undo().unwrap(); // commits the open group
        assert_eq!(group.len(), 1);
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
