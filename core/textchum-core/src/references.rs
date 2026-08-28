//! Telling a test apart from the code it tests.
//!
//! Ask where a function is used and the answer is usually dominated by
//! its test file: twenty references, sixteen of them assertions, four
//! of them the callers you were looking for, and the four scattered
//! among the sixteen because the list is sorted by path.
//!
//! There is no fact of the matter about which files are tests — no
//! language marks them, and the language server does not say. There are
//! only conventions, and they are strong ones: a directory called
//! `tests`, a file called `foo_test.go`. So this is a heuristic, and it
//! is written to be a cautious one. `latest.rs` is not a test.

use std::path::Path;

/// Directory names that make everything under them a test.
const TEST_DIRECTORIES: &[&str] = &[
    "__tests__",
    "spec",
    "specs",
    "test",
    "testdata",
    "tests",
];

/// Stem suffixes that make a file a test, matched case-sensitively so
/// `latest` and `protest` are left alone: the convention is a separator
/// (`foo_test`) or a capital (`FooTest`).
const TEST_SUFFIXES: &[&str] = &[
    "-spec", "-test", "Spec", "Specs", "Test", "Tests", "_spec", "_test", "_tests",
];

/// Stem prefixes that make a file a test.
const TEST_PREFIXES: &[&str] = &["test_"];

/// Whether `path` looks like a test by the conventions of the languages
/// this editor knows.
///
/// A path decides; the contents are never read. Rust's `#[cfg(test)]
/// mod tests` inside an ordinary file is therefore counted as code,
/// which is what its path says — reading and parsing every file a
/// reference landed in would be a great deal of work for a list.
pub fn is_test_path(path: &str) -> bool {
    let path = Path::new(path);
    let in_test_directory = path
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|component| component.as_os_str().to_str())
        .any(|name| {
            let lowered = name.to_lowercase();
            TEST_DIRECTORIES.contains(&lowered.as_str())
        });
    if in_test_directory {
        return true;
    }

    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    // `button.test.ts` and `button.spec.js`: the marker sits between
    // the name and the extension rather than at either end.
    let mut parts = name.split('.');
    let stem = parts.next().unwrap_or("");
    let middle: Vec<&str> = parts.collect();
    if middle.len() > 1 && matches!(middle[0], "test" | "spec") {
        return true;
    }
    TEST_PREFIXES.iter().any(|prefix| stem.starts_with(prefix))
        || TEST_SUFFIXES.iter().any(|suffix| stem.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_named_for_tests_makes_everything_under_it_one() {
        assert!(is_test_path("/p/tests/helpers.rs"));
        assert!(is_test_path("/p/test/deep/inside/thing.py"));
        assert!(is_test_path("/p/spec/models/user_spec.rb"));
        assert!(is_test_path("/p/src/__tests__/Button.tsx"));
        assert!(is_test_path("/p/testdata/fixture.json"));
        // Case is not the convention's point.
        assert!(is_test_path("/p/Tests/AppTests.swift"));
    }

    #[test]
    fn file_name_conventions_per_language() {
        assert!(is_test_path("/p/src/parser_test.go"));
        assert!(is_test_path("/p/src/parser_test.rs"));
        assert!(is_test_path("/p/src/test_parser.py"));
        assert!(is_test_path("/p/src/parser_test.py"));
        assert!(is_test_path("/p/src/Button.test.ts"));
        assert!(is_test_path("/p/src/Button.spec.tsx"));
        assert!(is_test_path("/p/src/ParserTest.java"));
        assert!(is_test_path("/p/src/ParserTests.swift"));
        assert!(is_test_path("/p/src/user_spec.rb"));
    }

    #[test]
    fn ordinary_files_are_not_tests_however_they_are_spelled() {
        assert!(!is_test_path("/p/src/main.rs"));
        // The trap this heuristic exists to avoid.
        assert!(!is_test_path("/p/src/latest.rs"));
        assert!(!is_test_path("/p/src/protest.go"));
        assert!(!is_test_path("/p/src/contest.py"));
        assert!(!is_test_path("/p/src/manifest.json"));
        assert!(!is_test_path("/p/src/attest.c"));
        // A directory whose name merely contains one of the words.
        assert!(!is_test_path("/p/testing-library/index.js"));
        assert!(!is_test_path("/p/latest/main.rs"));
        // The file itself named `test` is a directory's job, not a
        // suffix's — but it is still a test.
        assert!(is_test_path("/p/src/foo_test.cc"));
    }

    #[test]
    fn a_directory_named_for_tests_beats_the_file_name() {
        // The file says nothing; the directory says everything.
        assert!(is_test_path("/p/tests/main.rs"));
    }

    #[test]
    fn nothing_that_is_not_a_path_is_a_test() {
        assert!(!is_test_path(""));
        assert!(!is_test_path("/"));
    }
}
