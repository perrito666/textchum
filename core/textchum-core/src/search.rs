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

/// What a [`Filter`] applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    /// The matching line's text.
    Line,
    /// The hit's relative file path.
    File,
}

/// A stacked refinement over grep results: hits survive only if the
/// filtered value does (include) or does not (exclude) contain `pattern`
/// (case-insensitive substring). File excludes prune whole files before
/// they are searched, so filtered searches stay as fast as plain ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub kind: FilterKind,
    /// true = the value must contain the pattern; false = must not.
    pub include: bool,
    pub pattern: String,
}

impl Filter {
    fn passes(&self, value: &str) -> bool {
        let contains = value.to_lowercase().contains(&self.pattern.to_lowercase());
        contains == self.include
    }
}

/// The project's file list (ignore-aware, relative paths), for callers
/// that want to walk once and match many times — the shape Open
/// Quickly needs, since re-walking a real repository on every
/// keystroke is what makes a fuzzy finder feel broken.
pub fn list_files(root: &Path) -> Vec<String> {
    walk(root)
}

/// Fuzzy-matches an already-walked list, best first. An empty query
/// lists alphabetically, like [`fuzzy_files`].
pub fn match_files(paths: &[String], query: &str, limit: usize) -> Vec<String> {
    if query.trim().is_empty() {
        let mut sorted = paths.to_vec();
        sorted.sort();
        sorted.truncate(limit);
        return sorted;
    }
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut scored = pattern.match_list(paths.to_vec(), &mut matcher);
    scored.truncate(limit);
    scored.into_iter().map(|(path, _)| path).collect()
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
    match_files(&walk(root), query, limit)
}

/// Searches file contents under `root` for `pattern` (a regex), returning
/// up to `limit` hits. Binary files quit at the first NUL; unreadable
/// files are skipped. Errors are bad patterns, phrased for humans.
/// What a search actually did, so an empty result can explain itself:
/// "no matches in 4,000 files" is a query problem, "0 files" is a scope
/// or permission problem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchStats {
    /// Files the walker offered (before filters).
    pub files_seen: usize,
    /// Files actually opened and searched.
    pub files_searched: usize,
    /// Entries the walker could not read (permissions, broken links).
    pub errors: usize,
}

/// [`grep`] with the counts of what was walked and read.
pub fn grep_with_stats(
    root: &Path,
    pattern: &str,
    case_insensitive: bool,
    limit: usize,
    filters: &[Filter],
) -> Result<(Vec<SearchHit>, SearchStats), String> {
    let mut stats = SearchStats::default();
    let hits = grep_inner(root, pattern, case_insensitive, limit, filters, &mut stats)?;
    Ok((hits, stats))
}

pub fn grep(
    root: &Path,
    pattern: &str,
    case_insensitive: bool,
    limit: usize,
    filters: &[Filter],
) -> Result<Vec<SearchHit>, String> {
    let mut stats = SearchStats::default();
    grep_inner(root, pattern, case_insensitive, limit, filters, &mut stats)
}

