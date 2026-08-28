//! Who last touched a line, and what they were doing at the time.
//!
//! The change gutter says a line differs from the committed file. This
//! answers the opposite question about every line it does not mark.
//!
//! Blame is asked with the buffer's own text on standard input, not
//! with whatever is on disk. An edited buffer's line numbers stop
//! lining up with the saved file the moment a line is added above, and
//! an answer about the wrong line — delivered with a name and a date
//! and every appearance of being right — is worse than no answer.
//! Given the text, git blames against it and reports the lines that are
//! not committed as exactly that.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// What git knows about one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blame {
    /// The line this is about, one-based. Not always the line asked
    /// for: a caret on the empty line past the end of the file is
    /// answered about the last line there is.
    pub line: usize,
    /// The full commit hash, empty when the line is not committed.
    pub commit: String,
    /// The short hash, for showing.
    pub abbreviated: String,
    pub author: String,
    pub author_mail: String,
    /// As git formats it: `2026-08-28 14:03:22 +0200`.
    pub author_date: String,
    /// Set only when it differs from the author — a rebase or a
    /// cherry-pick is a different story from an ordinary commit, and
    /// worth showing rather than hiding.
    pub committer: String,
    pub committer_date: String,
    /// The commit's first line.
    pub summary: String,
    /// The rest of the message, where the reasoning lives.
    pub body: String,
    /// The file's name at that commit, set only when it has been
    /// renamed since.
    pub renamed_from: String,
    /// The line was typed and not yet committed.
    pub uncommitted: bool,
}

/// Why a line could not be blamed. Each of these is an ordinary
/// situation with a sentence to say, not a failure to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlameError {
    /// No `git` on PATH, or it could not be run.
    NoGit,
    /// The file is not in a repository, or not known to it.
    NotTracked,
    /// git ran and refused, with what it said.
    Refused(String),
}

impl std::fmt::Display for BlameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoGit => write!(f, "git is not installed, or could not be run"),
            Self::NotTracked => write!(f, "this file is not in a git repository"),
            Self::Refused(message) => write!(f, "{message}"),
        }
    }
}

/// Blames one-based `line` of `path`, against `contents` — the buffer's
/// text, so the answer is about the line the reader is looking at.
pub fn blame_line(path: &Path, line: usize, contents: &str) -> Result<Blame, BlameError> {
    let path = path.canonicalize().map_err(|_| BlameError::NotTracked)?;
    let directory = path.parent().ok_or(BlameError::NotTracked)?;
    // The caret sits on the empty line past the end as readily as
    // anywhere else, and git refuses a line number the file does not
    // have. The last line is what was meant.
    let line = line.clamp(1, contents.lines().count().max(1));

    let porcelain = git_with_stdin(
        directory,
        &[
            "blame",
            "-L",
            &format!("{line},{line}"),
            "--line-porcelain",
            // The buffer's text, so an unsaved edit above this line
            // does not shift the answer onto its neighbour.
            "--contents",
            "-",
            "--",
            &path.to_string_lossy(),
        ],
        contents,
    )?;

    let mut blame = parse_porcelain(&porcelain).ok_or_else(|| {
        BlameError::Refused("git answered in a shape this does not understand".into())
    })?;
    blame.line = line;
    // The porcelain always names the file, whether or not it moved. It
    // is only worth saying when it is different from the name now.
    if path.ends_with(&blame.renamed_from) {
        blame.renamed_from.clear();
    }
    if blame.uncommitted {
        return Ok(blame);
    }

    // The dates and the message body, formatted by git rather than by
    // hand. `%b` comes last: it is the only field that can hold
    // anything, including the separator.
    let separator = '\u{1f}';
    let format = format!(
        "%h{separator}%an{separator}%ae{separator}%ad{separator}%cn{separator}%cd{separator}%s{separator}%b"
    );
    if let Some(shown) = git(
        directory,
        &["show", "-s", &format!("--format={format}"), "--date=iso", &blame.commit],
    ) {
        let mut fields = shown.trim_end_matches('\n').splitn(8, separator);
        blame.abbreviated = fields.next().unwrap_or_default().to_owned();
        blame.author = fields.next().unwrap_or(&blame.author).to_owned();
        blame.author_mail = fields.next().unwrap_or(&blame.author_mail).to_owned();
        blame.author_date = fields.next().unwrap_or_default().to_owned();
        let committer = fields.next().unwrap_or_default().to_owned();
        let committer_date = fields.next().unwrap_or_default().to_owned();
        blame.summary = fields.next().unwrap_or(&blame.summary).to_owned();
        blame.body = fields.next().unwrap_or_default().trim().to_owned();
        // Only worth showing when it is a different story.
        if committer != blame.author || committer_date != blame.author_date {
            blame.committer = committer;
            blame.committer_date = committer_date;
        }
    }
    Ok(blame)
}

