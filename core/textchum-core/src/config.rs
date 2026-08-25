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

/// Where opening a file puts it: a tab of the current window's group, or
/// a separate window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenTarget {
    #[default]
    Tab,
    Window,
}

impl OpenTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tab => "tab",
            Self::Window => "window",
        }
    }
}

/// The user's appearance choice: follow the system, or force one mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    /// The value as stored in `config.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

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

    /// The appearance choice (top-level `"appearance"` key). Unknown or
    /// missing values mean "follow the system".
    pub fn appearance(&self) -> Appearance {
        self.root
            .get("appearance")
            .and_then(Value::as_str)
            .and_then(Appearance::parse)
            .unwrap_or_default()
    }

    /// Sets the appearance choice. `System` removes the key (it is the
    /// default, and an absent key reads most naturally in the file).
    pub fn set_appearance(&mut self, appearance: Appearance) {
        let root = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        match appearance {
            Appearance::System => {
                root.remove("appearance");
            }
            other => {
                root.insert("appearance".into(), Value::String(other.as_str().to_owned()));
            }
        }
    }

    /// The language-server configuration section (`lsp`), serialized:
    /// `{"defaults": {lang: cmdline}, "projects": {root: {lang: cmdline}}}`.
    /// Empty object when unset. The pool consumes this verbatim.
    pub fn lsp_json(&self) -> String {
        self.root
            .get("lsp")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".into())
    }

    /// Sets (or, with `None`, removes) the server command line for a
    /// language — under `lsp.projects.<root>` when `root` is given,
    /// under `lsp.defaults` otherwise. Empty sections are pruned.
    pub fn set_lsp_entry(&mut self, root: Option<&str>, language: &str, command: Option<&str>) {
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        let lsp = ensure_object(top, "lsp");
        let section = match root {
            Some(root) => ensure_object(ensure_object(lsp, "projects"), root),
            None => ensure_object(lsp, "defaults"),
        };
        match command.map(str::trim).filter(|c| !c.is_empty()) {
            Some(command) => {
                section.insert(language.into(), Value::String(command.into()));
            }
            None => {
                section.remove(language);
            }
        }
        prune_empty(top, "lsp");
    }

    /// The save-preprocessor section (`preprocessors`), serialized:
    /// `{"defaults": {lang: [cmd, ...]}, "projects": {root: {lang: [...]}}}`.
    /// Empty object when unset.
    pub fn preprocessors_json(&self) -> String {
        self.root
            .get("preprocessors")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".into())
    }

    /// Sets (or, with `None`/blank, removes) the save-preprocessor
    /// command list for a language — one command per line, stored as an
    /// array — under `preprocessors.projects.<root>` when `root` is
    /// given, under `preprocessors.defaults` otherwise.
    pub fn set_preprocessor_entry(
        &mut self,
        root: Option<&str>,
        language: &str,
        commands: Option<&str>,
    ) {
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        let section = ensure_object(top, "preprocessors");
        let section = match root {
            Some(root) => ensure_object(ensure_object(section, "projects"), root),
            None => ensure_object(section, "defaults"),
        };
        let lines: Vec<Value> = commands
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| Value::String(line.into()))
            .collect();
        if lines.is_empty() {
            section.remove(language);
        } else {
            section.insert(language.into(), Value::Array(lines));
        }
        prune_empty(top, "preprocessors");
    }

    /// The preprocessor command chain for a language: the project
    /// entry when the root has one, the defaults entry otherwise.
    /// A string entry counts as a one-command chain, so hand-written
    /// configs need no array brackets for the common case.
    pub fn preprocessor_commands(&self, root: Option<&str>, language: &str) -> Vec<String> {
        let section = self.root.get("preprocessors");
        let commands_in = |table: Option<&Value>| -> Option<Vec<String>> {
            let entry = table?.get(language)?;
            match entry {
                Value::String(command) => Some(vec![command.clone()]),
                Value::Array(items) => Some(
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect(),
                ),
                _ => None,
            }
        };
        root.and_then(|root| {
            commands_in(section.and_then(|s| s.get("projects")).and_then(|p| p.get(root)))
        })
        .or_else(|| commands_in(section.and_then(|s| s.get("defaults"))))
        .unwrap_or_default()
    }

    /// The prose spell-check language (`editor.spell`): a spelling
    /// identifier like "en_US", `"auto"` for automatic detection, or
    /// `None` when spell checking is off (the default).
    pub fn spell_language(&self) -> Option<String> {
        self.editor()
            .get("spell")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }

    pub fn set_spell_language(&mut self, language: Option<&str>) {
        match language.map(str::trim).filter(|s| !s.is_empty()) {
            Some(language) => {
                self.editor_mut()
                    .insert("spell".into(), Value::String(language.into()));
            }
            None => {
                self.editor_mut().remove("spell");
            }
        }
    }

    /// Whether the editor shows a line-number gutter
    /// (`editor.line_numbers`, default true).
    /// The chosen theme name (a built-in or a user theme file's name);
    /// absent means the default theme.
    pub fn theme(&self) -> String {
        self.root
            .get("theme")
            .and_then(Value::as_str)
            .unwrap_or(crate::theme::DEFAULT_THEME)
            .to_owned()
    }

    /// Sets the theme choice. The default removes the key, like
    /// [`Self::set_appearance`].
    pub fn set_theme(&mut self, name: &str) {
        let root = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        if name == crate::theme::DEFAULT_THEME {
            root.remove("theme");
        } else {
            root.insert("theme".into(), Value::String(name.to_owned()));
        }
    }

    /// Whether hover documentation pops up on mouse rest
    /// (`editor.hover`, default true).
    pub fn hover_docs(&self) -> bool {
        self.editor()
            .get("hover")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn set_hover_docs(&mut self, enabled: bool) {
        self.editor_mut()
            .insert("hover".into(), Value::Bool(enabled));
    }

    pub fn line_numbers(&self) -> bool {
        self.editor()
            .get("line_numbers")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn set_line_numbers(&mut self, shown: bool) {
        self.editor_mut()
            .insert("line_numbers".into(), Value::Bool(shown));
    }

    /// The keyboard-shortcut overrides (`keys`), serialized: an object of
    /// `{action: "modifiers+key"}` entries (e.g. `"save": "cmd+s"`).
    /// Empty object when unset; hand-edited, no UI.
    pub fn keys_json(&self) -> String {
        self.root
            .get("keys")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".into())
    }

    /// The workspace-behavior section (`workspace`), serialized:
    /// `{"manifest_projects": bool, "recursive_config": bool,
    /// "projects": {root: {same flags}}}`. Top-level flags are the
    /// defaults; per-root entries override them. `{}` when unset.
    pub fn workspace_json(&self) -> String {
        self.root
            .get("workspace")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".into())
    }

    /// Sets (or, with `None`, removes) a workspace flag —
    /// `manifest_projects` or `recursive_config` — for a project root, or
    /// the defaults when `root` is `None`. Empty sections are pruned.
    pub fn set_workspace_flag(&mut self, root: Option<&str>, key: &str, value: Option<bool>) {
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        let workspace = ensure_object(top, "workspace");
        let section = match root {
            Some(root) => ensure_object(ensure_object(workspace, "projects"), root),
            None => workspace,
        };
        match value {
            Some(value) => {
                section.insert(key.into(), Value::Bool(value));
            }
            None => {
                section.remove(key);
            }
        }
        prune_empty(top, "workspace");
    }

    /// Where opened files go (`editor.open_files_in`): tabs by default.
    pub fn open_target(&self) -> OpenTarget {
        match self.editor().get("open_files_in").and_then(Value::as_str) {
            Some("window") => OpenTarget::Window,
            _ => OpenTarget::Tab,
        }
    }

    pub fn set_open_target(&mut self, target: OpenTarget) {
        self.editor_mut()
            .insert("open_files_in".into(), Value::String(target.as_str().into()));
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

/// Gets `key` in `map` as an object, replacing any mistyped value.
fn ensure_object<'a>(
    map: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    if !map.get(key).map(Value::is_object).unwrap_or(false) {
        map.insert(key.into(), Value::Object(Map::new()));
    }
    map.get_mut(key)
        .and_then(Value::as_object_mut)
        .expect("just ensured an object")
}

/// Removes `key` if it (recursively) holds only empty objects, keeping
/// the hand-edited file free of husks.
fn prune_empty(map: &mut Map<String, Value>, key: &str) {
    fn is_effectively_empty(value: &Value) -> bool {
        match value.as_object() {
            Some(object) => object.values().all(is_effectively_empty),
            None => false,
        }
    }
    if map.get(key).is_some_and(is_effectively_empty) {
        map.remove(key);
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
    fn appearance_round_trips_and_tolerates_junk() {
        let path = temp_path("appearance.json");
        let (mut config, _) = Config::load(&path);
        assert_eq!(config.appearance(), Appearance::System);

        config.set_appearance(Appearance::Dark);
        config.save().unwrap();
        let (reloaded, _) = Config::load(&path);
        assert_eq!(reloaded.appearance(), Appearance::Dark);

        // Back to system removes the key entirely.
        config.set_appearance(Appearance::System);
        config.save().unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("appearance"));

        std::fs::write(&path, r#"{"appearance": "solarized-disco"}"#).unwrap();
        let (weird, warning) = Config::load(&path);
        assert!(warning.is_none());
        assert_eq!(weird.appearance(), Appearance::System);
    }

    #[test]
    fn preprocessors_round_trip_and_resolution() {
        let path = temp_path("preprocessors.json");
        let (mut config, _) = Config::load(&path);
        assert_eq!(config.preprocessors_json(), "{}");
        assert!(config.preprocessor_commands(None, "python").is_empty());

        config.set_preprocessor_entry(None, "python", Some("ruff check --fix -\nblack -"));
        config.set_preprocessor_entry(Some("/work/projA"), "python", Some("black -"));
        config.save().unwrap();

        let (reloaded, _) = Config::load(&path);
        assert_eq!(
            reloaded.preprocessor_commands(None, "python"),
            vec!["ruff check --fix -", "black -"]
        );
        // The project entry replaces the defaults, not appends to them.
        assert_eq!(
            reloaded.preprocessor_commands(Some("/work/projA"), "python"),
            vec!["black -"]
        );
        // A root without its own entry falls back to the defaults.
        assert_eq!(
            reloaded.preprocessor_commands(Some("/elsewhere"), "python"),
            vec!["ruff check --fix -", "black -"]
        );

        // A hand-written plain string counts as a one-command chain.
        let mut by_hand = reloaded;
        by_hand
            .root
            .as_object_mut()
            .unwrap()
            .insert(
                "preprocessors".into(),
                serde_json::json!({"defaults": {"go": "gofmt"}}),
            );
        assert_eq!(by_hand.preprocessor_commands(None, "go"), vec!["gofmt"]);

        // Blank removes; empty sections prune away.
        let (mut config, _) = Config::load(&path);
        config.set_preprocessor_entry(None, "python", None);
        config.set_preprocessor_entry(Some("/work/projA"), "python", Some("  "));
        assert_eq!(config.preprocessors_json(), "{}");
    }

    #[test]
    fn spell_language_round_trip() {
        let path = temp_path("spell.json");
        let (mut config, _) = Config::load(&path);
        assert_eq!(config.spell_language(), None);
        config.set_spell_language(Some("es_ES"));
        config.save().unwrap();
        let (mut reloaded, _) = Config::load(&path);
        assert_eq!(reloaded.spell_language().as_deref(), Some("es_ES"));
        reloaded.set_spell_language(None);
        assert_eq!(reloaded.spell_language(), None);
    }

    #[test]
    fn lsp_entries_round_trip_defaults_and_projects() {
        let path = temp_path("lsp.json");
        let (mut config, _) = Config::load(&path);
        assert_eq!(config.lsp_json(), "{}");

        config.set_lsp_entry(None, "python", Some("pylsp"));
        config.set_lsp_entry(Some("/work/projA"), "python", Some("pyright-langserver --stdio"));
        config.save().unwrap();

        let (reloaded, _) = Config::load(&path);
        let lsp: Value = serde_json::from_str(&reloaded.lsp_json()).unwrap();
        assert_eq!(lsp["defaults"]["python"], "pylsp");
        assert_eq!(
            lsp["projects"]["/work/projA"]["python"],
            "pyright-langserver --stdio"
        );

        // Removing entries prunes empty sections away entirely.
        let mut config = reloaded;
        config.set_lsp_entry(None, "python", None);
        config.set_lsp_entry(Some("/work/projA"), "python", None);
        assert_eq!(config.lsp_json(), "{}");
        config.save().unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("lsp"));
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
