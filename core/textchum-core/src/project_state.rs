//! What a file remembers about itself, kept per project.
//!
//! How a file is split, where the caret and the scroll are in each
//! view, what is folded, what language it was told it is: none of that
//! is configuration, and none of it belongs to a session either — it is
//! what this file looks like when it is opened, whenever that is.
//!
//! One record per project root, JSON like everything else:
//!
//! ```json
//! {
//!   "version": 1,
//!   "root": "/work/engine",
//!   "updated": 1756500000,
//!   "files": {
//!     "src/parser.rs": {
//!       "views": 2,
//!       "dividers": [0.45],
//!       "folds": [[12, 48]],
//!       "language": "rust",
//!       "places": [{"caret": 812, "scroll": 240.0, "top": 790}]
//!     }
//!   }
//! }
//! ```
//!
//! Records live in the profile by default, one file per root, so a run
//! pointed at a scratch profile can never write into the real one. The
//! alternative is `<root>/.tchum`, for a layout that travels with the
//! checkout.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// Where a view was looking.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Place {
    /// The caret, in UTF-16 units.
    pub caret: usize,
    /// The scroll offset, in points.
    pub scroll: f64,
    /// The first character shown, in UTF-16 units: the line the view
    /// was looking at, which survives a reflow where the offset does not.
    pub top: usize,
}

/// What one file remembers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileState {
    /// Views stacked in the column showing it; one when it is not
    /// stacked at all.
    pub views: usize,
    /// Where the dividers between them sit, as fractions of the
    /// column's height.
    pub dividers: Vec<f64>,
    /// Folded stretches, as first and last line, both zero-based.
    pub folds: Vec<(usize, usize)>,
    /// The language this file was told it is, when its name does not
    /// say. Data about the file, which is why it is here.
    pub language: Option<String>,
    /// Where each view was looking, in the order they are stacked.
    pub places: Vec<Place>,
}

