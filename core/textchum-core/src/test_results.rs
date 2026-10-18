//! The tests a run reported, read from its output: what passed, what
//! failed, what was skipped, grouped by the suite that ran them — and
//! the command that runs again only the ones that failed.
//!
//! Read from the lines the tools print for people, not from a report
//! format asked for on the side, so any command line that runs tests is
//! read the same way, whoever typed it: cargo's test harness (`test x
//! ... ok`), `go test -v` (`--- PASS: TestX`), and pytest, verbose
//! (`file.py::test_x PASSED`) or not (its `FAILED file.py::test_x`
//! summary). Without `-v`, `go test` names only what failed, and so
//! does pytest; the run marks ask for it.

use regex::Regex;
use std::sync::OnceLock;

use crate::runnables::quote;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
    Skipped,
    /// Started and not reported yet; once the run is over, a test left
    /// like this was cut short.
    Running,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::Skipped => "skipped",
            Outcome::Running => "running",
        }
    }
}

/// Which tool printed the results, which decides how they are run again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runner {
    Cargo,
    Go,
    Pytest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    /// What the runner calls it: `tests::adds`, `TestSell/empty`,
    /// `TestShop::test_sells`.
    pub name: String,
    pub outcome: Outcome,
    /// The output line, counted from the first line ever read, that
    /// reported it last.
    pub line: usize,
    /// As the tool printed it, when it did: `0.01s`.
    pub duration: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suite {
    /// The test binary, the Go package or the Python file.
    pub name: String,
    pub tests: Vec<TestResult>,
}

/// The results so far, read a line at a time as lines are finished.
#[derive(Debug, Default)]
pub struct TestLog {
    pub runner: Option<Runner>,
    pub suites: Vec<Suite>,
    /// Go reports a test before saying which package it was in: the
    /// tests read since the last package line, waiting for its name.
    unclaimed: Vec<TestResult>,
}

struct Patterns {
    cargo_suite: Regex,
    cargo_doc: Regex,
    cargo_test: Regex,
    go_run: Regex,
    go_result: Regex,
    go_package: Regex,
    pytest_verbose: Regex,
    pytest_summary: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| Patterns {
        cargo_suite: Regex::new(r"^\s*Running (?:unittests )?(\S+)").unwrap(),
        cargo_doc: Regex::new(r"^\s*Doc-tests (\S+)").unwrap(),
        cargo_test: Regex::new(r"^test (.+?) \.\.\. (ok|FAILED|ignored)\b").unwrap(),
        go_run: Regex::new(r"^\s*=== RUN\s+(\S+)").unwrap(),
        go_result: Regex::new(r"^\s*--- (PASS|FAIL|SKIP): (\S+)(?: \(([\d.]+s)\))?").unwrap(),
        go_package: Regex::new(r"^(ok|FAIL|\?)\s+(\S+)\s").unwrap(),
        pytest_verbose: Regex::new(r"^(\S+?\.py)::(\S+) (PASSED|FAILED|ERROR|SKIPPED|XFAIL|XPASS)\b").unwrap(),
        pytest_summary: Regex::new(r"^(FAILED|ERROR) (\S+?\.py)::(\S+)").unwrap(),
    })
}

