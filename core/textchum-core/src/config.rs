//! Application configuration, backed by a JSON file.
//!
//! Configuration is deliberately a plain, human-editable JSON file: the
//! settings UI is the comfortable way to change it, and a text editor is
//! the escape hatch when something goes wrong. Three properties follow from
//! that:
//!
//! * **Defaults are total.** Every getter returns a sensible value whether
//!   the file is missing, a key is absent, or a value has the wrong type or
//!   an out-of-range number. A broken config can degrade the experience,
//!   never the ability to edit.
//! * **Broken files are preserved, not clobbered.** If the file fails to
//!   parse, the core reports a warning and runs on defaults — and before
//!   the next successful save would overwrite the file, the unparseable
//!   original is copied aside to `<name>.bak` so hand editing is never
//!   lost.
//! * **Hand edits survive the GUI.** The whole JSON document is kept in
//!   memory; setters modify only the keys they own, so unknown keys a
//!   human (or a future version) added are written back untouched. Saves
//!   are atomic, like every write the core does.
//!
//! The core does not decide where the file lives — platform conventions are
//! shell knowledge. Shells pass the path in.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::fsutil::write_atomically;

/// Bounds and defaults for the recognized settings. Out-of-range values
/// from the file are clamped rather than rejected.
pub const DEFAULT_FONT_SIZE: f64 = 13.0;
pub const FONT_SIZE_RANGE: (f64, f64) = (6.0, 72.0);
pub const DEFAULT_TAB_WIDTH: u32 = 4;
pub const TAB_WIDTH_RANGE: (u32, u32) = (1, 16);

/// The application's configuration: the parsed JSON document plus the path
/// it round-trips through.
pub struct Config {
    path: PathBuf,
    root: Value,
    /// The on-disk file exists but could not be parsed; back it up before
    /// the next save overwrites it.
    broken_on_disk: bool,
}

