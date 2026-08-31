//! Which lines differ from the file as it stands in git.
//!
//! Reading a file under edit, the gutter can say which lines are yours
//! and which were already there. That needs two things: the committed
//! text, and a line diff against what is in the buffer now.
//!
//! Both live here rather than in the shells. Two shells asking git the
//! same question separately is two answers that will differ eventually,
//! and the diff is the kind of code that is worth having one of.

use std::path::Path;
use std::process::Command;

/// What happened to a line, as a gutter draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Not in the committed file.
    Added,
    /// In it, but reading differently.
    Modified,
    /// Lines were here and are gone. Nothing occupies their place, so
    /// the mark belongs on the boundary above `line`.
    Removed,
}

impl ChangeKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Removed => "removed",
        }
    }
}

/// One mark, on a zero-based line of the current text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineChange {
    pub line: usize,
    pub kind: ChangeKind,
}

/// Above this many lines on either side, the diff is not attempted: a
/// gutter is an orientation aid, and one that costs a visible pause on
/// a generated file has stopped being one.
pub const MAX_LINES: usize = 50_000;

/// The marks for `current` against `baseline`.
///
/// A run where lines were replaced reads as modified for as far as the
/// two runs overlap, with the remainder added or removed — which is how
/// a gutter shows an edited paragraph rather than claiming the whole
/// thing is new.
pub fn line_changes(baseline: &str, current: &str) -> Vec<LineChange> {
    let old: Vec<&str> = baseline.lines().collect();
    let new: Vec<&str> = current.lines().collect();
    if old.len() > MAX_LINES || new.len() > MAX_LINES {
        return Vec::new();
    }

    // Matching heads and tails are the bulk of any edited file, and
    // cost nothing to skip.
    let mut head = 0;
    while head < old.len() && head < new.len() && old[head] == new[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < old.len() - head && tail < new.len() - head
        && old[old.len() - 1 - tail] == new[new.len() - 1 - tail]
    {
        tail += 1;
    }
    let old_middle = &old[head..old.len() - tail];
    let new_middle = &new[head..new.len() - tail];

    let mut changes = Vec::new();
    for (old_run, new_run) in diff_runs(old_middle, new_middle) {
        emit(&mut changes, head, old_run, new_run);
    }
    changes
}

/// A run of the middle: the old lines removed and the new lines added
/// at one place, as `(old range, new range)` in middle coordinates.
type Run = (std::ops::Range<usize>, std::ops::Range<usize>);

/// Beyond this many differences the diff gives up and calls the file
/// wholly rewritten. A gutter is an orientation aid; nobody is
/// orienting themselves in a file that differs from its committed self
/// in two thousand places by reading the marks.
const MAX_DIFFERENCES: usize = 1_000;

/// The differing runs between two slices, by Myers' algorithm.
///
/// The cost is O(ND) in the number of differences rather than in the
/// size of the file, which is the property that matters here: this runs
/// while someone is typing. Filling a longest-common-subsequence table
/// instead is O(NM), and measured 4 ms on a two-thousand-line file with
/// five scattered edits — the sort of per-keystroke cost that shows up
/// as a stutter.
fn diff_runs(old: &[&str], new: &[&str]) -> Vec<Run> {
    if old.is_empty() && new.is_empty() {
        return Vec::new();
    }
    if old.is_empty() || new.is_empty() {
        return vec![(0..old.len(), 0..new.len())];
    }
    let Some((deleted, added)) = myers(old, new) else {
        return vec![(0..old.len(), 0..new.len())];
    };

    // Everything not deleted and not added is a match, in order, so
    // walking the two together finds the runs without any bookkeeping
    // about where they sit.
    let mut runs = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && !deleted[i] && !added[j] {
            i += 1;
            j += 1;
            continue;
        }
        let (start_old, start_new) = (i, j);
        while i < old.len() && deleted[i] {
            i += 1;
        }
        while j < new.len() && added[j] {
            j += 1;
        }
        runs.push((start_old..i, start_new..j));
    }
    runs
}