impl TestLog {
    /// Reads one finished output line, `index` counted from the first
    /// line ever read.
    pub fn read(&mut self, index: usize, text: &str) {
        let p = patterns();
        if let Some(found) = p.cargo_suite.captures(text).or_else(|| p.cargo_doc.captures(text)) {
            // A path such as `src/lib.rs` or `tests/api.rs`; doc tests
            // are named after the crate.
            let name = if p.cargo_doc.is_match(text) { format!("doc tests: {}", &found[1]) } else { found[1].to_owned() };
            self.suites.push(Suite { name, tests: Vec::new() });
            return;
        }
        if let Some(found) = p.cargo_test.captures(text) {
            let outcome = match &found[2] {
                "ok" => Outcome::Passed,
                "FAILED" => Outcome::Failed,
                _ => Outcome::Skipped,
            };
            self.runner = Some(Runner::Cargo);
            self.record("", found[1].to_owned(), outcome, index, None);
            return;
        }
        if let Some(found) = p.go_run.captures(text) {
            self.runner = Some(Runner::Go);
            self.record_go(found[1].to_owned(), Outcome::Running, index, None);
            return;
        }
        if let Some(found) = p.go_result.captures(text) {
            let outcome = match &found[1] {
                "PASS" => Outcome::Passed,
                "FAIL" => Outcome::Failed,
                _ => Outcome::Skipped,
            };
            self.runner = Some(Runner::Go);
            self.record_go(found[2].to_owned(), outcome, index, found.get(3).map(|d| d.as_str().to_owned()));
            return;
        }
        if self.runner == Some(Runner::Go) {
            if let Some(found) = p.go_package.captures(text) {
                let tests = std::mem::take(&mut self.unclaimed);
                if !tests.is_empty() {
                    self.suites.push(Suite { name: found[2].to_owned(), tests });
                }
                return;
            }
        }
        if let Some(found) = p.pytest_verbose.captures(text) {
            let outcome = match &found[3] {
                "PASSED" | "XFAIL" => Outcome::Passed,
                "FAILED" | "ERROR" | "XPASS" => Outcome::Failed,
                _ => Outcome::Skipped,
            };
            self.runner = Some(Runner::Pytest);
            self.record(&found[1], found[2].to_owned(), outcome, index, None);
            return;
        }
        if let Some(found) = p.pytest_summary.captures(text) {
            // The summary at the end repeats what verbose output already
            // said; what it adds is the failures a quiet run named only here.
            let (file, name) = (found[2].to_owned(), found[3].to_owned());
            let known = self
                .suites
                .iter()
                .any(|suite| suite.name == file && suite.tests.iter().any(|test| test.name == name));
            if !known {
                self.runner = Some(Runner::Pytest);
                self.record(&file, name, Outcome::Failed, index, None);
            }
        }
    }

    /// A result for `suite` ("" for the one being read): a test reported
    /// again — pytest's ERROR in teardown after PASSED — keeps the worse.
    fn record(&mut self, suite: &str, name: String, outcome: Outcome, line: usize, duration: Option<String>) {
        let at = match self.suites.iter().rposition(|candidate| suite.is_empty() || candidate.name == suite) {
            Some(at) => at,
            None => {
                self.suites.push(Suite { name: suite.to_owned(), tests: Vec::new() });
                self.suites.len() - 1
            }
        };
        let tests = &mut self.suites[at].tests;
        match tests.iter_mut().find(|test| test.name == name) {
            Some(test) if outcome == Outcome::Failed || test.outcome == Outcome::Running => {
                test.outcome = outcome;
                test.line = line;
            }
            Some(_) => {}
            None => tests.push(TestResult { name, outcome, line, duration }),
        }
    }

    fn record_go(&mut self, name: String, outcome: Outcome, line: usize, duration: Option<String>) {
        match self.unclaimed.iter_mut().find(|test| test.name == name) {
            Some(test) => {
                test.outcome = outcome;
                test.line = line;
                test.duration = duration;
            }
            None => self.unclaimed.push(TestResult { name, outcome, line, duration }),
        }
    }

    /// The suites as they stand, Go's tests still waiting for their
    /// package under one with no name.
    pub fn suites(&self) -> Vec<Suite> {
        let mut suites: Vec<Suite> = self.suites.iter().filter(|suite| !suite.tests.is_empty()).cloned().collect();
        if !self.unclaimed.is_empty() {
            suites.push(Suite { name: String::new(), tests: self.unclaimed.clone() });
        }
        suites
    }

    pub fn failed(&self) -> Vec<(String, String)> {
        self.suites()
            .into_iter()
            .flat_map(|suite| {
                suite
                    .tests
                    .into_iter()
                    .filter(|test| test.outcome == Outcome::Failed)
                    .map(move |test| (suite.name.clone(), test.name))
            })
            .collect()
    }
}

