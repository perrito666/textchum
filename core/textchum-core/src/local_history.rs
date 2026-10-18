//! Local history: the versions of a file the editor saved, kept apart
//! from the file so an earlier one can be looked at and brought back —
//! after a save that should not have happened, a formatter that went
//! too far, or a change thrown away by a checkout.
//!
//! Every save records what was written, and, when the file on disk is
//! not the newest version already kept, what was there before, so the
//! first save of a file keeps the version it replaced. A version the
//! same as the newest is not kept twice.
//!
//! Where the versions live is the shell's to say ([`set_directory`]);
//! without a directory nothing is kept. One folder per file, named
//! after it and a hash of its path, holds the path itself and one text
//! file per version named by the time it was kept. At most
//! [`KEEP_VERSIONS`] are kept per file, and [`sweep`] drops those older
//! than [`KEEP_DAYS`]. A file larger than [`MAX_BYTES`] is not kept.
//! Failing to keep a version never fails the save.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::fsutil::write_atomically;

pub const KEEP_VERSIONS: usize = 100;
pub const KEEP_DAYS: u64 = 30;
pub const MAX_BYTES: usize = 4 * 1024 * 1024;

static DIRECTORY: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Where versions are kept from now on; None keeps none.
pub fn set_directory(dir: Option<&Path>) {
    if let Ok(mut directory) = DIRECTORY.write() {
        *directory = dir.map(Path::to_path_buf);
    }
}

fn directory() -> Option<PathBuf> {
    DIRECTORY.read().ok().and_then(|directory| directory.clone())
}

/// One kept version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Names it to [`read`]: the time it was kept, in milliseconds.
    pub id: String,
    pub time_ms: u64,
    pub size: u64,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_millis() as u64).unwrap_or(0)
}

/// FNV-1a, 64 bits, over the path: stable across runs and platforms,
/// which the standard hasher is not promised to be.
fn fingerprint(path: &Path) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in path.to_string_lossy().bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The folder of `path`'s versions in `dir`: readable by a person who
/// goes looking, unique by the hash.
fn folder(dir: &Path, path: &Path) -> PathBuf {
    let name: String = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' })
        .take(40)
        .collect();
    dir.join(format!("{name}-{:016x}", fingerprint(path)))
}

/// Keeps `text` as the newest version of `path` in `dir`, unless it is
/// the newest already. `time_ms` is when it was that version.
pub fn record_in(dir: &Path, path: &Path, text: &str, time_ms: u64) -> std::io::Result<()> {
    if text.len() > MAX_BYTES {
        return Ok(());
    }
    let folder = folder(dir, path);
    std::fs::create_dir_all(&folder)?;
    let path_file = folder.join("path");
    if !path_file.exists() {
        write_atomically(&path_file, path.to_string_lossy().as_bytes())?;
    }
    if let Some(newest) = versions_in(dir, path).first() {
        if read_in(dir, path, &newest.id).as_deref() == Some(text) {
            return Ok(());
        }
    }
    // Two versions in one millisecond still get a name each.
    let mut time = time_ms;
    while folder.join(format!("{time}.txt")).exists() {
        time += 1;
    }
    write_atomically(&folder.join(format!("{time}.txt")), text.as_bytes())?;
    for old in versions_in(dir, path).iter().skip(KEEP_VERSIONS) {
        let _ = std::fs::remove_file(folder.join(format!("{}.txt", old.id)));
    }
    Ok(())
}

/// The versions of `path` kept in `dir`, newest first.
pub fn versions_in(dir: &Path, path: &Path) -> Vec<Version> {
    let folder = folder(dir, path);
    let Ok(entries) = std::fs::read_dir(&folder) else { return Vec::new() };
    let mut versions: Vec<Version> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let time_ms = name.strip_suffix(".txt")?.parse::<u64>().ok()?;
            let size = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            Some(Version { id: time_ms.to_string(), time_ms, size })
        })
        .collect();
    versions.sort_by(|a, b| b.time_ms.cmp(&a.time_ms));
    versions
}

/// The text of version `id` of `path` kept in `dir`.
pub fn read_in(dir: &Path, path: &Path, id: &str) -> Option<String> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    std::fs::read_to_string(folder(dir, path).join(format!("{id}.txt"))).ok()
}

/// Drops the versions in `dir` older than `keep_days`, and the folders
/// left with none. Returns how many versions went.
pub fn sweep_in(dir: &Path, keep_days: u64) -> usize {
    let cutoff = now_ms().saturating_sub(Duration::from_secs(keep_days * 24 * 3600).as_millis() as u64);
    let Ok(folders) = std::fs::read_dir(dir) else { return 0 };
    let mut dropped = 0;
    for folder in folders.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()) {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        let mut left = 0;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(time_ms) = name.strip_suffix(".txt").and_then(|stem| stem.parse::<u64>().ok()) else {
                continue;
            };
            if time_ms < cutoff && std::fs::remove_file(entry.path()).is_ok() {
                dropped += 1;
            } else {
                left += 1;
            }
        }
        if left == 0 {
            let _ = std::fs::remove_dir_all(&folder);
        }
    }
    dropped
}