impl FileState {
    /// Reads one entry of a record.
    pub fn from_json(value: &Value) -> FileState {
        FileState {
            views: value["views"].as_u64().unwrap_or(1) as usize,
            dividers: numbers(&value["dividers"]),
            folds: value["folds"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let pair = item.as_array()?;
                            Some((pair.first()?.as_u64()? as usize, pair.get(1)?.as_u64()? as usize))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            language: value["language"].as_str().map(str::to_owned),
            places: value["places"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| Place {
                            caret: item["caret"].as_u64().unwrap_or(0) as usize,
                            scroll: item["scroll"].as_f64().unwrap_or(0.0),
                            top: item["top"].as_u64().unwrap_or(0) as usize,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Writes one entry, leaving out what has nothing to say.
    pub fn to_json(&self) -> Value {
        let mut entry = serde_json::Map::new();
        entry.insert("views".into(), json!(self.views.max(1)));
        if !self.dividers.is_empty() {
            entry.insert("dividers".into(), json!(self.dividers));
        }
        if !self.folds.is_empty() {
            let folds: Vec<Value> = self
                .folds
                .iter()
                .map(|(start, end)| json!([start, end]))
                .collect();
            entry.insert("folds".into(), Value::Array(folds));
        }
        if let Some(language) = &self.language {
            entry.insert("language".into(), json!(language));
        }
        if !self.places.is_empty() {
            let places: Vec<Value> = self
                .places
                .iter()
                .map(|place| json!({"caret": place.caret, "scroll": place.scroll, "top": place.top}))
                .collect();
            entry.insert("places".into(), Value::Array(places));
        }
        Value::Object(entry)
    }
}

fn numbers(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

impl FileState {
    /// Whether this is worth writing down at all.
    pub fn is_empty(&self) -> bool {
        self.views <= 1
            && self.dividers.is_empty()
            && self.folds.is_empty()
            && self.language.is_none()
            && self
                .places
                .iter()
                .all(|place| place.caret == 0 && place.scroll == 0.0 && place.top == 0)
    }
}

/// One project's record.
#[derive(Clone, Debug)]
pub struct ProjectState {
    pub version: u32,
    pub root: String,
    /// When this was last written, in seconds since the epoch. The
    /// sweep goes by the file's own time; this is for reading.
    pub updated: u64,
    pub files: BTreeMap<String, FileState>,
}

impl Default for ProjectState {
    fn default() -> Self {
        ProjectState {
            version: 1,
            root: String::new(),
            updated: 0,
            files: BTreeMap::new(),
        }
    }
}

impl ProjectState {
    /// What `path` remembers, by its place in the project.
    pub fn file(&self, root: &Path, path: &Path) -> Option<&FileState> {
        self.files.get(&relative(root, path))
    }

    /// Writes down what a file remembers. An entry with nothing in it
    /// is removed instead, so a record stays a list of files that have
    /// something to say.
    pub fn set_file(&mut self, root: &Path, path: &Path, state: FileState) {
        let key = relative(root, path);
        if state.is_empty() {
            self.files.remove(&key);
        } else {
            self.files.insert(key, state);
        }
    }
}

/// A file's path relative to the project root, which is what a record
/// is keyed by: the same checkout in two places reads the same record.
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Where a project's record lives: `<root>/.tchum` when records are
/// kept with the checkout, a file of its own in `dir` otherwise.
///
/// The central name carries the project's own name so that the folder
/// can be read, and a hash of the whole path so that two projects
/// called `engine` are two records.
pub fn record_path(root: &Path, dir: &Path, in_project: bool) -> PathBuf {
    if in_project {
        return root.join(".tchum");
    }
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    dir.join(format!("{safe}-{:08x}.tchum", fingerprint(root)))
}

/// A stable 32-bit fingerprint of a path (FNV-1a). Short enough to read
/// in a file name, and only ever used to tell two roots apart.
fn fingerprint(root: &Path) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for byte in root.to_string_lossy().as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

/// Reads a project's record, or an empty one when there is none. A
/// record that fails to parse reads as empty and is replaced on the
/// next write, which is what every other file here does.
pub fn load(root: &Path, dir: &Path, in_project: bool) -> ProjectState {
    let path = record_path(root, dir, in_project);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ProjectState {
            root: root.to_string_lossy().into_owned(),
            ..ProjectState::default()
        };
    };
    let mut state = from_json(&text);
    state.root = root.to_string_lossy().into_owned();
    state
}

/// Writes a project's record. A record with nothing in it is removed
/// instead of written empty.
pub fn save(state: &ProjectState, root: &Path, dir: &Path, in_project: bool) -> std::io::Result<()> {
    let path = record_path(root, dir, in_project);
    if state.files.is_empty() {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut files = serde_json::Map::new();
    for (name, entry) in &state.files {
        files.insert(name.clone(), entry.to_json());
    }
    let document = json!({
        "version": 1,
        "root": root.to_string_lossy(),
        "updated": now(),
        "files": Value::Object(files),
    });
    let text = serde_json::to_string_pretty(&document).unwrap_or_default();
    // Atomic like every write here: temporary file, then rename.
    let temp = path.with_extension("tchum.tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, &path)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// One record, as the cleanup window lists them.
#[derive(Clone, Debug)]
pub struct Record {
    /// The project root the record is about.
    pub root: String,
    /// The record file itself.
    pub path: String,
    pub bytes: u64,
    /// When it was last written, in seconds since the epoch.
    pub updated: u64,
    /// Whether the root it describes is still there.
    pub missing: bool,
    /// How many files it has something to say about.
    pub files: usize,
}

/// Every record in `dir`, newest first.
pub fn records(dir: &Path) -> Vec<Record> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return found };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("tchum") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let state = from_json(&text);
        if state.root.is_empty() {
            continue;
        }
        let bytes = entry.metadata().map(|data| data.len()).unwrap_or(0);
        let updated = entry
            .metadata()
            .ok()
            .and_then(|data| data.modified().ok())
            .and_then(|when| when.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_secs())
            .unwrap_or(state.updated);
        found.push(Record {
            missing: !Path::new(&state.root).is_dir(),
            root: state.root,
            path: path.to_string_lossy().into_owned(),
            bytes,
            updated,
            files: state.files.len(),
        });
    }
    found.sort_by(|a, b| b.updated.cmp(&a.updated));
    found
}

/// Reads a record from its text. Anything that does not parse reads as
/// empty, which is what every other file here does.
pub fn from_json(text: &str) -> ProjectState {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return ProjectState::default();
    };
    let files = value["files"]
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .map(|(name, entry)| (name.clone(), FileState::from_json(entry)))
                .collect()
        })
        .unwrap_or_default();
    ProjectState {
        version: value["version"].as_u64().unwrap_or(1) as u32,
        root: value["root"].as_str().unwrap_or_default().to_owned(),
        updated: value["updated"].as_u64().unwrap_or(0),
        files,
    }
}

/// Every record in `dir`, as JSON for the shells to list.
pub fn records_json(dir: &Path) -> String {
    let listed: Vec<Value> = records(dir)
        .into_iter()
        .map(|record| {
            json!({
                "root": record.root,
                "path": record.path,
                "bytes": record.bytes,
                "updated": record.updated,
                "missing": record.missing,
                "files": record.files,
            })
        })
        .collect();
    serde_json::to_string(&Value::Array(listed)).unwrap_or_else(|_| "[]".into())
}

/// Forgets one record.
pub fn forget(path: &Path) -> bool {
    std::fs::remove_file(path).is_ok()
}

/// Forgets the records for roots that are gone, and those not written
/// for longer than `keep_days`. Answers how many were forgotten.
///
/// A record is small; the reason to sweep is that a machine that has
/// seen a thousand checkouts should not keep a thousand records of
/// projects it will never open again.
pub fn sweep(dir: &Path, keep_days: u64) -> usize {
    let cutoff = now().saturating_sub(keep_days.saturating_mul(24 * 60 * 60));
    let mut gone = 0;
    for record in records(dir) {
        let stale = keep_days > 0 && record.updated < cutoff;
        if record.missing || stale {
            if forget(Path::new(&record.path)) {
                gone += 1;
            }
        }
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("textchum-project-state-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_remembers_its_shape_across_a_write_and_a_read() {
        let dir = scratch("round-trip");
        let root = dir.join("project");
        std::fs::create_dir_all(&root).unwrap();
        let mut state = load(&root, &dir, false);
        state.set_file(
            &root,
            &root.join("src/parser.rs"),
            FileState {
                views: 2,
                dividers: vec![0.4],
                folds: vec![(12, 48)],
                language: Some("rust".into()),
                places: vec![Place { caret: 10, scroll: 4.0, top: 2 }, Place::default()],
            },
        );
        save(&state, &root, &dir, false).unwrap();
        let read = load(&root, &dir, false);
        let file = read.file(&root, &root.join("src/parser.rs")).unwrap();
        assert_eq!(file.views, 2);
        assert_eq!(file.folds, vec![(12, 48)]);
        assert_eq!(file.language.as_deref(), Some("rust"));
        assert_eq!(file.places[0].caret, 10);
    }

    #[test]
    fn an_entry_with_nothing_to_say_is_not_written_down() {
        let dir = scratch("empty");
        let root = dir.join("project");
        let mut state = load(&root, &dir, false);
        state.set_file(&root, &root.join("a.txt"), FileState { views: 1, ..Default::default() });
        assert!(state.files.is_empty());
        save(&state, &root, &dir, false).unwrap();
        assert!(!record_path(&root, &dir, false).exists());
    }

    #[test]
    fn two_projects_of_one_name_are_two_records() {
        let dir = scratch("names");
        let first = record_path(Path::new("/work/one/engine"), &dir, false);
        let second = record_path(Path::new("/work/two/engine"), &dir, false);
        assert_ne!(first, second);
        assert!(first.file_name().unwrap().to_string_lossy().starts_with("engine-"));
    }

    #[test]
    fn a_record_kept_with_the_checkout_is_called_tchum() {
        let path = record_path(Path::new("/work/engine"), Path::new("/elsewhere"), true);
        assert_eq!(path, Path::new("/work/engine/.tchum"));
    }

    #[test]
    fn the_sweep_forgets_roots_that_are_gone_and_keeps_the_rest() {
        let dir = scratch("sweep");
        let here = dir.join("here");
        std::fs::create_dir_all(&here).unwrap();
        let gone = dir.join("gone");
        for root in [&here, &gone] {
            let mut state = load(root, &dir, false);
            state.set_file(root, &root.join("a.rs"), FileState { views: 2, ..Default::default() });
            save(&state, root, &dir, false).unwrap();
        }
        assert_eq!(records(&dir).len(), 2);
        // Nothing is old enough to go for age; one root is not there.
        assert_eq!(sweep(&dir, 90), 1);
        let left = records(&dir);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].root, here.to_string_lossy());
        assert!(!left[0].missing);
    }

    #[test]
    fn a_record_that_does_not_parse_reads_as_empty() {
        let dir = scratch("broken");
        let root = dir.join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(record_path(&root, &dir, false), "{ not json").unwrap();
        let state = load(&root, &dir, false);
        assert!(state.files.is_empty());
        assert_eq!(state.root, root.to_string_lossy());
    }
}