fn grep_inner(
    root: &Path,
    pattern: &str,
    case_insensitive: bool,
    limit: usize,
    filters: &[Filter],
    stats: &mut SearchStats,
) -> Result<Vec<SearchHit>, String> {
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .case_insensitive(case_insensitive)
        .build(pattern)
        .map_err(|e| format!("bad pattern: {e}"))?;
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .line_number(true)
        .build();

    let (file_filters, line_filters): (Vec<&Filter>, Vec<&Filter>) = {
        let mut files = Vec::new();
        let mut lines = Vec::new();
        for filter in filters {
            match filter.kind {
                FilterKind::File => files.push(filter),
                FilterKind::Line => lines.push(filter),
            }
        }
        (files, lines)
    };

    let mut hits = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .max_filesize(Some(MAX_FILE_SIZE))
        .build()
    {
        if hits.len() >= limit {
            break;
        }
        // Unreadable entries are counted rather than silently dropped:
        // a scope that yields nothing but errors is a permissions
        // problem, and the caller can say so.
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                stats.errors += 1;
                continue;
            }
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        stats.files_seen += 1;
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().into_owned();
        // File filters prune before the file is even opened.
        if !file_filters.iter().all(|f| f.passes(&relative)) {
            continue;
        }
        stats.files_searched += 1;
        let _ = searcher.search_path(
            &matcher,
            entry.path(),
            UTF8(|line, text| {
                let text = text.trim_end();
                if line_filters.iter().all(|f| f.passes(text)) {
                    hits.push(SearchHit {
                        path: relative.clone(),
                        line,
                        text: text.chars().take(400).collect(),
                    });
                }
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

        // Walking once and matching many times is the same answer as
        // walking per query — the whole point of the split.
        let listed = list_files(&root);
        assert_eq!(match_files(&listed, "mainrs", 10), fuzzy_files(&root, "mainrs", 10));
        assert_eq!(match_files(&listed, "", 10), all);
    }

    #[test]
    fn grep_finds_lines_with_numbers_and_respects_ignores() {
        let root = project("grep_finds_lines_with_numbers_and_respects_ignores");
        let hits = grep(&root, "needle", false, 10, &[]).unwrap();
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
        assert!(grep(&root, "NEEDLE", false, 10, &[]).unwrap().is_empty());
        assert_eq!(grep(&root, "NEEDLE", true, 10, &[]).unwrap().len(), 2);
        assert_eq!(grep(&root, "needle", true, 1, &[]).unwrap().len(), 1);
    }

    #[test]
    fn stacked_filters_narrow_lines_and_files() {
        let root = project("stacked_filters_narrow_lines_and_files");
        std::fs::write(
            root.join("src/lib.rs"),
            "foo alone\nfoo with bar\nbar only\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib_test.rs"), "foo with bar in tests\n").unwrap();

        // The canonical stack: lines with foo where bar also appears,
        // excluding files with "test" in the name.
        let filters = [
            Filter {
                kind: FilterKind::Line,
                include: true,
                pattern: "bar".into(),
            },
            Filter {
                kind: FilterKind::File,
                include: false,
                pattern: "test".into(),
            },
        ];
        let hits = grep(&root, "foo", false, 50, &filters).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].path, "src/lib.rs");
        assert_eq!(hits[0].text, "foo with bar");

        // Filters are case-insensitive substrings.
        let case = [Filter {
            kind: FilterKind::Line,
            include: true,
            pattern: "BAR".into(),
        }];
        assert_eq!(grep(&root, "foo", false, 50, &case).unwrap().len(), 2);

        // File include narrows to matching paths.
        let only_tests = [Filter {
            kind: FilterKind::File,
            include: true,
            pattern: "test".into(),
        }];
        let hits = grep(&root, "foo", false, 50, &only_tests).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "src/lib_test.rs");
    }

    #[test]
    fn grep_rejects_bad_patterns_gracefully() {
        let root = project("grep_rejects_bad_patterns_gracefully");
        let error = grep(&root, "unclosed(", false, 10, &[]).unwrap_err();
        assert!(error.contains("bad pattern"), "got: {error}");
    }

    #[test]
    fn stats_separate_no_matches_from_nothing_searched() {
        let root = project("stats_separate_no_matches_from_nothing_searched");
        // A query that matches nothing still reports the files it read.
        let (hits, stats) =
            grep_with_stats(&root, "zzz-not-here-zzz", false, 10, &[]).unwrap();
        assert!(hits.is_empty());
        assert!(stats.files_searched > 0, "files were searched: {stats:?}");
        assert_eq!(stats.files_seen, stats.files_searched, "no filters, no pruning");

        // An empty scope reads nothing at all — the case that used to be
        // indistinguishable from "no matches".
        let empty = root.join("empty-dir");
        std::fs::create_dir_all(&empty).unwrap();
        let (hits, stats) = grep_with_stats(&empty, "anything", false, 10, &[]).unwrap();
        assert!(hits.is_empty());
        assert_eq!(stats.files_searched, 0, "nothing to search: {stats:?}");

        // File filters prune before opening, which the counts show.
        let filters = [Filter {
            kind: FilterKind::File,
            include: true,
            pattern: "no-such-name".into(),
        }];
        let (_, stats) = grep_with_stats(&root, "fn", false, 10, &filters).unwrap();
        assert!(stats.files_seen > 0 && stats.files_searched == 0, "{stats:?}");
    }
}
