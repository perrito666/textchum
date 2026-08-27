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

/// User-configurable workspace behavior, parsed from the configuration's
/// `workspace` section: `{"manifest_projects": bool, "recursive_config":
/// bool, "projects": {root: {same flags}}}`. Missing flags default to
/// false.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceSettings {
    parsed: serde_json::Value,
}

impl WorkspaceSettings {
    pub fn from_json(json: &str) -> Self {
        Self {
            parsed: serde_json::from_str(json).unwrap_or(serde_json::Value::Null),
        }
    }

    /// A named boolean: the project's own entry when present, else the
    /// top-level default, else false. Public because shells resolve their
    /// own flags (like the ctags fallback) with the same rules.
    pub fn flag(&self, root: &Path, key: &str) -> bool {
        let per_project =
            self.parsed["projects"][root.to_string_lossy().as_ref()][key].as_bool();
        per_project.unwrap_or_else(|| self.parsed[key].as_bool().unwrap_or(false))
    }

    /// Whether nested language manifests split `root` (a repository) into
    /// sub-projects, restoring nearest-manifest behavior inside it.
    pub fn manifest_projects(&self, root: &Path) -> bool {
        self.flag(root, "manifest_projects")
    }

    /// Whether per-project configuration attached to `root` cascades into
    /// the nested projects beneath it.
    pub fn recursive_config(&self, root: &Path) -> bool {
        self.flag(root, "recursive_config")
    }
}

/// The project root for `path` with default settings; see
/// [`project_root_with`].
pub fn project_root_for(path: &Path) -> Option<PathBuf> {
    project_root_with(path, &WorkspaceSettings::default())
}

/// The project root for `path` (a file or directory); see the module docs
/// for the resolution order. When the settings enable `manifest_projects`
/// for the enclosing repository, nested manifests split it into
/// sub-projects (nearest manifest wins within the repository).
pub fn project_root_with(path: &Path, settings: &WorkspaceSettings) -> Option<PathBuf> {
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

    if let Some(explicit) = explicit {
        return Some(explicit.to_owned());
    }
    if let Some(repo) = outermost_vcs {
        // Inside a repository, manifests only matter when the user opted
        // this repository (or everything) into manifest-based projects.
        if settings.manifest_projects(repo) {
            if let Some(manifest) = nearest_manifest {
                if manifest.starts_with(repo) {
                    return Some(manifest.to_owned());
                }
            }
        }
        return Some(repo.to_owned());
    }
    nearest_manifest.map(Path::to_owned)
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
    fn manifest_projects_toggle_restores_nested_projects() {
        let repo = scratch("toggle");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let module = repo.join("pkg/module");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(module.join("pyproject.toml"), "").unwrap();
        let file = module.join("thing.py");
        std::fs::write(&file, "").unwrap();

        // Off (default): the repository is one project.
        assert_eq!(project_root_for(&file), Some(repo.clone()));

        // Per-repository toggle: nested manifests split again.
        let per_repo = WorkspaceSettings::from_json(&format!(
            r#"{{"projects": {{"{}": {{"manifest_projects": true}}}}}}"#,
            repo.display()
        ));
        assert_eq!(project_root_with(&file, &per_repo), Some(module.clone()));

        // Global default toggle works too, and a per-repo false wins over
        // a global true.
        let global = WorkspaceSettings::from_json(r#"{"manifest_projects": true}"#);
        assert_eq!(project_root_with(&file, &global), Some(module.clone()));
        let overridden = WorkspaceSettings::from_json(&format!(
            r#"{{"manifest_projects": true,
                 "projects": {{"{}": {{"manifest_projects": false}}}}}}"#,
            repo.display()
        ));
        assert_eq!(project_root_with(&file, &overridden), Some(repo));
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

/// Whether `name` matches the shell-style glob `pattern` (`*` any run,
/// `?` any one character; everything else literal). Matching is over
/// the bare file name, the way navigator hiding uses it.
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    fn matches(pattern: &[char], name: &[char]) -> bool {
        match (pattern.first(), name.first()) {
            (None, None) => true,
            (Some('*'), _) => {
                matches(&pattern[1..], name)
                    || (!name.is_empty() && matches(pattern, &name[1..]))
            }
            (Some('?'), Some(_)) => matches(&pattern[1..], &name[1..]),
            (Some(p), Some(n)) if p == n => matches(&pattern[1..], &name[1..]),
            _ => false,
        }
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    matches(&pattern, &name)
}

/// Whether any of `globs` hides `name`.
pub fn is_hidden(name: &str, globs: &[String]) -> bool {
    globs.iter().any(|glob| glob_matches(glob, name))
}

#[cfg(test)]
mod glob_tests {
    use super::*;

    #[test]
    fn globs_match_like_a_shell() {
        assert!(glob_matches(".*", ".git"));
        assert!(!glob_matches(".*", "src"));
        assert!(glob_matches("*.pyc", "module.pyc"));
        assert!(!glob_matches("*.pyc", "module.py"));
        assert!(glob_matches("node_modules", "node_modules"));
        assert!(glob_matches("?ar", "bar"));
        assert!(!glob_matches("?ar", "bazar"));
        assert!(is_hidden("target", &["*.o".into(), "target".into()]));
        assert!(!is_hidden("main.rs", &["*.o".into(), "target".into()]));
    }
}