/// Reads `git blame --line-porcelain` for a single line.
fn parse_porcelain(text: &str) -> Option<Blame> {
    let mut lines = text.lines();
    let header = lines.next()?;
    let commit = header.split_whitespace().next()?.to_owned();
    let uncommitted = commit.chars().all(|c| c == '0');

    let mut blame = Blame {
        line: 0,
        abbreviated: commit.chars().take(9).collect(),
        commit: if uncommitted { String::new() } else { commit },
        author: String::new(),
        author_mail: String::new(),
        author_date: String::new(),
        committer: String::new(),
        committer_date: String::new(),
        summary: String::new(),
        body: String::new(),
        renamed_from: String::new(),
        uncommitted,
    };
    if uncommitted {
        blame.abbreviated = String::new();
    }

    for line in lines {
        // The blamed line itself arrives prefixed with a tab, and ends
        // the header.
        if line.starts_with('\t') {
            break;
        }
        let (key, value) = match line.split_once(' ') {
            Some(pair) => pair,
            None => (line, ""),
        };
        match key {
            "author" => blame.author = value.to_owned(),
            "author-mail" => {
                blame.author_mail = value.trim_matches(|c| c == '<' || c == '>').to_owned()
            }
            "summary" => blame.summary = value.to_owned(),
            "filename" => blame.renamed_from = value.to_owned(),
            _ => {}
        }
    }
    Some(blame)
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

/// Runs git with `input` on standard input, distinguishing "no git"
/// from "git said no" so the caller can say which.
fn git_with_stdin(
    directory: &Path,
    arguments: &[&str],
    input: &str,
) -> Result<String, BlameError> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| BlameError::NoGit)?;
    if let Some(stdin) = child.stdin.as_mut() {
        // A broken pipe here means git rejected the arguments and is
        // already on its way out; its message is the useful part.
        let _ = stdin.write_all(input.as_bytes());
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().map_err(|_| BlameError::NoGit)?;
    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map_err(|_| BlameError::Refused("git answered in something other than UTF-8".into()));
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if message.contains("no such path")
        || message.contains("not a git repository")
        || message.contains("does not exist")
    {
        return Err(BlameError::NotTracked);
    }
    Err(BlameError::Refused(if message.is_empty() {
        "git could not blame this line".into()
    } else {
        message
    }))
}