/// Splits a command line into words the way the shell would, for the
/// plain command lines tests are run with; None for anything with a
/// pipe, a list, a redirection or a substitution, which is not taken
/// apart.
fn words(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut characters = command.chars();
    while let Some(c) = characters.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match characters.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match characters.next()? {
                        '"' => break,
                        '\\' => word.push(characters.next()?),
                        '$' | '`' => return None,
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(characters.next()?);
            }
            '|' | '&' | ';' | '<' | '>' | '$' | '`' | '(' | ')' => return None,
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

fn join(words: &[String]) -> String {
    words.iter().map(|word| quote(word)).collect::<Vec<_>>().join(" ")
}

/// Options of `cargo test` that take a value as the next word.
const CARGO_VALUED: [&str; 14] = [
    "-p", "--package", "--bin", "--test", "--example", "--bench", "-F", "--features", "--manifest-path", "-j",
    "--jobs", "--target", "--profile", "--exclude",
];

/// Options of `go test` that take a value as the next word, the ones
/// that choose tests dropped later.
const GO_VALUED: [&str; 12] =
    ["-run", "-bench", "-skip", "-count", "-timeout", "-tags", "-parallel", "-cpu", "-p", "-benchtime", "-o", "-exec"];

/// Options of pytest that take a value as the next word.
const PYTEST_VALUED: [&str; 10] = ["-k", "-m", "-p", "-c", "-o", "--rootdir", "--maxfail", "-W", "--tb", "--deselect"];

/// The command that runs again only `failed` — (suite, test) pairs —
/// made from the `command` that ran them; None when nothing failed or
/// the command is not one this knows how to narrow.
pub fn rerun_failed(command: &str, runner: Option<Runner>, failed: &[(String, String)]) -> Option<String> {
    if failed.is_empty() {
        return None;
    }
    let words = words(command)?;
    match runner? {
        Runner::Cargo => {
            let at = words.windows(2).position(|pair| pair[0] == "cargo" && pair[1] == "test")?;
            // What comes before the test harness's own `--`, without the
            // name filters: those are what is replaced.
            let mut kept: Vec<String> = words[..at + 2].to_vec();
            let mut rest = words[at + 2..].iter();
            while let Some(word) = rest.next() {
                if word == "--" {
                    break;
                }
                if word.starts_with('-') {
                    kept.push(word.clone());
                    if CARGO_VALUED.contains(&word.as_str()) {
                        kept.extend(rest.next().cloned());
                    }
                }
            }
            let mut names: Vec<String> = Vec::new();
            for (_, name) in failed {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            kept.push("--".to_owned());
            kept.extend(names);
            kept.push("--exact".to_owned());
            Some(join(&kept))
        }
        Runner::Go => {
            let at = words.windows(2).position(|pair| pair[0] == "go" && pair[1] == "test")?;
            let mut kept: Vec<String> = words[..at + 2].to_vec();
            let mut rest = words[at + 2..].iter();
            while let Some(word) = rest.next() {
                let flag = word.split('=').next().unwrap_or(word);
                let dropped = ["-run", "-bench", "-skip", "--run", "--bench", "--skip"].contains(&flag);
                let valued = GO_VALUED.contains(&word.as_str());
                if !dropped {
                    kept.push(word.clone());
                }
                if valued {
                    let value = rest.next().cloned();
                    if !dropped {
                        kept.extend(value);
                    }
                }
            }
            // A failed subtest fails its parent: run the parents again,
            // which runs the subtests with them.
            let mut names: Vec<String> = Vec::new();
            for (_, name) in failed {
                let top = name.split('/').next().unwrap_or(name).to_owned();
                if !names.contains(&top) {
                    names.push(top);
                }
            }
            let pattern = format!("^({})$", names.join("|"));
            kept.insert(at + 2, "-run".to_owned());
            kept.insert(at + 3, pattern);
            Some(join(&kept))
        }
        Runner::Pytest => {
            let at = words.iter().position(|word| word == "pytest" || word.ends_with("/pytest"))?;
            let mut kept: Vec<String> = words[..=at].to_vec();
            let mut rest = words[at + 1..].iter();
            while let Some(word) = rest.next() {
                if word.starts_with('-') && word != "-k" {
                    kept.push(word.clone());
                    if PYTEST_VALUED.contains(&word.as_str()) {
                        kept.extend(rest.next().cloned());
                    }
                } else if word == "-k" {
                    // A keyword filter would narrow the ids given again.
                    rest.next();
                }
            }
            for (file, name) in failed {
                let id = format!("{file}::{name}");
                if !kept.contains(&id) {
                    kept.push(id);
                }
            }
            Some(join(&kept))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(output: &str) -> TestLog {
        let mut log = TestLog::default();
        for (index, line) in output.lines().enumerate() {
            log.read(index, line);
        }
        log
    }

    fn summary(log: &TestLog) -> Vec<(String, Vec<(String, &'static str)>)> {
        log.suites()
            .into_iter()
            .map(|suite| {
                (suite.name, suite.tests.into_iter().map(|test| (test.name, test.outcome.name())).collect())
            })
            .collect()
    }

    #[test]
    fn cargo_tests_are_grouped_by_the_binary_that_ran_them() {
        let log = log("   Compiling shop v0.1.0\n     Running unittests src/lib.rs (target/debug/deps/shop-1)\n\nrunning 2 tests\ntest tests::adds ... ok\ntest tests::sells ... FAILED\n\n     Running tests/api.rs (target/debug/deps/api-2)\ntest serves ... ignored, slow\n   Doc-tests shop\ntest src/lib.rs - add (line 3) ... ok\n");
        assert_eq!(
            summary(&log),
            [
                ("src/lib.rs".to_owned(), vec![("tests::adds".to_owned(), "passed"), ("tests::sells".to_owned(), "failed")]),
                ("tests/api.rs".to_owned(), vec![("serves".to_owned(), "skipped")]),
                ("doc tests: shop".to_owned(), vec![("src/lib.rs - add (line 3)".to_owned(), "passed")]),
            ]
        );
        assert_eq!(log.suites()[0].tests[1].line, 5);
        assert_eq!(
            rerun_failed("cargo test --bin shop tests", log.runner, &log.failed()).as_deref(),
            Some("cargo test --bin shop -- tests::sells --exact")
        );
    }

    #[test]
    fn go_tests_wait_for_their_package_and_subtests_rerun_by_parent() {
        let log = log("=== RUN   TestSell\n=== RUN   TestSell/empty\n    --- FAIL: TestSell/empty (0.00s)\n--- FAIL: TestSell (0.01s)\n=== RUN   TestBuy\n--- PASS: TestBuy (0.00s)\nFAIL\nFAIL\texample.com/shop\t0.012s\n=== RUN   TestSlow\n");
        assert_eq!(
            summary(&log),
            [
                (
                    "example.com/shop".to_owned(),
                    vec![
                        ("TestSell".to_owned(), "failed"),
                        ("TestSell/empty".to_owned(), "failed"),
                        ("TestBuy".to_owned(), "passed")
                    ]
                ),
                (String::new(), vec![("TestSlow".to_owned(), "running")]),
            ]
        );
        assert_eq!(log.suites()[0].tests[0].duration.as_deref(), Some("0.01s"));
        assert_eq!(
            rerun_failed("go test -v -run '^TestSell$' -count=1 ./...", log.runner, &log.failed()).as_deref(),
            Some("go test -run '^(TestSell)$' -v -count=1 ./...")
        );
    }

    #[test]
    fn pytest_verbose_lines_and_the_quiet_summary_both_count_once() {
        let log = log("tests/test_shop.py::test_adds PASSED                     [ 33%]\ntests/test_shop.py::TestShop::test_sells FAILED          [ 66%]\ntests/test_shop.py::test_skip SKIPPED (no db)           [100%]\n=========================== short test summary info ============================\nFAILED tests/test_shop.py::TestShop::test_sells - assert 1 == 2\nFAILED tests/test_cart.py::test_total - KeyError\n");
        assert_eq!(
            summary(&log),
            [
                (
                    "tests/test_shop.py".to_owned(),
                    vec![
                        ("test_adds".to_owned(), "passed"),
                        ("TestShop::test_sells".to_owned(), "failed"),
                        ("test_skip".to_owned(), "skipped")
                    ]
                ),
                ("tests/test_cart.py".to_owned(), vec![("test_total".to_owned(), "failed")]),
            ]
        );
        assert_eq!(
            rerun_failed("python3 -m pytest -v -k shop tests", log.runner, &log.failed()).as_deref(),
            Some("python3 -m pytest -v tests/test_shop.py::TestShop::test_sells tests/test_cart.py::test_total")
        );
    }

    #[test]
    fn a_command_line_with_more_than_words_is_not_taken_apart() {
        let failed = [(String::new(), "adds".to_owned())];
        assert_eq!(rerun_failed("cargo test && echo done", Some(Runner::Cargo), &failed), None);
        assert_eq!(rerun_failed("make test", Some(Runner::Cargo), &failed), None);
        assert_eq!(rerun_failed("cargo test", Some(Runner::Cargo), &[]), None);
        assert_eq!(words("a 'b c' \"d\\\"e\" f\\ g"), Some(vec!["a".into(), "b c".into(), "d\"e".into(), "f g".into()]));
    }
}