/// Called by a document as it saves `text` to `path`, before writing:
/// keeps what the file held, when that is not the newest version kept.
pub(crate) fn before_save(path: &Path) {
    let Some(dir) = directory() else { return };
    let Ok(metadata) = std::fs::metadata(path) else { return };
    if metadata.len() as usize > MAX_BYTES {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else { return };
    let text = String::from_utf8_lossy(&bytes);
    let time_ms = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_millis() as u64)
        .unwrap_or_else(now_ms);
    // The newest kept may be later than the file's own time, when the
    // file was put back as it was; this version still goes before it
    // only if it is a different text, which record_in checks.
    let _ = record_in(&dir, path, &text, time_ms.min(now_ms()));
}

/// Called by a document once `text` is saved to `path`.
pub(crate) fn after_save(path: &Path, text: &str) {
    if let Some(dir) = directory() {
        let _ = record_in(&dir, path, text, now_ms());
    }
}

/// The versions of `path` kept, newest first; none without a directory.
pub fn versions(path: &Path) -> Vec<Version> {
    directory().map(|dir| versions_in(&dir, path)).unwrap_or_default()
}

pub fn read(path: &Path, id: &str) -> Option<String> {
    read_in(&directory()?, path, id)
}

/// Drops versions older than [`KEEP_DAYS`]; at launch, off the main
/// thread.
pub fn sweep() -> usize {
    directory().map(|dir| sweep_in(&dir, KEEP_DAYS)).unwrap_or(0)
}

/// The versions of `path` as JSON, each with what restoring it would
/// change in `current`: `[{id, time, size, added, removed}]`, `time` in
/// milliseconds since the epoch, newest first.
pub fn versions_json(path: &Path, current: &str) -> String {
    let rows: Vec<serde_json::Value> = versions(path)
        .into_iter()
        .map(|version| {
            let diff = read(path, &version.id).map(|text| crate::changes::unified(current, &text, 0));
            serde_json::json!({
                "id": version.id,
                "time": version.time_ms,
                "size": version.size,
                "added": diff.as_ref().map_or(0, |diff| diff.added),
                "removed": diff.as_ref().map_or(0, |diff| diff.removed),
            })
        })
        .collect();
    serde_json::Value::Array(rows).to_string()
}

/// What restoring version `id` would change in `current`, as unified
/// diff text; None for a version not kept.
pub fn diff(path: &Path, id: &str, current: &str) -> Option<String> {
    let text = read(path, id)?;
    let diff = crate::changes::unified(current, &text, 3);
    Some(if diff.too_large {
        crate::i18n::tr("Too large to compare.")
    } else if diff.text.is_empty() {
        crate::i18n::tr("The same as the text now.")
    } else {
        diff.text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("textchum-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn versions_are_kept_newest_first_and_once_each() {
        let dir = scratch("keep");
        let path = Path::new("/project/src/main.rs");
        record_in(&dir, path, "one", 1_000).unwrap();
        record_in(&dir, path, "two", 2_000).unwrap();
        record_in(&dir, path, "two", 3_000).unwrap();
        let versions = versions_in(&dir, path);
        let ids: Vec<&str> = versions.iter().map(|version| version.id.as_str()).collect();
        assert_eq!(ids, ["2000", "1000"], "the same text twice is kept once");
        assert_eq!(read_in(&dir, path, "1000").as_deref(), Some("one"));
        assert_eq!(read_in(&dir, path, "../path"), None, "an id is a time, nothing else");
        assert!(versions_in(&dir, Path::new("/project/src/other.rs")).is_empty());
        let folder = folder(&dir, path);
        assert!(folder.file_name().unwrap().to_string_lossy().starts_with("main.rs-"));
        assert_eq!(std::fs::read_to_string(folder.join("path")).unwrap(), "/project/src/main.rs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_so_many_versions_are_kept_and_old_ones_swept() {
        let dir = scratch("bounds");
        let path = Path::new("/project/notes.md");
        for index in 0..(KEEP_VERSIONS as u64 + 5) {
            record_in(&dir, path, &format!("version {index}"), 1_000 + index).unwrap();
        }
        let versions = versions_in(&dir, path);
        assert_eq!(versions.len(), KEEP_VERSIONS);
        assert_eq!(versions.last().unwrap().time_ms, 1_005, "the oldest went first");
        // Every one of them is from 1970: all older than any cutoff.
        assert_eq!(sweep_in(&dir, KEEP_DAYS), KEEP_VERSIONS);
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none(), "the empty folder went too");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