impl Config {
    /// Loads the configuration at `path`.
    ///
    /// Always returns a usable `Config`. The second element carries a
    /// human-readable warning when the file existed but could not be used
    /// (parse error, unreadable, not a JSON object); the shell should show
    /// it to the user once.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        let fresh = |broken| Self {
            path: path.to_owned(),
            root: Value::Object(Map::new()),
            broken_on_disk: broken,
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (fresh(false), None),
            Err(e) => {
                return (
                    fresh(false),
                    Some(format!(
                        "could not read {}: {e}; using default settings",
                        path.display()
                    )),
                );
            }
        };
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(root @ Value::Object(_)) => (
                Self {
                    path: path.to_owned(),
                    root,
                    broken_on_disk: false,
                },
                None,
            ),
            Ok(_) => (
                fresh(true),
                Some(format!(
                    "{} is valid JSON but not an object; using default settings \
                     (the file will be backed up before it is next written)",
                    path.display()
                )),
            ),
            Err(e) => (
                fresh(true),
                Some(format!(
                    "{} is not valid JSON ({e}); using default settings \
                     (the file will be backed up before it is next written)",
                    path.display()
                )),
            ),
        }
    }

    /// Editor font family, if one is configured. `None` means "use the
    /// platform's monospaced font".
    pub fn font_family(&self) -> Option<&str> {
        self.editor().get("font_family")?.as_str().filter(|s| !s.is_empty())
    }

    /// Editor font size in points, clamped to [`FONT_SIZE_RANGE`].
    pub fn font_size(&self) -> f64 {
        self.editor()
            .get("font_size")
            .and_then(Value::as_f64)
            .filter(|size| size.is_finite())
            .map(|size| size.clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1))
            .unwrap_or(DEFAULT_FONT_SIZE)
    }

    /// Tab width in columns, clamped to [`TAB_WIDTH_RANGE`].
    pub fn tab_width(&self) -> u32 {
        self.editor()
            .get("tab_width")
            .and_then(Value::as_u64)
            .map(|width| (width as u32).clamp(TAB_WIDTH_RANGE.0, TAB_WIDTH_RANGE.1))
            .unwrap_or(DEFAULT_TAB_WIDTH)
    }

    /// Sets the font family; `None` (or empty) removes the key, returning
    /// to the platform default.
    pub fn set_font_family(&mut self, family: Option<&str>) {
        match family.filter(|f| !f.is_empty()) {
            Some(family) => {
                self.editor_mut()
                    .insert("font_family".into(), Value::String(family.to_owned()));
            }
            None => {
                self.editor_mut().remove("font_family");
            }
        }
    }

    pub fn set_font_size(&mut self, size: f64) {
        let size = if size.is_finite() {
            size.clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1)
        } else {
            DEFAULT_FONT_SIZE
        };
        // Serialize whole sizes as integers so the file stays pleasant to
        // hand-edit ("13" rather than "13.0").
        let value = if size.fract() == 0.0 {
            Value::from(size as u64)
        } else {
            Value::from(size)
        };
        self.editor_mut().insert("font_size".into(), value);
    }

    pub fn set_tab_width(&mut self, width: u32) {
        let width = width.clamp(TAB_WIDTH_RANGE.0, TAB_WIDTH_RANGE.1);
        self.editor_mut().insert("tab_width".into(), Value::from(width));
    }

    /// Writes the configuration back to its path, pretty-printed and
    /// atomic. If the on-disk file was unparseable at load time it is
    /// copied to `<name>.bak` first so the hand-edited content survives.
    pub fn save(&mut self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        if self.broken_on_disk {
            let mut backup = self.path.as_os_str().to_owned();
            backup.push(".bak");
            std::fs::copy(&self.path, PathBuf::from(backup))?;
        }
        let mut pretty = serde_json::to_string_pretty(&self.root)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        pretty.push('\n');
        write_atomically(&self.path, pretty.as_bytes())?;
        self.broken_on_disk = false;
        Ok(())
    }

    /// The `editor` section, read-only. Missing or mistyped sections read
    /// as empty.
    fn editor(&self) -> &Map<String, Value> {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        self.root
            .get("editor")
            .and_then(Value::as_object)
            .unwrap_or_else(|| EMPTY.get_or_init(Map::new))
    }

    /// The `editor` section, created (or replaced, if it was mistyped) on
    /// first write.
    fn editor_mut(&mut self) -> &mut Map<String, Value> {
        let root = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        if !root.get("editor").map(Value::is_object).unwrap_or(false) {
            root.insert("editor".into(), Value::Object(Map::new()));
        }
        root.get_mut("editor")
            .and_then(Value::as_object_mut)
            .expect("just ensured the editor section is an object")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("textchum-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn missing_file_yields_defaults_without_warning() {
        let (config, warning) = Config::load(&temp_path("nonexistent.json"));
        assert!(warning.is_none());
        assert_eq!(config.font_family(), None);
        assert_eq!(config.font_size(), DEFAULT_FONT_SIZE);
        assert_eq!(config.tab_width(), DEFAULT_TAB_WIDTH);
    }

    #[test]
    fn values_round_trip_through_disk() {
        let path = temp_path("roundtrip.json");
        let (mut config, _) = Config::load(&path);
        config.set_font_family(Some("Menlo"));
        config.set_font_size(15.0);
        config.set_tab_width(8);
        config.save().unwrap();

        let (reloaded, warning) = Config::load(&path);
        assert!(warning.is_none());
        assert_eq!(reloaded.font_family(), Some("Menlo"));
        assert_eq!(reloaded.font_size(), 15.0);
        assert_eq!(reloaded.tab_width(), 8);
    }

    #[test]
    fn unknown_keys_survive_a_gui_save() {
        let path = temp_path("hand-edited.json");
        std::fs::write(
            &path,
            r#"{"editor": {"font_size": 11, "future_setting": true}, "experimental": {"x": 1}}"#,
        )
        .unwrap();

        let (mut config, warning) = Config::load(&path);
        assert!(warning.is_none());
        config.set_font_size(14.0);
        config.save().unwrap();

        let written: Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written["editor"]["font_size"], 14);
        assert_eq!(written["editor"]["future_setting"], true);
        assert_eq!(written["experimental"]["x"], 1);
    }

    #[test]
    fn broken_file_warns_defaults_and_is_backed_up_on_save() {
        let path = temp_path("broken.json");
        std::fs::write(&path, "{ this is not json").unwrap();

        let (mut config, warning) = Config::load(&path);
        assert!(warning.unwrap().contains("not valid JSON"));
        assert_eq!(config.font_size(), DEFAULT_FONT_SIZE);
        // The broken file is untouched by merely loading.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ this is not json");

        config.set_tab_width(2);
        config.save().unwrap();
        let backup = PathBuf::from(format!("{}.bak", path.display()));
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "{ this is not json"
        );
        let (reloaded, warning) = Config::load(&path);
        assert!(warning.is_none());
        assert_eq!(reloaded.tab_width(), 2);
    }

    #[test]
    fn mistyped_and_out_of_range_values_degrade_to_sane_ones() {
        let path = temp_path("weird.json");
        std::fs::write(
            &path,
            r#"{"editor": {"font_family": 42, "font_size": 4000, "tab_width": 0}}"#,
        )
        .unwrap();
        let (config, warning) = Config::load(&path);
        assert!(warning.is_none(), "recognizable JSON is not a parse error");
        assert_eq!(config.font_family(), None);
        assert_eq!(config.font_size(), FONT_SIZE_RANGE.1);
        assert_eq!(config.tab_width(), TAB_WIDTH_RANGE.0);
    }

    #[test]
    fn whole_sizes_serialize_as_integers() {
        let path = temp_path("integers.json");
        let (mut config, _) = Config::load(&path);
        config.set_font_size(13.0);
        config.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"font_size\": 13"), "got: {text}");
        assert!(!text.contains("13.0"), "got: {text}");
    }
}
