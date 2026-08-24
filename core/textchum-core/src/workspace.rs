//! The workspace model: which project does a file belong to?
//!
//! A file's *project root* is the nearest ancestor directory carrying a
//! root marker — a VCS directory or a build/manifest file. This single
//! notion of "project" drives everything that groups by project: the file
//! navigation drawer, and (next) one language-server instance per project.
//!
//! Detection is deliberately dumb and predictable: nearest marker wins,
//! no scoring. Monorepos with nested manifests therefore resolve to the
//! *innermost* project, which is what per-project language servers want;
//! when the guess is wrong the fix is a manual override in the UI (to
//! come), not a cleverer heuristic.

use std::path::{Path, PathBuf};

/// Marker files/directories that make a directory a project root, checked
/// in order. `.textchum.json` first so an explicit marker always wins.
pub const ROOT_MARKERS: &[&str] = &[
    ".textchum.json",
    ".git",
    ".hg",
    ".svn",
    "Cargo.toml",
    "go.mod",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "requirements.txt",
    "Package.swift",
    "build.zig",
    "Makefile",
];

/// The project root for `path` (a file or directory): the nearest ancestor
/// containing a root marker. `None` for loose files with no project.
pub fn project_root_for(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() { path } else { path.parent()? };
    let mut current = Some(start);
    while let Some(dir) = current {
        if ROOT_MARKERS.iter().any(|marker| dir.join(marker).exists()) {
            return Some(dir.to_owned());
        }
        current = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("textchum-ws-{}", std::process::id()))
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn nearest_marker_wins() {
        let outer = scratch("mono");
        std::fs::create_dir_all(outer.join(".git")).unwrap();
        let inner = outer.join("services/api");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("Cargo.toml"), "").unwrap();
        let file = inner.join("src/main.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "").unwrap();

        assert_eq!(project_root_for(&file), Some(inner.clone()));
        // A sibling without its own manifest resolves to the repo root.
        let doc = outer.join("README.md");
        std::fs::write(&doc, "").unwrap();
        assert_eq!(project_root_for(&doc), Some(outer));
    }

    #[test]
    fn files_without_markers_are_loose() {
        let dir = scratch("loose-standalone");
        let file = dir.join("notes.txt");
        std::fs::write(&file, "").unwrap();
        // The system temp dir tree should carry no markers; if this ever
        // flakes, the environment has a marker above the temp dir.
        assert_eq!(project_root_for(&file), None);
    }

    #[test]
    fn directories_resolve_like_their_files() {
        let root = scratch("dirproj");
        std::fs::write(root.join("go.mod"), "").unwrap();
        let sub = root.join("internal/util");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(project_root_for(&sub), Some(root));
    }
}