/// Which old lines were deleted and which new lines were added, by the
/// greedy Myers walk with its trace. `None` when the two differ in more
/// than [`MAX_DIFFERENCES`] places.
fn myers(old: &[&str], new: &[&str]) -> Option<(Vec<bool>, Vec<bool>)> {
    let n = old.len() as isize;
    let m = new.len() as isize;
    let max = (n + m).min(MAX_DIFFERENCES as isize);

    // One row per d, holding only the diagonals d can reach — so the
    // whole trace is O(D²) rather than O(D·(N+M)).
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut previous: Vec<isize> = Vec::new();
    let mut end = None;

    for d in 0..=max {
        let mut row = vec![0isize; (2 * d + 1) as usize];
        for k in (-d..=d).step_by(2) {
            // A diagonal of the previous row, if it had one.
            let before = |k: isize| -> Option<isize> {
                let offset = k + (d - 1);
                (d > 0 && offset >= 0 && (offset as usize) < previous.len())
                    .then(|| previous[offset as usize])
            };
            let downward =
                k == -d || (k != d && before(k - 1).unwrap_or(-1) < before(k + 1).unwrap_or(-1));
            let mut x = if downward {
                before(k + 1).unwrap_or(0)
            } else {
                before(k - 1).unwrap_or(0) + 1
            };
            let mut y = x - k;
            while x < n && y < m && old[x as usize] == new[y as usize] {
                x += 1;
                y += 1;
            }
            row[(k + d) as usize] = x;
            if x >= n && y >= m {
                end = Some(d);
            }
        }
        previous = row.clone();
        trace.push(row);
        if end.is_some() {
            break;
        }
    }

    let end = end?;
    let mut deleted = vec![false; old.len()];
    let mut added = vec![false; new.len()];
    let (mut x, mut y) = (n, m);
    for d in (1..=end).rev() {
        let row = &trace[(d - 1) as usize];
        let k = x - y;
        let before = |k: isize| -> Option<isize> {
            let offset = k + (d - 1);
            (offset >= 0 && (offset as usize) < row.len()).then(|| row[offset as usize])
        };
        let downward =
            k == -d || (k != d && before(k - 1).unwrap_or(-1) < before(k + 1).unwrap_or(-1));
        let previous_k = if downward { k + 1 } else { k - 1 };
        let previous_x = before(previous_k).unwrap_or(0);
        let previous_y = previous_x - previous_k;
        // The diagonal run of matching lines comes off first.
        while x > previous_x && y > previous_y {
            x -= 1;
            y -= 1;
        }
        if downward {
            y -= 1;
            added[y as usize] = true;
        } else {
            x -= 1;
            deleted[x as usize] = true;
        }
    }
    Some((deleted, added))
}

/// Turns one differing run into gutter marks.
fn emit(changes: &mut Vec<LineChange>, offset: usize, old: std::ops::Range<usize>, new: std::ops::Range<usize>) {
    let removed = old.len();
    let added = new.len();
    let shared = removed.min(added);
    for step in 0..shared {
        changes.push(LineChange {
            line: offset + new.start + step,
            kind: ChangeKind::Modified,
        });
    }
    for step in shared..added {
        changes.push(LineChange {
            line: offset + new.start + step,
            kind: ChangeKind::Added,
        });
    }
    if removed > added {
        // The lines are gone; the mark goes where they were, which is
        // the boundary at the end of what replaced them.
        changes.push(LineChange {
            line: offset + new.end,
            kind: ChangeKind::Removed,
        });
    }
}

/// The file's contents at `HEAD`, or `None` when there is no such
/// version to compare against: outside a repository, in one with no
/// commits, for a file never committed, or with no `git` on PATH.
pub fn head_baseline(path: &Path) -> Option<String> {
    // Resolved, because git answers with a resolved path and macOS
    // hands out `/tmp` for `/private/tmp`: unresolved, the two never
    // line up and every file would look untracked.
    let path = path.canonicalize().ok()?;
    let directory = path.parent()?;
    let top = git(directory, &["rev-parse", "--show-toplevel"])?;
    let relative = path.strip_prefix(top.trim()).ok()?;
    // `--` keeps a path that looks like a revision from being read as
    // one.
    let blob = git(
        directory,
        &["show", &format!("HEAD:{}", relative.to_string_lossy())],
    )?;
    Some(blob)
}

/// The marks for a file on disk against its committed version. Empty
/// when there is nothing to compare against, which is the same answer
/// as "nothing changed" and is the honest one: every line of an
/// untracked file being new is true and useless.
pub fn changes_for(path: &Path, current: &str) -> Vec<LineChange> {
    match head_baseline(path) {
        Some(baseline) => line_changes(&baseline, current),
        None => Vec::new(),
    }
}

