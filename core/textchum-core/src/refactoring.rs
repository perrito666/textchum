//! Refactorings that touch several files — a rename, a code action —
//! undone and redone as one, wherever they reached.
//!
//! A shell applying a workspace edit starts a refactoring and tags the
//! edit group it makes in each open document with its id, so Undo in
//! any of them can see that the step belongs to more than this file and
//! take it back in every document whose newest step it still is. A file
//! that is not open is written here instead, atomically, and what it
//! held before is kept, so undoing puts it back — unless it has changed
//! since, which leaves it alone and says so. Kept for the session only:
//! the documents' own histories do not outlive it either.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::fsutil::write_atomically;

/// How many refactorings are remembered; the oldest go first.
const KEEP: usize = 50;

struct ClosedFile {
    path: PathBuf,
    before: String,
    after: String,
}

struct Refactoring {
    id: u64,
    label: String,
    open_documents: usize,
    closed: Vec<ClosedFile>,
}

struct Registry {
    next: u64,
    refactorings: Vec<Refactoring>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry { next: 1, refactorings: Vec::new() });

fn with<T>(id: u64, read: impl FnOnce(&mut Refactoring) -> T) -> Option<T> {
    let mut registry = REGISTRY.lock().ok()?;
    registry.refactorings.iter_mut().find(|refactoring| refactoring.id == id).map(read)
}

/// Starts a refactoring called `label` ("Rename", a code action's
/// title); its id tags what it does.
pub fn begin(label: &str) -> u64 {
    let Ok(mut registry) = REGISTRY.lock() else { return 0 };
    let id = registry.next;
    registry.next += 1;
    registry.refactorings.push(Refactoring { id, label: label.to_owned(), open_documents: 0, closed: Vec::new() });
    if registry.refactorings.len() > KEEP {
        registry.refactorings.remove(0);
    }
    id
}

/// An open document took part, its edits tagged with `id`.
pub fn note_open_document(id: u64) {
    with(id, |refactoring| refactoring.open_documents += 1);
}

/// Writes `after` to `path`, a file not open, keeping what it held.
pub fn write_closed(id: u64, path: &Path, after: &str) -> Result<(), String> {
    let before = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    write_atomically(path, after.as_bytes()).map_err(|error| format!("{}: {error}", path.display()))?;
    with(id, |refactoring| {
        refactoring.closed.push(ClosedFile { path: path.to_owned(), before, after: after.to_owned() })
    });
    Ok(())
}

/// How many files refactoring `id` touched, open or not; 0 for one not
/// remembered.
pub fn files_touched(id: u64) -> usize {
    with(id, |refactoring| refactoring.open_documents + refactoring.closed.len()).unwrap_or(0)
}

pub fn label(id: u64) -> Option<String> {
    with(id, |refactoring| refactoring.label.clone())
}

/// What undoing (or with `undo` false, redoing) refactoring `id` did to
/// the files that were not open: the ones put back, and the ones left
/// alone because they changed since.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Restored {
    pub written: Vec<PathBuf>,
    pub changed_since: Vec<PathBuf>,
}

pub fn restore_closed(id: u64, undo: bool) -> Restored {
    let files: Vec<(PathBuf, String, String)> = with(id, |refactoring| {
        refactoring.closed.iter().map(|file| (file.path.clone(), file.before.clone(), file.after.clone())).collect()
    })
    .unwrap_or_default();
    let mut restored = Restored::default();
    for (path, before, after) in files {
        let (expected, wanted) = if undo { (after, before) } else { (before, after) };
        let current = std::fs::read_to_string(&path).ok();
        if current.as_deref() == Some(expected.as_str()) && write_atomically(&path, wanted.as_bytes()).is_ok() {
            restored.written.push(path);
        } else {
            restored.changed_since.push(path);
        }
    }
    restored
}

/// Undoes (or redoes) refactoring `id` in the files not open, after
/// the shell has done so in `documents` open ones, and says what was
/// done in words: how many files, and which were left alone.
pub fn finish(id: u64, undo: bool, documents: usize) -> String {
    let restored = restore_closed(id, undo);
    let label = label(id).unwrap_or_default();
    let files = documents + restored.written.len();
    let count = files.to_string();
    let mut said = if undo {
        crate::i18n::fill(&crate::i18n::tr_n("Undid {} in {} file.", "Undid {} in {} files.", files), &[&label, &count])
    } else {
        crate::i18n::fill(&crate::i18n::tr_n("Redid {} in {} file.", "Redid {} in {} files.", files), &[&label, &count])
    };
    if !restored.changed_since.is_empty() {
        let names: Vec<String> = restored
            .changed_since
            .iter()
            .map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        let left = restored.changed_since.len();
        said.push(' ');
        said.push_str(&crate::i18n::fill(
            &crate::i18n::tr_n(
                "{} file had changed since and was left alone: {}.",
                "{} files had changed since and were left alone: {}.",
                left,
            ),
            &[&left.to_string(), &names.join(", ")],
        ));
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_file_is_put_back_and_written_again_unless_it_changed() {
        let dir = std::env::temp_dir().join(format!("textchum-refactoring-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let (one, two) = (dir.join("one.rs"), dir.join("two.rs"));
        std::fs::write(&one, "old one").unwrap();
        std::fs::write(&two, "old two").unwrap();
        let id = begin("Rename");
        note_open_document(id);
        write_closed(id, &one, "new one").unwrap();
        write_closed(id, &two, "new two").unwrap();
        assert_eq!((files_touched(id), label(id).as_deref()), (3, Some("Rename")));
        // Someone edits one of them afterwards.
        std::fs::write(&two, "edited two").unwrap();
        let undone = restore_closed(id, true);
        assert_eq!((undone.written, undone.changed_since), (vec![one.clone()], vec![two.clone()]));
        assert_eq!(std::fs::read_to_string(&one).unwrap(), "old one");
        let redone = restore_closed(id, false);
        assert_eq!(redone.written, vec![one.clone()]);
        assert_eq!(std::fs::read_to_string(&one).unwrap(), "new one");
        assert_eq!(files_touched(0), 0, "an id never given out");
        assert_eq!(finish(id, true, 1), "Undid Rename in 2 files. 1 file had changed since and was left alone: two.rs.");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
