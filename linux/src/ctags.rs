//! Symbol definitions from Universal Ctags, the fallback for projects
//! whose language has no server running — the Rust twin of the macOS
//! CtagsIndex. Off by default; the workspace section's
//! `ctags_fallback` flag opts a project (or everything) in. One index
//! per project root, cached briefly so repeated jumps stay instant and
//! edits are picked up soon after.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

struct Index {
    built: Instant,
    /// Symbol name → definition sites, in ctags output order.
    symbols: HashMap<String, Vec<(String, usize)>>,
}

thread_local! {
    static INDEXES: RefCell<HashMap<String, Index>> = RefCell::new(HashMap::new());
    /// The resolved Universal Ctags executable; a completed search that
    /// found nothing is remembered too.
    static BINARY: RefCell<Option<Option<String>>> = const { RefCell::new(None) };
}

/// The first definition site of `name` inside `root`'s project, with an
/// absolute path and zero-based line — or `None` when the symbol is
/// unknown or ctags is unavailable.
pub fn definition(name: &str, root: &Path) -> Option<(PathBuf, i32)> {
    let key = root.to_string_lossy().into_owned();
    refresh(&key, root);
    INDEXES.with(|indexes| {
        let indexes = indexes.borrow();
        let found = indexes.get(&key)?.symbols.get(name)?.first()?;
        Some((root.join(&found.0), (found.1.saturating_sub(1)) as i32))
    })
}

/// Whether a Universal Ctags binary exists at all — for telling the
/// user why the fallback cannot help.
pub fn available() -> bool {
    resolve_binary().is_some()
}

fn refresh(key: &str, root: &Path) {
    let fresh = INDEXES.with(|indexes| {
        indexes
            .borrow()
            .get(key)
            .is_some_and(|index| index.built.elapsed() < Duration::from_secs(30))
    });
    if fresh {
        return;
    }
    let Some(binary) = resolve_binary() else { return };
    let Ok(output) = Command::new(&binary)
        .args(["-R", "--output-format=json", "--fields=+n", "-f", "-", "."])
        .current_dir(root)
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let mut symbols: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    for line in output.stdout.split(|byte| *byte == b'\n') {
        let Ok(tag) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if tag["_type"].as_str() != Some("tag") {
            continue;
        }
        let (Some(name), Some(path), Some(line_number)) = (
            tag["name"].as_str(),
            tag["path"].as_str(),
            tag["line"].as_u64(),
        ) else {
            continue;
        };
        symbols
            .entry(name.to_owned())
            .or_default()
            .push((path.to_owned(), line_number as usize));
    }
    eprintln!("textchum: ctags indexed {} symbols under {key}", symbols.len());
    INDEXES.with(|indexes| {
        indexes.borrow_mut().insert(
            key.to_owned(),
            Index {
                built: Instant::now(),
                symbols,
            },
        );
    });
}

/// The first candidate that really is Universal Ctags — its JSON
/// output is what this index reads; the Exuberant ctags some distros
/// still ship cannot produce it.
fn resolve_binary() -> Option<String> {
    BINARY.with(|cell| {
        if let Some(resolved) = cell.borrow().as_ref() {
            return resolved.clone();
        }
        let found = ["ctags", "uctags", "ctags-universal"]
            .iter()
            .find(|candidate| {
                Command::new(candidate)
                    .arg("--version")
                    .output()
                    .is_ok_and(|output| {
                        String::from_utf8_lossy(&output.stdout).contains("Universal Ctags")
                    })
            })
            .map(|candidate| (*candidate).to_owned());
        *cell.borrow_mut() = Some(found.clone());
        found
    })
}