/// The branch names tried, in order, when git does not say which
/// branch is the default one. Configurable, this is only the answer
/// when nobody configured one.
pub const DEFAULT_MERGE_BASE_BRANCHES: &[&str] = &["main", "master", "trunk", "develop"];

/// What the gutter compares against: the last commit, or the commit
/// this branch grew from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Baseline {
    Head,
    Branch,
}

impl Baseline {
    /// `"branch"` is the fork point; anything else is the last commit,
    /// which is also what an unset key means.
    pub fn parse(name: &str) -> Self {
        if name == "branch" {
            Baseline::Branch
        } else {
            Baseline::Head
        }
    }
}

/// The commit this branch grew from: the merge base of `HEAD` and the
/// default branch — `origin/HEAD` when git knows it, else the first
/// name on `priorities` that exists, locally or on `origin`.
///
/// `None` when there is no fork to speak of: outside a repository, or
/// standing on the default branch itself, where the merge base is
/// `HEAD` and the branch view would say nothing the plain one does not.
pub fn fork_point(directory: &Path, priorities: &[String]) -> Option<String> {
    // An empty list means nobody chose one, the same as the config's
    // absent key.
    let default: Vec<String>;
    let priorities = if priorities.is_empty() {
        default = DEFAULT_MERGE_BASE_BRANCHES
            .iter()
            .map(|name| name.to_string())
            .collect();
        &default
    } else {
        priorities
    };
    let head = git(directory, &["rev-parse", "HEAD"])?;
    let head = head.trim();
    let mut candidates: Vec<String> = Vec::new();
    if let Some(default) = git(
        directory,
        &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    ) {
        candidates.push(default.trim().to_string());
    }
    for name in priorities {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        candidates.push(name.to_string());
        candidates.push(format!("origin/{name}"));
    }
    for candidate in candidates {
        let Some(base) = git(directory, &["merge-base", "HEAD", &candidate]) else {
            continue;
        };
        let base = base.trim();
        if base == head {
            continue;
        }
        return Some(base.to_string());
    }
    None
}

/// The file's contents at the branch's fork point. `None` for the same
/// reasons as [`head_baseline`], when there is no fork point, and for a
/// file the branch added — which, like an untracked file, would mark
/// every line, and is left to the changed-files list to say.
pub fn branch_baseline(path: &Path, priorities: &[String]) -> Option<String> {
    let path = path.canonicalize().ok()?;
    let directory = path.parent()?;
    let top = git(directory, &["rev-parse", "--show-toplevel"])?;
    let relative = path.strip_prefix(top.trim()).ok()?;
    let base = fork_point(directory, priorities)?;
    git(
        directory,
        &["show", &format!("{base}:{}", relative.to_string_lossy())],
    )
}

/// [`changes_for`], against the chosen baseline. Branch mode falls
/// back to the last commit when there is no fork point to compare
/// against, so the gutter never goes quieter than it was.
pub fn changes_against(
    path: &Path,
    current: &str,
    baseline: Baseline,
    priorities: &[String],
) -> Vec<LineChange> {
    let text = match baseline {
        Baseline::Head => head_baseline(path),
        Baseline::Branch => {
            if fork_point_exists(path, priorities) {
                branch_baseline(path, priorities)
            } else {
                head_baseline(path)
            }
        }
    };
    match text {
        Some(text) => line_changes(&text, current),
        None => Vec::new(),
    }
}

fn fork_point_exists(path: &Path, priorities: &[String]) -> bool {
    path.canonicalize()
        .ok()
        .and_then(|path| path.parent().map(|d| d.to_path_buf()))
        .is_some_and(|directory| fork_point(&directory, priorities).is_some())
}

