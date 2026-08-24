//! The workspace model: which project does a file belong to?
//!
//! A file's *project root* drives everything that groups by project: the
//! file navigation drawer and the one-language-server-per-project pool.
//! Resolution, in order:
//!
//! 1. **The nearest `.textchum.json`** — the explicit, human-placed
//!    override always wins.
//! 2. **The outermost version-control root** (`.git`/`.hg`/`.svn`). A
//!    repository is one project no matter how many nested manifests it
//!    contains: a Python package inside a repo belongs to the repo, a
//!    workspace-member crate belongs to the workspace's repo, and nested
//!    repositories (submodules) resolve to the outermost one.
//! 3. **The nearest language manifest** (`Cargo.toml`, `pyproject.toml`,
//!    …) — the fallback for trees that are not under version control.
//!
//! `None` only for genuinely loose files.

use std::path::{Path, PathBuf};

/// Version-control directories: the outermost one wins.
pub const VCS_MARKERS: &[&str] = &[".git", ".hg", ".svn"];

/// Build/manifest files: outside version control, the nearest one wins.
pub const MANIFEST_MARKERS: &[&str] = &[
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

/// The project root for `path` (a file or directory); see the module docs
/// for the resolution order.
pub fn project_root_for(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() { path } else { path.parent()? };

    let mut explicit: Option<&Path> = None;
    let mut outermost_vcs: Option<&Path> = None;
    let mut nearest_manifest: Option<&Path> = None;

    let mut current = Some(start);
    while let Some(dir) = current {
        if explicit.is_none() && dir.join(".textchum.json").exists() {
            explicit = Some(dir);
        }
        if VCS_MARKERS.iter().any(|m| dir.join(m).exists()) {
            outermost_vcs = Some(dir); // keep climbing: outermost wins
        }
        if nearest_manifest.is_none()
            && MANIFEST_MARKERS.iter().any(|m| dir.join(m).exists())
        {
            nearest_manifest = Some(dir);
        }
        current = dir.parent();
    }

    explicit
        .or(outermost_vcs)
        .or(nearest_manifest)
        .map(Path::to_owned)
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
    fn repository_root_beats_nested_manifests() {
        let outer = scratch("mono");
        std::fs::create_dir_all(outer.join(".git")).unwrap();
        let inner = outer.join("services/api");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("Cargo.toml"), "").unwrap();
        let module = outer.join("pkg/module");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(module.join("pyproject.toml"), "").unwrap();
        let file = inner.join("src/main.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "").unwrap();
        let py = module.join("thing.py");
        std::fs::write(&py, "").unwrap();

        // Everything in the repository is one project, nested manifests
        // or not.
        assert_eq!(project_root_for(&file), Some(outer.clone()));
        assert_eq!(project_root_for(&py), Some(outer.clone()));
        let doc = outer.join("README.md");
        std::fs::write(&doc, "").unwrap();
        assert_eq!(project_root_for(&doc), Some(outer));
    }

    #[test]
    fn outermost_repository_wins_over_nested_ones() {
        let outer = scratch("super");
        std::fs::create_dir_all(outer.join(".git")).unwrap();
        let submodule = outer.join("vendor/dep");
        std::fs::create_dir_all(submodule.join(".git")).unwrap();
        let file = submodule.join("lib.rs");
        std::fs::write(&file, "").unwrap();
        assert_eq!(project_root_for(&file), Some(outer));
    }

    #[test]
    fn manifests_apply_outside_version_control() {
        let root = scratch("no-vcs");
        std::fs::write(root.join("pyproject.toml"), "").unwrap();
        let nested = root.join("src/pkg");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("mod.py");
        std::fs::write(&file, "").unwrap();
        assert_eq!(project_root_for(&file), Some(root));
    }

    #[test]
    fn explicit_marker_beats_the_repository() {
        let repo = scratch("explicit");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let sub = repo.join("special");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(".textchum.json"), "{}").unwrap();
        let file = sub.join("x.rs");
        std::fs::write(&file, "").unwrap();
        assert_eq!(project_root_for(&file), Some(sub));
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
