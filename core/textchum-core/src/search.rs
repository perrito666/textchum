//! Project search: fuzzy file finding and content grep.
//!
//! Both share one principle with the UI that calls them: **the scope is an
//! explicit root path**, never an implicit "wherever". The walk comes from
//! ripgrep's `ignore` crate (gitignore-aware, hidden files skipped, size
//! capped), content matching from ripgrep's `grep-*` crates, and fuzzy
//! scoring from `nucleo` — fzf's spirit, in process.
//!
//! Unlike the rest of the core these are pure functions over the file
//! system: no handles, no shared state. Shells may call them from any
//! thread, and should call them off the UI thread for large scopes.

use std::path::Path;

use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

/// Files larger than this are not searched or listed.
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
/// Hard cap on walked files, so a mistyped scope ("/") stays bounded.
const MAX_WALK: usize = 100_000;

/// One content-search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Path relative to the searched root.
    pub path: String,
    /// One-based line number.
    pub line: u64,
    /// The matching line, trimmed.
    pub text: String,
}

/// Walks `root` (ignore-aware) collecting relative file paths.
fn walk(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .max_filesize(Some(MAX_FILE_SIZE))
        .build()
        .flatten()
    {
        if paths.len() >= MAX_WALK {
            break;
        }
        if entry.file_type().is_some_and(|t| t.is_file()) {
            if let Ok(relative) = entry.path().strip_prefix(root) {
                paths.push(relative.to_string_lossy().into_owned());
            }
        }
    }
    paths
}

/// Fuzzy-matches file paths under `root` against `query`, best first.
/// An empty query lists files alphabetically instead.
pub fn fuzzy_files(root: &Path, query: &str, limit: usize) -> Vec<String> {
    let mut paths = walk(root);
    if query.trim().is_empty() {
        paths.sort();
        paths.truncate(limit);
        return paths;
    }
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut scored = pattern.match_list(paths, &mut matcher);
    scored.truncate(limit);
    scored.into_iter().map(|(path, _)| path).collect()
}

/// Searches file contents under `root` for `pattern` (a regex), returning
/// up to `limit` hits. Binary files quit at the first NUL; unreadable
/// files are skipped. Errors are bad patterns, phrased for humans.
pub fn grep(
    root: &Path,
    pattern: &str,
    case_insensitive: bool,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .case_insensitive(case_insensitive)
        .build(pattern)
        .map_err(|e| format!("bad pattern: {e}"))?;
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .line_number(true)
        .build();

    let mut hits = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .max_filesize(Some(MAX_FILE_SIZE))
        .build()
        .flatten()
    {
        if hits.len() >= limit {
            break;
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().into_owned();
        let _ = searcher.search_path(
            &matcher,
            entry.path(),
            UTF8(|line, text| {
                hits.push(SearchHit {
                    path: relative.clone(),
                    line,
                    text: text.trim_end().chars().take(400).collect(),
                });
                Ok(hits.len() < limit)
            }),
        );
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn project(name: &str) -> PathBuf {
        // One directory per test: tests run in parallel, and the
        // delete-then-create setup must never race a sibling's reads.
        let root = std::env::temp_dir()
            .join(format!("textchum-search-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() { needle(); }\n").unwrap();
        std::fs::write(root.join("src/deep/util.rs"), "// no such thing\n").unwrap();
        std::fs::write(root.join("README.md"), "a needle in the docs\n").unwrap();
        std::fs::write(root.join("target/generated.rs"), "needle\n").unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        // .gitignore applies to tracked trees; the walker respects it even
        // without a .git directory when parents opt in — create one.
        std::fs::create_dir_all(root.join(".git")).unwrap();
        root
    }

    #[test]
    fn fuzzy_ranks_and_respects_ignores() {
        let root = project("fuzzy_ranks_and_respects_ignores");
        let results = fuzzy_files(&root, "mainrs", 10);
        assert_eq!(results.first().map(String::as_str), Some("src/main.rs"));
        assert!(
            !results.iter().any(|p| p.starts_with("target/")),
            "gitignored files must not appear: {results:?}"
        );

        let all = fuzzy_files(&root, "", 10);
        assert!(all.contains(&"README.md".to_owned()));
        assert!(!all.iter().any(|p| p.starts_with("target/")));
    }

    #[test]
    fn grep_finds_lines_with_numbers_and_respects_ignores() {
        let root = project("grep_finds_lines_with_numbers_and_respects_ignores");
        let hits = grep(&root, "needle", false, 10).unwrap();
        let mut paths: Vec<_> = hits.iter().map(|h| h.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, ["README.md", "src/main.rs"]);
        let main_hit = hits.iter().find(|h| h.path == "src/main.rs").unwrap();
        assert_eq!(main_hit.line, 1);
        assert!(main_hit.text.contains("needle()"));
    }

    #[test]
    fn grep_case_flag_and_limit() {
        let root = project("grep_case_flag_and_limit");
        assert!(grep(&root, "NEEDLE", false, 10).unwrap().is_empty());
        assert_eq!(grep(&root, "NEEDLE", true, 10).unwrap().len(), 2);
        assert_eq!(grep(&root, "needle", true, 1).unwrap().len(), 1);
    }

    #[test]
    fn grep_rejects_bad_patterns_gracefully() {
        let root = project("grep_rejects_bad_patterns_gracefully");
        let error = grep(&root, "unclosed(", false, 10).unwrap_err();
        assert!(error.contains("bad pattern"), "got: {error}");
    }
}