/// The files this branch touches — committed on it, changed in the
/// working tree, or not yet tracked: the pull request's files, read
/// from git alone. Standing on the default branch, the working tree's
/// own changes. Paths are relative to the returned repository root;
/// deleted files are left out, there being nothing to open.
pub fn branch_files(start: &Path, priorities: &[String]) -> Option<(String, Vec<(char, String)>)> {
    let start = start.canonicalize().ok()?;
    let directory = if start.is_dir() {
        start.as_path()
    } else {
        start.parent()?
    };
    let top = git(directory, &["rev-parse", "--show-toplevel"])?;
    let top = top.trim().to_string();
    let against = fork_point(directory, priorities).unwrap_or_else(|| "HEAD".into());
    let mut files: std::collections::BTreeMap<String, char> = Default::default();
    if let Some(listed) = git(directory, &["diff", "--name-status", &against]) {
        for line in listed.lines() {
            let mut parts = line.split('\t');
            let Some(status) = parts.next().and_then(|s| s.chars().next()) else {
                continue;
            };
            // A rename lists old then new; the new name is the file.
            let Some(path) = parts.last() else { continue };
            if status == 'D' {
                continue;
            }
            files.insert(path.to_string(), status);
        }
    }
    if let Some(untracked) = git(directory, &["ls-files", "--others", "--exclude-standard"]) {
        for line in untracked.lines() {
            if !line.is_empty() {
                files.entry(line.to_string()).or_insert('A');
            }
        }
    }
    Some((top, files.into_iter().map(|(path, status)| (status, path)).collect()))
}

/// The list as JSON — `{"root": "...", "files": [{"status": "M",
/// "path": "src/a.rs"}, …]}` — for shells on the C ABI. `{}` when
/// there is no repository.
pub fn branch_files_json(start: &Path, priorities: &[String]) -> String {
    let Some((root, files)) = branch_files(start, priorities) else {
        return "{}".into();
    };
    let items: Vec<serde_json::Value> = files
        .iter()
        .map(|(status, path)| serde_json::json!({"status": status.to_string(), "path": path}))
        .collect();
    serde_json::json!({"root": root, "files": items}).to_string()
}

/// The marks as JSON, which is how they cross the FFI.
pub fn to_json(changes: &[LineChange]) -> String {
    let items: Vec<serde_json::Value> = changes
        .iter()
        .map(|change| serde_json::json!({"line": change.line, "kind": change.kind.name()}))
        .collect();
    serde_json::Value::Array(items).to_string()
}

