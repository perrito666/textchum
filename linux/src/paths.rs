//! Where Textchum keeps what it owns, and how a run can be told to keep
//! it somewhere else.
//!
//! Ordinarily that is the XDG layout: configuration and themes under
//! `~/.config/textchum`, icon packs under `~/.local/share/textchum`,
//! the session and the server log under `~/.local/state/textchum`.
//!
//! `--data-dir <path>` puts the lot under one directory instead, so a
//! run can be given a profile built for the occasion and thrown away
//! afterwards, without the real one ever being opened. A run with its
//! own profile is also its own process: handing the files to an
//! instance already running would open them in the profile that
//! instance has, and the flag would have done nothing.

use std::path::PathBuf;
use std::sync::OnceLock;

use gtk::glib;

/// The directory `--data-dir` named, if a run named one.
///
/// Read once: the arguments do not change, and a path that moved
/// mid-run would leave half the profile behind.
pub fn data_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let path = data_dir_in(std::env::args())?;
        let _ = std::fs::create_dir_all(&path);
        Some(path)
    })
    .as_ref()
}

/// The directory `--data-dir` names in `arguments`, if any. Kept apart
/// from the stored answer so it can be asked about.
pub fn data_dir_in(arguments: impl Iterator<Item = String>) -> Option<PathBuf> {
    let mut arguments = arguments;
    while let Some(argument) = arguments.next() {
        if argument != "--data-dir" {
            continue;
        }
        let path = arguments.next()?;
        if path.is_empty() {
            return None;
        }
        return Some(match path.strip_prefix("~/") {
            Some(rest) => glib::home_dir().join(rest),
            None => PathBuf::from(path),
        });
    }
    None
}

/// Whether this run has a profile of its own.
pub fn has_data_dir() -> bool {
    data_dir().is_some()
}

/// `~/.config/textchum/config.json` — the Linux home of the same file.
pub fn config_path() -> PathBuf {
    match data_dir() {
        Some(dir) => dir.join("config.json"),
        None => glib::user_config_dir().join("textchum/config.json"),
    }
}

/// User theme JSON files, one per theme, named by their file stem.
pub fn themes_dir() -> PathBuf {
    match data_dir() {
        Some(dir) => dir.join("themes"),
        None => glib::user_config_dir().join("textchum/themes"),
    }
}

/// Imported icon packs.
pub fn icons_dir() -> PathBuf {
    match data_dir() {
        Some(dir) => dir.join("icons"),
        None => glib::user_data_dir().join("textchum/icons"),
    }
}

/// Project records: what each file remembers about itself. State, like
/// the session, and one file per project root.
pub fn projects_dir() -> PathBuf {
    match data_dir() {
        Some(dir) => dir.join("projects"),
        None => state_dir().join("textchum/projects"),
    }
}

/// The session: not config (this is not configuration) and not cache
/// (losing it loses real state).
pub fn session_path() -> PathBuf {
    match data_dir() {
        Some(dir) => dir.join("session.json"),
        None => state_dir().join("textchum/session.json"),
    }
}

/// The language-server debug trail.
pub fn lsp_log_path() -> PathBuf {
    match data_dir() {
        Some(dir) => dir.join("lsp.log"),
        None => state_dir().join("textchum/lsp.log"),
    }
}

/// What to tell someone looking for the log.
pub fn lsp_log_for_display() -> String {
    match data_dir() {
        Some(_) => lsp_log_path().to_string_lossy().into_owned(),
        None => "~/.local/state/textchum/lsp.log".to_string(),
    }
}

/// The XDG state directory.
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> std::vec::IntoIter<String> {
        list.iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn the_flag_names_the_profile() {
        assert_eq!(
            data_dir_in(args(&["textchum-gtk", "--data-dir", "/tmp/profile", "a.py"])),
            Some(PathBuf::from("/tmp/profile"))
        );
    }

    #[test]
    fn without_the_flag_there_is_no_profile() {
        assert_eq!(data_dir_in(args(&["textchum-gtk", "a.py"])), None);
        // A flag with nothing after it names nothing.
        assert_eq!(data_dir_in(args(&["textchum-gtk", "--data-dir"])), None);
        assert_eq!(data_dir_in(args(&["textchum-gtk", "--data-dir", ""])), None);
    }

    #[test]
    fn a_home_relative_path_is_expanded() {
        let expanded = data_dir_in(args(&["textchum-gtk", "--data-dir", "~/scratch"]));
        assert_eq!(expanded, Some(glib::home_dir().join("scratch")));
    }
}