/// The blame as JSON, which is how it crosses the FFI.
pub fn to_json(blame: &Blame) -> String {
    serde_json::json!({
        "line": blame.line,
        "commit": blame.commit,
        "abbreviated": blame.abbreviated,
        "author": blame.author,
        "authorMail": blame.author_mail,
        "authorDate": blame.author_date,
        "committer": blame.committer,
        "committerDate": blame.committer_date,
        "summary": blame.summary,
        "body": blame.body,
        "renamedFrom": blame.renamed_from,
        "uncommitted": blame.uncommitted,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Repo {
        dir: std::path::PathBuf,
    }

    impl Repo {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("textchum-blame-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let repo = Self { dir };
            repo.git(&["init", "-q"]);
            repo.git(&["config", "user.email", "ada@example.invalid"]);
            repo.git(&["config", "user.name", "Ada Lovelace"]);
            repo
        }

        fn git(&self, arguments: &[&str]) {
            let status = Command::new("git")
                .arg("-C")
                .arg(&self.dir)
                .args(arguments)
                .output()
                .expect("git runs");
            assert!(
                status.status.success(),
                "git {arguments:?}: {}",
                String::from_utf8_lossy(&status.stderr)
            );
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_committed_line_reports_who_wrote_it_and_why() {
        let repo = Repo::new("committed");
        let path = repo.dir.join("thing.txt");
        std::fs::write(&path, "first\nsecond\nthird\n").unwrap();
        repo.git(&["add", "thing.txt"]);
        repo.git(&[
            "commit",
            "-qm",
            "Add the thing\n\nBecause the other thing needed one.",
        ]);

        let blame = blame_line(&path, 2, "first\nsecond\nthird\n").unwrap();
        assert!(!blame.uncommitted);
        assert_eq!(blame.author, "Ada Lovelace");
        assert_eq!(blame.author_mail, "ada@example.invalid");
        assert_eq!(blame.summary, "Add the thing");
        assert_eq!(blame.body, "Because the other thing needed one.");
        assert_eq!(blame.commit.len(), 40);
        assert!(!blame.abbreviated.is_empty());
        // An ISO date, which is what the shells show.
        assert!(blame.author_date.starts_with("20"), "{}", blame.author_date);
        // Author and committer are the same person here, so the second
        // pair stays empty rather than repeating the first.
        assert!(blame.committer.is_empty());
    }

    #[test]
    fn the_blame_follows_the_buffer_rather_than_the_file_on_disk() {
        let repo = Repo::new("buffer");
        let path = repo.dir.join("thing.txt");
        std::fs::write(&path, "first\nsecond\n").unwrap();
        repo.git(&["add", "thing.txt"]);
        repo.git(&["commit", "-qm", "First commit"]);

        // Two lines typed above "second", not saved. On disk, line 4
        // does not exist; in the buffer it is the committed "second".
        let buffer = "first\ntyped one\ntyped two\nsecond\n";
        let blame = blame_line(&path, 4, buffer).unwrap();
        assert!(!blame.uncommitted, "line 4 of the buffer is committed text");
        assert_eq!(blame.summary, "First commit");

        // And the typed lines say so rather than borrowing a commit.
        let typed = blame_line(&path, 2, buffer).unwrap();
        assert!(typed.uncommitted);
        assert!(typed.commit.is_empty());
        assert!(typed.abbreviated.is_empty());
    }

    #[test]
    fn the_caret_past_the_last_line_blames_the_last_line() {
        let repo = Repo::new("past-the-end");
        let path = repo.dir.join("thing.txt");
        std::fs::write(&path, "first\nsecond\n").unwrap();
        repo.git(&["add", "thing.txt"]);
        repo.git(&["commit", "-qm", "Two lines"]);

        // A trailing newline puts the caret on a line 3 that git has
        // never heard of; refusing to answer teaches nothing.
        let blame = blame_line(&path, 3, "first\nsecond\n").unwrap();
        assert_eq!(blame.summary, "Two lines");
        assert!(!blame.uncommitted);
        // And it says which line it answered about, so the dialog's
        // title does not claim a line that was never blamed.
        assert_eq!(blame.line, 2);
    }

    #[test]
    fn a_renamed_file_reports_the_name_it_had() {
        let repo = Repo::new("renamed");
        let old = repo.dir.join("before.txt");
        std::fs::write(&old, "a line\n").unwrap();
        repo.git(&["add", "before.txt"]);
        repo.git(&["commit", "-qm", "Add before.txt"]);
        repo.git(&["mv", "before.txt", "after.txt"]);
        repo.git(&["commit", "-qm", "Rename it"]);

        let blame = blame_line(&repo.dir.join("after.txt"), 1, "a line\n").unwrap();
        assert_eq!(blame.renamed_from, "before.txt");
        assert_eq!(blame.summary, "Add before.txt");
    }

    #[test]
    fn a_file_that_was_never_renamed_says_nothing_about_its_name() {
        let repo = Repo::new("not-renamed");
        let path = repo.dir.join("steady.txt");
        std::fs::write(&path, "a line\n").unwrap();
        repo.git(&["add", "steady.txt"]);
        repo.git(&["commit", "-qm", "Add it"]);
        // git names the file on every line it blames; repeating it back
        // as "named at the time" would be noise on every answer.
        assert_eq!(blame_line(&path, 1, "a line\n").unwrap().renamed_from, "");
    }

    #[test]
    fn a_file_outside_a_repository_says_so() {
        let dir = std::env::temp_dir().join(format!("textchum-blame-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loose.txt");
        std::fs::write(&path, "hello\n").unwrap();
        assert_eq!(blame_line(&path, 1, "hello\n"), Err(BlameError::NotTracked));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_never_added_says_so_too() {
        let repo = Repo::new("untracked");
        let path = repo.dir.join("new.txt");
        std::fs::write(&path, "hello\n").unwrap();
        assert!(blame_line(&path, 1, "hello\n").is_err());
    }

    #[test]
    fn json_carries_what_a_dialog_shows() {
        let blame = Blame {
            line: 12,
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            abbreviated: "0123456".into(),
            author: "Ada".into(),
            author_mail: "ada@example.invalid".into(),
            author_date: "2026-08-28 14:03:22 +0200".into(),
            committer: String::new(),
            committer_date: String::new(),
            summary: "Do the thing".into(),
            body: "Because.".into(),
            renamed_from: String::new(),
            uncommitted: false,
        };
        let parsed: serde_json::Value = serde_json::from_str(&to_json(&blame)).unwrap();
        assert_eq!(parsed["line"], 12);
        assert_eq!(parsed["abbreviated"], "0123456");
        assert_eq!(parsed["authorMail"], "ada@example.invalid");
        assert_eq!(parsed["summary"], "Do the thing");
        assert_eq!(parsed["uncommitted"], false);
    }
}