fn git(directory: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(baseline: &str, current: &str) -> Vec<(usize, &'static str)> {
        line_changes(baseline, current)
            .into_iter()
            .map(|change| (change.line, change.kind.name()))
            .collect()
    }

    #[test]
    fn an_unchanged_file_has_no_marks() {
        assert!(marks("a\nb\nc\n", "a\nb\nc\n").is_empty());
        assert!(marks("", "").is_empty());
    }

    #[test]
    fn a_line_typed_over_is_modified() {
        assert_eq!(marks("a\nb\nc\n", "a\nB\nc\n"), vec![(1, "modified")]);
    }

    #[test]
    fn lines_inserted_are_added() {
        assert_eq!(
            marks("a\nc\n", "a\nb1\nb2\nc\n"),
            vec![(1, "added"), (2, "added")]
        );
    }

    #[test]
    fn lines_deleted_leave_a_mark_on_the_boundary() {
        // "b" and "c" are gone; nothing occupies their place, so the
        // mark sits where they were.
        assert_eq!(marks("a\nb\nc\nd\n", "a\nd\n"), vec![(1, "removed")]);
    }

    #[test]
    fn a_rewritten_paragraph_is_modified_as_far_as_it_overlaps() {
        // Three lines became two: two modified, and the third is gone.
        assert_eq!(
            marks("a\nb\nc\nd\n", "a\nB\nC\n"),
            vec![(1, "modified"), (2, "modified"), (3, "removed")]
        );
        // Two became three: two modified and one added.
        assert_eq!(
            marks("a\nb\nc\n", "a\nB\nC\nD\n"),
            vec![(1, "modified"), (2, "modified"), (3, "added")]
        );
    }

    #[test]
    fn everything_new_is_added_and_everything_gone_is_removed() {
        assert_eq!(marks("", "a\nb\n"), vec![(0, "added"), (1, "added")]);
        assert_eq!(marks("a\nb\n", ""), vec![(0, "removed")]);
    }

    #[test]
    fn edits_far_apart_are_marked_separately() {
        let baseline = "one\ntwo\nthree\nfour\nfive\nsix\n";
        let current = "one\nTWO\nthree\nfour\nfive\nSIX\n";
        assert_eq!(marks(baseline, current), vec![(1, "modified"), (5, "modified")]);
    }

    #[test]
    fn a_moved_block_is_not_claimed_to_be_everything() {
        // The common subsequence keeps the untouched lines untouched
        // rather than reporting the whole file rewritten.
        let baseline = "a\nb\nc\nd\ne\n";
        let current = "a\nc\nd\nb\ne\n";
        let found = marks(baseline, current);
        assert!(found.len() <= 3, "{found:?}");
        assert!(found.iter().all(|(line, _)| *line != 0 && *line != 4), "{found:?}");
    }

    #[test]
    fn a_file_that_differs_everywhere_is_called_rewritten_rather_than_diffed() {
        // Past the difference cap the walk stops and says the whole
        // thing changed, which is both true and cheap. Diffing it
        // exactly costs milliseconds and tells a reader nothing they
        // could not see.
        let baseline: String = (0..MAX_DIFFERENCES).map(|n| format!("old {n}\n")).collect();
        let current: String = (0..MAX_DIFFERENCES).map(|n| format!("new {n}\n")).collect();
        let found = line_changes(&baseline, &current);
        assert_eq!(found.len(), MAX_DIFFERENCES);
        assert!(found.iter().all(|change| change.kind == ChangeKind::Modified));
    }

    #[test]
    fn a_file_too_large_to_diff_is_left_unmarked() {
        let big: String = (0..MAX_LINES + 1).map(|n| format!("line {n}\n")).collect();
        assert!(line_changes(&big, "one line\n").is_empty());
    }

    #[test]
    fn a_file_with_no_committed_version_gets_no_marks() {
        let dir = std::env::temp_dir().join(format!("textchum-changes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("never-committed.txt");
        std::fs::write(&path, "hello\n").unwrap();
        assert!(changes_for(&path, "hello\nworld\n").is_empty());
    }

    #[test]
    fn a_committed_file_reports_what_changed_since() {
        let dir = std::env::temp_dir().join(format!("textchum-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |arguments: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(arguments)
                .output()
                .expect("git runs")
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.invalid"]);
        run(&["config", "user.name", "Test"]);
        let path = dir.join("thing.txt");
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        run(&["add", "thing.txt"]);
        run(&["commit", "-qm", "first"]);

        // Committed and unchanged.
        assert!(changes_for(&path, "one\ntwo\nthree\n").is_empty());
        // Edited in the buffer, not yet saved: the marks come from what
        // is in the buffer, not from the file on disk.
        let found = changes_for(&path, "one\nTWO\nthree\nfour\n");
        assert_eq!(
            found,
            vec![
                LineChange { line: 1, kind: ChangeKind::Modified },
                LineChange { line: 3, kind: ChangeKind::Added },
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_branch_compares_against_where_it_forked() {
        let dir = std::env::temp_dir().join(format!("textchum-branch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |arguments: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(arguments)
                .output()
                .expect("git runs")
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "test@example.invalid"]);
        run(&["config", "user.name", "Test"]);
        let path = dir.join("thing.txt");
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        run(&["add", "thing.txt"]);
        run(&["commit", "-qm", "first"]);
        run(&["checkout", "-qb", "feature"]);
        std::fs::write(&path, "one\nTWO\nthree\n").unwrap();
        run(&["commit", "-qam", "branch work"]);

        let priorities: Vec<String> = vec!["main".into()];
        // Committed on the branch: quiet against HEAD, loud against the
        // fork point.
        assert!(changes_against(&path, "one\nTWO\nthree\n", Baseline::Head, &priorities)
            .is_empty());
        assert_eq!(
            changes_against(&path, "one\nTWO\nthree\n", Baseline::Branch, &priorities),
            vec![LineChange { line: 1, kind: ChangeKind::Modified }]
        );
        // The list of touched files: the committed edit plus a file not
        // yet tracked, the deleted one left out.
        std::fs::write(dir.join("fresh.txt"), "new\n").unwrap();
        let (root, files) = branch_files(&dir, &priorities).expect("a repository");
        assert!(root.ends_with(&dir.file_name().unwrap().to_string_lossy().to_string()));
        assert_eq!(
            files,
            vec![('A', "fresh.txt".into()), ('M', "thing.txt".into())]
        );
        // Standing on the default branch there is no fork point, and
        // the branch view says what the plain one says.
        run(&["checkout", "-q", "main"]);
        assert!(fork_point(&dir, &priorities).is_none());
        assert_eq!(
            changes_against(&path, "one\nTWO\nthree\n", Baseline::Branch, &priorities),
            changes_against(&path, "one\nTWO\nthree\n", Baseline::Head, &priorities),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_carries_the_line_and_the_kind() {
        let json = to_json(&line_changes("a\nb\n", "a\nB\n"));
        assert_eq!(json, r#"[{"kind":"modified","line":1}]"#);
    }
}
