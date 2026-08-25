//! The shapes a file's location takes for the clipboard: bare name,
//! project-relative, absolute, or a URL on the repository's forge
//! (GitHub, GitLab, Forgejo and friends) — the Rust twin of the macOS
//! shell's PathActions.

use std::path::Path;
use std::process::Command;

/// The path relative to the project root when the file is inside it,
/// otherwise the absolute path with the home directory abbreviated.
pub fn relative_path(path: &str, project_root: Option<&str>) -> String {
    if let Some(root) = project_root {
        if let Some(rest) = path.strip_prefix(&format!("{root}/")) {
            return rest.to_owned();
        }
    }
    let home = gtk::glib::home_dir();
    let home = home.to_string_lossy();
    match path.strip_prefix(&format!("{home}/")) {
        Some(rest) => format!("~/{rest}"),
        None => path.to_owned(),
    }
}

/// The file's page on the repository's forge: host and repository from
/// the `origin` remote (or the first remote there is), current branch,
/// and the file's path inside the repository. The URL shape follows
/// the host — GitHub's `blob`, GitLab's `-/blob`, and the
/// `src/branch` layout Forgejo and Gitea share.
pub fn forge_url(path: &str) -> Option<String> {
    let dir = Path::new(path).parent()?.to_string_lossy().into_owned();
    let top = git(&dir, &["rev-parse", "--show-toplevel"])?;
    let mut branch = git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let detached = branch == "HEAD";
    if detached {
        branch = git(&dir, &["rev-parse", "--short", "HEAD"])?;
    }
    let remote = remote_url(&dir)?;
    let (host, repo) = parse_remote(&remote)?;
    let relative = path
        .strip_prefix(&top)
        .unwrap_or("")
        .trim_matches('/')
        .to_owned();
    let base = format!("https://{host}/{repo}");
    if relative.is_empty() {
        return Some(base);
    }
    let location = format!(
        "{}/{}",
        escape(&branch),
        relative.split('/').map(escape).collect::<Vec<_>>().join("/")
    );
    Some(if host == "github.com" {
        format!("{base}/blob/{location}")
    } else if host.contains("gitlab") {
        format!("{base}/-/blob/{location}")
    } else {
        format!(
            "{base}/src/{}/{location}",
            if detached { "commit" } else { "branch" }
        )
    })
}

fn escape(component: impl AsRef<str>) -> String {
    let mut out = String::new();
    for byte in component.as_ref().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn git(dir: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn remote_url(dir: &str) -> Option<String> {
    if let Some(origin) = git(dir, &["remote", "get-url", "origin"]) {
        return Some(origin);
    }
    let first = git(dir, &["remote"])?;
    let first = first.lines().next()?.to_owned();
    git(dir, &["remote", "get-url", &first])
}

/// Host plus "owner/repo" out of the remote spellings in the wild:
/// scp-like `git@host:owner/repo.git`, `ssh://git@host/owner/repo`,
/// and plain `https://host/owner/repo.git`.
fn parse_remote(remote: &str) -> Option<(String, String)> {
    let remote = remote.strip_suffix(".git").unwrap_or(remote);
    if let Some(scheme_end) = remote.find("://") {
        let rest = &remote[scheme_end + 3..];
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        let host = host.split(':').next()?;
        let repo = path.trim_matches('/');
        return (!host.is_empty() && !repo.is_empty())
            .then(|| (host.to_owned(), repo.to_owned()));
    }
    let (host, repo) = remote.split_once(':')?;
    let host = host.rsplit('@').next()?;
    let repo = repo.trim_matches('/');
    (!host.is_empty() && !repo.is_empty()).then(|| (host.to_owned(), repo.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_spellings_parse() {
        assert_eq!(
            parse_remote("git@github.com:owner/repo.git"),
            Some(("github.com".into(), "owner/repo".into()))
        );
        assert_eq!(
            parse_remote("ssh://git@forge.example/owner/repo"),
            Some(("forge.example".into(), "owner/repo".into()))
        );
        assert_eq!(
            parse_remote("https://gitlab.com/group/sub/repo.git"),
            Some(("gitlab.com".into(), "group/sub/repo".into()))
        );
        assert_eq!(parse_remote("nonsense"), None);
    }

    #[test]
    fn relative_paths_prefer_the_root() {
        assert_eq!(relative_path("/work/proj/src/a.rs", Some("/work/proj")), "src/a.rs");
        assert_eq!(relative_path("/elsewhere/a.rs", Some("/work/proj")), "/elsewhere/a.rs");
    }
}
