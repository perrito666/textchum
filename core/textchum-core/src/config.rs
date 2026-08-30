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

/// The hidden-glob presets the settings UI offers out of the box —
/// the ecosystems that scatter build residue through a project.
pub const BUILTIN_HIDE_PRESETS: &[(&str, &[&str])] = &[
    ("Version control", &[".git", ".hg", ".svn"]),
    (
        "Python",
        &["__pycache__", "*.pyc", ".venv", ".mypy_cache", ".ruff_cache"],
    ),
    ("Node", &["node_modules", ".next", "dist"]),
    ("Rust", &["target"]),
    ("Editor noise", &[".DS_Store", "*.swp"]),
];
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

/// What a document has been told about itself, overriding what its
/// name implies: which language it is, how wide a tab is, whether
/// indents are tabs or spaces. An absent field means the usual answer
/// applies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileOverride {
    pub language: Option<String>,
    pub tab_width: Option<u32>,
    /// True for spaces, false for tabs.
    pub spaces: Option<bool>,
}

impl FileOverride {
    /// Whether this says anything at all. An override with nothing in
    /// it is stored as no override.
    pub fn is_empty(&self) -> bool {
        self.language.is_none() && self.tab_width.is_none() && self.spaces.is_none()
    }
}

/// How many documents keep their overrides. Deep enough to cover the
/// files somebody works on for weeks, shallow enough that the
/// configuration stays a file a person can read. The least recently
/// set drops out.
pub const FILE_OVERRIDE_MEMORY: usize = 200;

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

    /// Re-reads the file, replacing in-memory state — for following
    /// external edits while running. Same warning contract as
    /// [`Config::load`]; unknown keys and hand edits arrive intact
    /// because the whole document is replaced.
    pub fn reload(&mut self) -> Option<String> {
        let (fresh, warning) = Self::load(&self.path);
        self.root = fresh.root;
        self.broken_on_disk = fresh.broken_on_disk;
        warning
    }

    /// Per-project editor overrides (`workspace.projects.<root>.editor`):
    /// an object with any of `font_family`, `font_size`, `tab_width`.
    /// `{}` when the root has none.
    pub fn editor_overrides_json(&self, root: &str) -> String {
        self.root
            .get("workspace")
            .and_then(|w| w.get("projects"))
            .and_then(|p| p.get(root))
            .and_then(|entry| entry.get("editor"))
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".into())
    }

    /// Sets (or, with `None`/invalid JSON, removes) one per-project
    /// editor override. `value_json` is a JSON value — `13.5`,
    /// `"Menlo"`. Empty objects prune away.
    pub fn set_editor_override(&mut self, root: &str, key: &str, value_json: Option<&str>) {
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        let entry = ensure_object(
            ensure_object(ensure_object(top, "workspace"), "projects"),
            root,
        );
        let editor = ensure_object(entry, "editor");
        match value_json.and_then(|json| serde_json::from_str::<Value>(json).ok()) {
            Some(value) => {
                editor.insert(key.into(), value);
            }
            None => {
                editor.remove(key);
            }
        }
        // Prune the editor object, then any now-empty ancestors.
        if editor.is_empty() {
            entry.remove("editor");
        }
        let projects = top
            .get_mut("workspace")
            .and_then(Value::as_object_mut)
            .and_then(|w| w.get_mut("projects"))
            .and_then(Value::as_object_mut);
        if let Some(projects) = projects {
            if projects.get(root).and_then(Value::as_object).is_some_and(|o| o.is_empty()) {
                projects.remove(root);
            }
        }
        if let Some(workspace) = top.get_mut("workspace").and_then(Value::as_object_mut) {
            if workspace
                .get("projects")
                .and_then(Value::as_object)
                .is_some_and(|p| p.is_empty())
            {
                workspace.remove("projects");
            }
        }
        prune_empty(top, "workspace");
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

    /// The `languages` section, as the grammar loader wants it: the
    /// whole document would do, and this is the part of it that says
    /// which grammars to open.
    pub fn grammars_json(&self) -> String {
        match self.root.get("languages") {
            Some(section) => format!("{{\"languages\":{section}}}"),
            None => "{}".into(),
        }
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

    /// The navigator's hidden-name globs for a project root: the
    /// root's own `hide` list when it has one, the `workspace.hide`
    /// defaults otherwise, and `[".*"]` (dotfiles hidden) when nothing
    /// is configured — so showing hidden files is just an emptier list.
    /// A project entry replaces the defaults, never appends.
    pub fn hide_globs(&self, root: Option<&str>) -> Vec<String> {
        let workspace = self.root.get("workspace");
        let list_at = |value: Option<&Value>| -> Option<Vec<String>> {
            value?.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
        };
        root.and_then(|root| {
            list_at(
                workspace
                    .and_then(|w| w.get("projects"))
                    .and_then(|p| p.get(root))
                    .and_then(|entry| entry.get("hide")),
            )
        })
        .or_else(|| list_at(workspace.and_then(|w| w.get("hide"))))
        .unwrap_or_else(|| vec![".*".to_owned()])
    }

    /// Sets (or, with `None`/blank, removes) the hidden-name globs —
    /// whitespace-separated — for a project root, or the defaults when
    /// `root` is `None`.
    pub fn set_hide_globs(&mut self, root: Option<&str>, globs: Option<&str>) {
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        let list: Vec<Value> = globs
            .unwrap_or_default()
            .split_whitespace()
            .map(|glob| Value::String(glob.to_owned()))
            .collect();
        match root {
            Some(root) => {
                let entry = ensure_object(
                    ensure_object(ensure_object(top, "workspace"), "projects"),
                    root,
                );
                if list.is_empty() {
                    entry.remove("hide");
                } else {
                    entry.insert("hide".into(), Value::Array(list));
                }
            }
            None => {
                let workspace = ensure_object(top, "workspace");
                if list.is_empty() {
                    workspace.remove("hide");
                } else {
                    workspace.insert("hide".into(), Value::Array(list));
                }
            }
        }
        prune_empty(top, "workspace");
    }

    /// The named hidden-glob presets offered by the settings UI, sorted
    /// by name. The built-ins below apply until the user edits any of
    /// them; from then on `workspace.hide_presets` is authoritative
    /// (so a deleted preset stays deleted), and clearing the section
    /// brings the built-ins back.
    pub fn hide_presets(&self) -> Vec<(String, Vec<String>)> {
        let stored = self
            .root
            .get("workspace")
            .and_then(|w| w.get("hide_presets"))
            .and_then(Value::as_object);
        let mut presets: Vec<(String, Vec<String>)> = match stored {
            Some(stored) => stored
                .iter()
                .map(|(name, globs)| {
                    let globs = globs
                        .as_array()
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default();
                    (name.clone(), globs)
                })
                .collect(),
            None => BUILTIN_HIDE_PRESETS
                .iter()
                .map(|(name, globs)| {
                    (
                        (*name).to_owned(),
                        globs.iter().map(|glob| (*glob).to_owned()).collect(),
                    )
                })
                .collect(),
        };
        presets.sort_by(|a, b| a.0.cmp(&b.0));
        presets
    }

    /// Sets (or, with `None`/blank, removes) one preset. Editing any
    /// preset materializes the whole set, so removals stick.
    pub fn set_hide_preset(&mut self, name: &str, globs: Option<&str>) {
        let current = self.hide_presets();
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        let section = ensure_object(ensure_object(top, "workspace"), "hide_presets");
        if section.is_empty() {
            for (existing, globs) in current {
                section.insert(
                    existing,
                    Value::Array(globs.into_iter().map(Value::String).collect()),
                );
            }
        }
        let list: Vec<Value> = globs
            .unwrap_or_default()
            .split_whitespace()
            .map(|glob| Value::String(glob.to_owned()))
            .collect();
        if list.is_empty() {
            section.remove(name);
        } else {
            section.insert(name.to_owned(), Value::Array(list));
        }
        // An emptied set would read as "no presets"; that is what the
        // user asked for, so keep it — only a reset restores built-ins.
        prune_empty(top, "workspace");
    }

    /// Forgets the user's presets, restoring the built-ins.
    pub fn reset_hide_presets(&mut self) {
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        if let Some(workspace) = top.get_mut("workspace").and_then(Value::as_object_mut) {
            workspace.remove("hide_presets");
        }
        prune_empty(top, "workspace");
    }

    /// Whether the navigator reveals the current file in the tree as
    /// focus moves (`editor.follow_file`, default true).
    pub fn follow_file(&self) -> bool {
        self.editor()
            .get("follow_file")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn set_follow_file(&mut self, enabled: bool) {
        self.editor_mut()
            .insert("follow_file".into(), Value::Bool(enabled));
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

    /// The spell-check setting read as the list it is allowed to be: a
    /// bilingual document needs both dictionaries at once, and the
    /// natural way to ask for that is `"en_US, es_ES"`. One dictionary
    /// is the one-element case, and `"auto"` stays a single entry
    /// meaning "whatever the desktop's locale says".
    pub fn spell_languages(&self) -> Vec<String> {
        self.spell_language()
            .into_iter()
            .flat_map(|value| {
                value
                    .split([',', ' '])
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The words the spell checker should accept regardless of
    /// dictionary (`editor.spell_words`): project names, acronyms, and
    /// the rest of the vocabulary no dictionary ships with. Sorted, so
    /// the file stays readable and diffs stay small.
    pub fn spell_words(&self) -> Vec<String> {
        self.editor()
            .get("spell_words")
            .and_then(Value::as_array)
            .map(|words| {
                words
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|word| !word.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_spell_words(&mut self, words: &[String]) {
        let mut sorted: Vec<String> = words
            .iter()
            .map(|word| word.trim().to_owned())
            .filter(|word| !word.is_empty())
            .collect();
        sorted.sort();
        sorted.dedup();
        if sorted.is_empty() {
            self.editor_mut().remove("spell_words");
            return;
        }
        self.editor_mut().insert(
            "spell_words".into(),
            Value::Array(sorted.into_iter().map(Value::String).collect()),
        );
    }

    /// Adds one word to the personal list. Returns whether it was new,
    /// so a caller can skip re-checking when nothing changed.
    pub fn add_spell_word(&mut self, word: &str) -> bool {
        let word = word.trim();
        if word.is_empty() {
            return false;
        }
        let mut words = self.spell_words();
        if words.iter().any(|existing| existing == word) {
            return false;
        }
        words.push(word.to_owned());
        self.set_spell_words(&words);
        true
    }

    /// How long the editor waits after the last keystroke before saving
    /// a file by itself (`editor.autosave`, seconds). Absent or zero
    /// means off, which is the default: a save can run preprocessors and
    /// rewrite the buffer, so it stays something the user asks for.
    /// What `path` has been told about itself, if anything.
    pub fn file_override(&self, path: &str) -> FileOverride {
        self.file_overrides()
            .into_iter()
            .find(|(stored, _)| stored == path)
            .map(|(_, entry)| entry)
            .unwrap_or_default()
    }

    /// Records what a document is, replacing anything said before about
    /// the same path and moving it to the front. An empty override
    /// removes the entry: telling a file to go back to what its name
    /// implies should leave nothing behind.
    pub fn set_file_override(&mut self, path: &str, entry: &FileOverride) {
        let mut entries: Vec<(String, FileOverride)> = self
            .file_overrides()
            .into_iter()
            .filter(|(stored, _)| stored != path)
            .collect();
        if !entry.is_empty() {
            entries.insert(0, (path.to_owned(), entry.clone()));
        }
        // Newest first, so the tail is what nobody has touched.
        entries.truncate(FILE_OVERRIDE_MEMORY);
        if entries.is_empty() {
            self.editor_mut().remove("files");
            return;
        }
        let array = entries
            .into_iter()
            .map(|(path, entry)| {
                let mut object = Map::new();
                object.insert("path".into(), Value::String(path));
                if let Some(language) = entry.language {
                    object.insert("language".into(), Value::String(language));
                }
                if let Some(width) = entry.tab_width {
                    object.insert("tab_width".into(), Value::Number(width.into()));
                }
                if let Some(spaces) = entry.spaces {
                    object.insert("spaces".into(), Value::Bool(spaces));
                }
                Value::Object(object)
            })
            .collect();
        self.editor_mut().insert("files".into(), Value::Array(array));
    }

    /// Every remembered override, newest first. An array rather than an
    /// object because the order is the recency, and that is what decides
    /// which one drops out when the list is full.
    pub fn file_overrides(&self) -> Vec<(String, FileOverride)> {
        self.editor()
            .get("files")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        let object = entry.as_object()?;
                        let path = object.get("path")?.as_str()?.to_owned();
                        Some((
                            path,
                            FileOverride {
                                language: object
                                    .get("language")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned),
                                tab_width: object
                                    .get("tab_width")
                                    .and_then(Value::as_u64)
                                    .map(|width| width as u32),
                                spaces: object.get("spaces").and_then(Value::as_bool),
                            },
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Untitled documents are never autosaved — there is nowhere to put
    /// them.
    pub fn autosave_seconds(&self) -> u32 {
        self.editor()
            .get("autosave")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32
    }

    pub fn set_autosave_seconds(&mut self, seconds: u32) {
        if seconds == 0 {
            self.editor_mut().remove("autosave");
            return;
        }
        self.editor_mut()
            .insert("autosave".into(), Value::Number(seconds.into()));
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

    /// The file-icon pack in use (`icon_pack`), as a path to a VS Code
    /// icon theme JSON or the extension folder holding one. Absent
    /// means the desktop's own icons.
    pub fn icon_pack(&self) -> Option<String> {
        self.root
            .get("icon_pack")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
    }

    /// Sets (or, with `None`, removes) the icon pack.
    pub fn set_icon_pack(&mut self, path: Option<&str>) {
        let root = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        match path.map(str::trim).filter(|path| !path.is_empty()) {
            Some(path) => {
                root.insert("icon_pack".into(), Value::String(path.to_owned()));
            }
            None => {
                root.remove("icon_pack");
            }
        }
    }

    /// Icon packs opened from outside Textchum's own folder
    /// (`icon_packs`), so they stay on the list once seen.
    pub fn known_icon_packs(&self) -> Vec<String> {
        self.root
            .get("icon_packs")
            .and_then(Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Remembers a pack opened from elsewhere. Already-known paths are
    /// left where they are rather than moved to the end.
    pub fn remember_icon_pack(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            return;
        }
        let mut known = self.known_icon_packs();
        if known.iter().any(|had| had == path) {
            return;
        }
        known.push(path.to_owned());
        self.write_icon_packs(known);
    }

    /// Forgets one — a pack that has been imported, or one whose folder
    /// is gone.
    pub fn forget_icon_pack(&mut self, path: &str) {
        let known: Vec<String> = self
            .known_icon_packs()
            .into_iter()
            .filter(|had| had != path)
            .collect();
        self.write_icon_packs(known);
    }

    fn write_icon_packs(&mut self, paths: Vec<String>) {
        let root = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        if paths.is_empty() {
            root.remove("icon_packs");
            return;
        }
        root.insert(
            "icon_packs".into(),
            Value::Array(paths.into_iter().map(Value::String).collect()),
        );
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

    /// Whether selecting a word marks its other occurrences on screen
    /// (`editor.mark_occurrences`, default true).
    pub fn mark_occurrences(&self) -> bool {
        self.editor()
            .get("mark_occurrences")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn set_mark_occurrences(&mut self, enabled: bool) {
        self.editor_mut()
            .insert("mark_occurrences".into(), Value::Bool(enabled));
    }

    /// How those occurrences are matched
    /// (`editor.occurrences_case_sensitive` and
    /// `editor.occurrences_whole_word`, both default true).
    pub fn occurrence_options(&self) -> crate::occurrences::Options {
        let defaults = crate::occurrences::Options::default();
        crate::occurrences::Options {
            case_sensitive: self
                .editor()
                .get("occurrences_case_sensitive")
                .and_then(Value::as_bool)
                .unwrap_or(defaults.case_sensitive),
            whole_word: self
                .editor()
                .get("occurrences_whole_word")
                .and_then(Value::as_bool)
                .unwrap_or(defaults.whole_word),
        }
    }

    pub fn set_occurrences_case_sensitive(&mut self, enabled: bool) {
        self.editor_mut()
            .insert("occurrences_case_sensitive".into(), Value::Bool(enabled));
    }

    pub fn set_occurrences_whole_word(&mut self, enabled: bool) {
        self.editor_mut()
            .insert("occurrences_whole_word".into(), Value::Bool(enabled));
    }

    /// Whether a project's record lives with the checkout
    /// (`<root>/.tchum`) instead of in the profile. Global on purpose:
    /// a per-project answer would have to be kept centrally to be
    /// found, which is the thing it was trying to avoid.
    pub fn project_state_in_project(&self) -> bool {
        self.editor()
            .get("project_state_in_project")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn set_project_state_in_project(&mut self, in_project: bool) {
        self.editor_mut()
            .insert("project_state_in_project".into(), Value::Bool(in_project));
    }

    /// Where the records are kept when they are not with the checkout.
    /// None means the profile's own folder.
    pub fn project_state_dir(&self) -> Option<String> {
        self.editor()
            .get("project_state_dir")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
    }

    pub fn set_project_state_dir(&mut self, dir: Option<&str>) {
        match dir.filter(|path| !path.is_empty()) {
            Some(path) => {
                self.editor_mut()
                    .insert("project_state_dir".into(), Value::String(path.to_owned()));
            }
            None => {
                self.editor_mut().remove("project_state_dir");
            }
        }
    }

    /// Whether the sweep runs at launch, and how long a record is kept
    /// after the last time it was written. Zero days keeps them until
    /// they are forgotten by hand; the roots that are gone still go.
    pub fn project_state_sweep(&self) -> bool {
        self.editor()
            .get("project_state_sweep")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn set_project_state_sweep(&mut self, sweep: bool) {
        self.editor_mut()
            .insert("project_state_sweep".into(), Value::Bool(sweep));
    }

    pub fn project_state_keep_days(&self) -> u32 {
        self.editor()
            .get("project_state_keep_days")
            .and_then(Value::as_u64)
            .unwrap_or(90) as u32
    }

    pub fn set_project_state_keep_days(&mut self, days: u32) {
        self.editor_mut()
            .insert("project_state_keep_days".into(), Value::Number(days.into()));
    }

    /// Whether a file stays open, whole, when the window showing it
    /// closes — the text that was never saved with it. What becomes of
    /// those files is settled when the editor itself closes.
    pub fn keep_buffers(&self) -> bool {
        self.editor()
            .get("keep_buffers")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn set_keep_buffers(&mut self, keep: bool) {
        self.editor_mut()
            .insert("keep_buffers".into(), Value::Bool(keep));
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

    /// The chosen keyboard profile (`keys_profile`); empty means the
    /// editor's own bindings.
    pub fn keys_profile(&self) -> String {
        self.root
            .get("keys_profile")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    pub fn set_keys_profile(&mut self, name: Option<&str>) {
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        match name.filter(|name| !name.is_empty()) {
            Some(name) => {
                top.insert("keys_profile".into(), Value::String(name.to_owned()));
            }
            None => {
                top.remove("keys_profile");
            }
        }
    }

    /// The profiles saved here (`key_profiles`), serialized: an object
    /// of name to action-to-shortcut map. `{}` when unset.
    pub fn key_profiles_json(&self) -> String {
        self.root
            .get("key_profiles")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "{}".into())
    }

    /// Saves (or, with `None`, removes) a profile. `bindings_json` is
    /// an action-to-shortcut object; anything else is ignored, so a
    /// malformed write cannot replace a working profile with rubbish.
    pub fn set_key_profile(&mut self, name: &str, bindings_json: Option<&str>) {
        if name.is_empty() {
            return;
        }
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        match bindings_json {
            Some(json) => {
                let Ok(Value::Object(bindings)) = serde_json::from_str::<Value>(json) else {
                    return;
                };
                ensure_object(top, "key_profiles")
                    .insert(name.into(), Value::Object(bindings));
            }
            None => {
                if let Some(profiles) = top.get_mut("key_profiles").and_then(Value::as_object_mut)
                {
                    profiles.remove(name);
                }
            }
        }
        prune_empty(top, "key_profiles");
    }

    /// Sets (or, with `None`, removes) one shortcut override (`keys`).
    pub fn set_key_binding(&mut self, action: &str, spec: Option<&str>) {
        if action.is_empty() {
            return;
        }
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        let keys = ensure_object(top, "keys");
        match spec.filter(|spec| !spec.is_empty()) {
            Some(spec) => {
                keys.insert(action.into(), Value::String(spec.to_owned()));
            }
            None => {
                keys.remove(action);
            }
        }
        prune_empty(top, "keys");
    }

    /// Forgets every shortcut override, returning to the profile's
    /// bindings — or, with no profile, to the editor's own.
    pub fn clear_key_bindings(&mut self) {
        self.root
            .as_object_mut()
            .expect("config root is always an object")
            .remove("keys");
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

    /// Every project root the configuration mentions, in any section.
    ///
    /// The Settings window offers these to copy from, and checks them
    /// against the disk: a root whose directory is gone is an entry
    /// nothing will ever match.
    pub fn configured_projects(&self) -> Vec<String> {
        let mut roots: Vec<String> = Vec::new();
        for section in ["workspace", "lsp", "preprocessors"] {
            let Some(projects) = self
                .root
                .get(section)
                .and_then(|value| value.get("projects"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            for root in projects.keys() {
                if !roots.iter().any(|had| had == root) {
                    roots.push(root.clone());
                }
            }
        }
        roots.sort();
        roots
    }

    /// Removes every trace of a project root: flags, editor overrides,
    /// hidden globs, servers and save commands.
    pub fn remove_project(&mut self, root: &str) {
        let top = self
            .root
            .as_object_mut()
            .expect("config root is always an object");
        for section in ["workspace", "lsp", "preprocessors"] {
            if let Some(projects) = top
                .get_mut(section)
                .and_then(Value::as_object_mut)
                .and_then(|section| section.get_mut("projects"))
                .and_then(Value::as_object_mut)
            {
                projects.remove(root);
            }
            prune_empty(top, section);
        }
    }

    /// Copies one project's settings onto another root.
    ///
    /// A second service in the same layout wants the same settings, and
    /// entering them again is a transcription exercise with a typo in
    /// it. Each part asked for replaces the target's, so what ends up
    /// there is the source's answer rather than a merge of two.
    ///
    /// Returns whether anything was copied — a source with no settings
    /// of its own copies nothing.
    pub fn copy_project(&mut self, from: &str, to: &str, parts: ProjectParts) -> bool {
        if from == to {
            return false;
        }
        let sections = [
            ("workspace", parts.workspace),
            ("lsp", parts.servers),
            ("preprocessors", parts.preprocessors),
        ];
        let mut copied = false;
        for (section, wanted) in sections {
            if !wanted {
                continue;
            }
            let source = self
                .root
                .get(section)
                .and_then(|value| value.get("projects"))
                .and_then(|projects| projects.get(from))
                .cloned();
            let Some(source) = source else { continue };
            let top = self
                .root
                .as_object_mut()
                .expect("config root is always an object");
            ensure_object(ensure_object(top, section), "projects").insert(to.into(), source);
            copied = true;
        }
        copied
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

    /// Where File → New puts the fresh document (`editor.new_files_in`):
    /// a tab of the frontmost window's group (the default), or a window
    /// of its own.
    pub fn new_file_target(&self) -> OpenTarget {
        match self.editor().get("new_files_in").and_then(Value::as_str) {
            Some("window") => OpenTarget::Window,
            _ => OpenTarget::Tab,
        }
    }

    pub fn set_new_file_target(&mut self, target: OpenTarget) {
        self.editor_mut()
            .insert("new_files_in".into(), Value::String(target.as_str().into()));
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
/// What a project's settings are made of, for copying one onto
/// another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectParts {
    /// `workspace.projects.<root>`: the flags, the editor overrides and
    /// the hidden globs.
    pub workspace: bool,
    /// `lsp.projects.<root>`: the servers.
    pub servers: bool,
    /// `preprocessors.projects.<root>`: the save commands.
    pub preprocessors: bool,
}

impl Default for ProjectParts {
    /// Everything: a project copied for its layout is wanted whole.
    fn default() -> Self {
        Self {
            workspace: true,
            servers: true,
            preprocessors: true,
        }
    }
}

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
    fn a_keyboard_profile_is_chosen_saved_and_overridden() {
        let (mut config, _) = Config::load(&temp_path("keys.json"));
        assert_eq!(config.keys_profile(), "");

        config.set_keys_profile(Some("vscode"));
        assert_eq!(config.keys_profile(), "vscode");

        config.set_key_binding("goToLine", Some("cmd+g"));
        let bindings = crate::keys::effective(
            &config.keys_profile(),
            &config.key_profiles_json(),
            &config.keys_json(),
        );
        assert_eq!(bindings.get("goToLine").map(String::as_str), Some("cmd+g"));
        assert_eq!(bindings.get("renameSymbol").map(String::as_str), Some("f2"));

        // Saving the result as a profile of its own, then leaving the
        // overrides behind.
        config.set_key_profile("mine", Some(&crate::keys::to_json(&bindings)));
        config.set_keys_profile(Some("mine"));
        config.clear_key_bindings();
        assert_eq!(config.keys_json(), "{}");
        let bindings = crate::keys::effective(
            &config.keys_profile(),
            &config.key_profiles_json(),
            &config.keys_json(),
        );
        assert_eq!(bindings.get("goToLine").map(String::as_str), Some("cmd+g"));

        // Rubbish never replaces a working profile.
        config.set_key_profile("mine", Some("not json"));
        assert!(config.key_profiles_json().contains("goToLine"));

        config.set_key_profile("mine", None);
        assert_eq!(config.key_profiles_json(), "{}");
    }

    #[test]
    fn a_project_can_be_copied_onto_another_root() {
        let (mut config, _) = Config::load(&temp_path("copy.json"));
        config.set_workspace_flag(Some("/work/a"), "ctags_fallback", Some(true));
        config.set_editor_override("/work/a", "tab_width", Some("2"));
        config.set_lsp_entry(Some("/work/a"), "python", Some("pylsp"));
        config.set_preprocessor_entry(Some("/work/a"), "python", Some("black -"));

        assert!(config.copy_project("/work/a", "/work/b", ProjectParts::default()));
        let workspace = config.workspace_json();
        assert!(workspace.contains("/work/b"));
        assert!(config.editor_overrides_json("/work/b").contains("tab_width"));
        assert!(config.lsp_json().contains("/work/b"));
        assert!(config.preprocessors_json().contains("/work/b"));
    }

    #[test]
    fn copying_takes_only_the_parts_asked_for() {
        let (mut config, _) = Config::load(&temp_path("copy-parts.json"));
        config.set_workspace_flag(Some("/work/a"), "ctags_fallback", Some(true));
        config.set_lsp_entry(Some("/work/a"), "python", Some("pylsp"));

        let parts = ProjectParts {
            workspace: false,
            servers: true,
            preprocessors: false,
        };
        assert!(config.copy_project("/work/a", "/work/b", parts));
        assert!(config.lsp_json().contains("/work/b"));
        assert!(!config.workspace_json().contains("/work/b"));
    }

    #[test]
    fn copying_a_project_with_no_settings_copies_nothing() {
        let (mut config, _) = Config::load(&temp_path("copy-empty.json"));
        assert!(!config.copy_project("/work/a", "/work/b", ProjectParts::default()));
        // And a root cannot be copied onto itself.
        config.set_workspace_flag(Some("/work/a"), "ctags_fallback", Some(true));
        assert!(!config.copy_project("/work/a", "/work/a", ProjectParts::default()));
    }

    #[test]
    fn every_configured_project_is_listed_and_removable() {
        let (mut config, _) = Config::load(&temp_path("projects.json"));
        config.set_workspace_flag(Some("/work/b"), "ctags_fallback", Some(true));
        config.set_lsp_entry(Some("/work/a"), "python", Some("pylsp"));
        config.set_preprocessor_entry(Some("/work/c"), "python", Some("black -"));
        // Sorted, and each root once however many sections mention it.
        assert_eq!(
            config.configured_projects(),
            vec!["/work/a".to_string(), "/work/b".into(), "/work/c".into()]
        );

        config.remove_project("/work/a");
        assert_eq!(
            config.configured_projects(),
            vec!["/work/b".to_string(), "/work/c".into()]
        );
        assert!(!config.lsp_json().contains("/work/a"));
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
    fn reload_follows_external_edits() {
        let path = temp_path("reload.json");
        let (mut config, _) = Config::load(&path);
        config.set_tab_width(8);
        config.save().unwrap();
        std::fs::write(&path, r#"{"editor": {"tab_width": 3}, "kept": true}"#).unwrap();
        assert!(config.reload().is_none());
        assert_eq!(config.tab_width(), 3);
        // The replacement carried the hand-added key along.
        config.save().unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("kept"));
    }

    #[test]
    fn editor_overrides_round_trip_and_prune() {
        let path = temp_path("editor-overrides.json");
        let (mut config, _) = Config::load(&path);
        assert_eq!(config.editor_overrides_json("/work/projA"), "{}");
        config.set_editor_override("/work/projA", "tab_width", Some("2"));
        config.set_editor_override("/work/projA", "font_family", Some("\"Menlo\""));
        config.save().unwrap();
        let (mut reloaded, _) = Config::load(&path);
        let overrides: Value =
            serde_json::from_str(&reloaded.editor_overrides_json("/work/projA")).unwrap();
        assert_eq!(overrides["tab_width"], 2);
        assert_eq!(overrides["font_family"], "Menlo");
        // Removing both prunes the whole trail away.
        reloaded.set_editor_override("/work/projA", "tab_width", None);
        reloaded.set_editor_override("/work/projA", "font_family", None);
        assert_eq!(reloaded.editor_overrides_json("/work/projA"), "{}");
    }

    #[test]
    fn new_file_target_round_trip_defaults_to_tab() {
        let path = temp_path("new-target.json");
        let (mut config, _) = Config::load(&path);
        assert_eq!(config.new_file_target(), OpenTarget::Tab);
        config.set_new_file_target(OpenTarget::Window);
        config.save().unwrap();
        let (reloaded, _) = Config::load(&path);
        assert_eq!(reloaded.new_file_target(), OpenTarget::Window);
    }

    #[test]
    fn hide_presets_builtin_edit_and_reset() {
        let path = temp_path("presets.json");
        let (mut config, _) = Config::load(&path);
        let builtin = config.hide_presets();
        assert_eq!(builtin.len(), BUILTIN_HIDE_PRESETS.len());
        assert!(builtin.iter().any(|(name, globs)| name == "Rust"
            && globs == &vec!["target".to_owned()]));

        // Editing one materializes the set; deleting another sticks.
        config.set_hide_preset("Rust", Some("target target-wasm"));
        config.set_hide_preset("Node", None);
        config.save().unwrap();
        let (mut reloaded, _) = Config::load(&path);
        let presets = reloaded.hide_presets();
        assert!(presets.iter().any(|(name, globs)| name == "Rust"
            && globs == &vec!["target".to_owned(), "target-wasm".to_owned()]));
        assert!(!presets.iter().any(|(name, _)| name == "Node"));
        // Names come back sorted, so the UI does not shuffle.
        let names: Vec<&str> = presets.iter().map(|(name, _)| name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);

        reloaded.reset_hide_presets();
        assert_eq!(reloaded.hide_presets().len(), BUILTIN_HIDE_PRESETS.len());
    }

    #[test]
    fn hide_globs_default_replace_and_prune() {
        let path = temp_path("hide.json");
        let (mut config, _) = Config::load(&path);
        assert_eq!(config.hide_globs(None), vec![".*"]);
        assert_eq!(config.hide_globs(Some("/p")), vec![".*"]);
        config.set_hide_globs(None, Some(".* target"));
        config.set_hide_globs(Some("/p"), Some("node_modules"));
        config.save().unwrap();
        let (mut reloaded, _) = Config::load(&path);
        assert_eq!(reloaded.hide_globs(None), vec![".*", "target"]);
        // The project list replaces the defaults, never appends.
        assert_eq!(reloaded.hide_globs(Some("/p")), vec!["node_modules"]);
        assert_eq!(reloaded.hide_globs(Some("/other")), vec![".*", "target"]);
        reloaded.set_hide_globs(None, None);
        reloaded.set_hide_globs(Some("/p"), Some("  "));
        assert_eq!(reloaded.hide_globs(None), vec![".*"]);
        assert_eq!(reloaded.hide_globs(Some("/p")), vec![".*"]);
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
    fn spell_setting_reads_as_a_list_of_dictionaries() {
        let path = temp_path("spell-multi.json");
        let (mut config, _) = Config::load(&path);
        assert!(config.spell_languages().is_empty());
        config.set_spell_language(Some("en_US"));
        assert_eq!(config.spell_languages(), vec!["en_US"]);
        // Both separators a person would reach for, and stray spaces.
        config.set_spell_language(Some("en_US, es_ES"));
        assert_eq!(config.spell_languages(), vec!["en_US", "es_ES"]);
        config.set_spell_language(Some("en_US es_ES  fr_FR"));
        assert_eq!(config.spell_languages(), vec!["en_US", "es_ES", "fr_FR"]);
        // "auto" is one entry, not a dictionary name to split.
        config.set_spell_language(Some("auto"));
        assert_eq!(config.spell_languages(), vec!["auto"]);
    }

    #[test]
    fn personal_words_round_trip_sorted_and_deduplicated() {
        let path = temp_path("spell-words.json");
        let (mut config, _) = Config::load(&path);
        assert!(config.spell_words().is_empty());
        assert!(config.add_spell_word("SBX"));
        assert!(config.add_spell_word("Textchum"));
        // The same word twice is not an edit.
        assert!(!config.add_spell_word("SBX"));
        assert!(!config.add_spell_word("   "));
        config.save().unwrap();
        let (mut reloaded, _) = Config::load(&path);
        assert_eq!(reloaded.spell_words(), vec!["SBX", "Textchum"]);
        reloaded.set_spell_words(&[]);
        assert!(reloaded.spell_words().is_empty());
    }

    #[test]
    fn a_file_can_be_told_what_it_is() {
        let path = temp_path("file-overrides.json");
        let (mut config, _) = Config::load(&path);
        assert_eq!(config.file_override("/w/report.txt"), FileOverride::default());

        config.set_file_override(
            "/w/report.txt",
            &FileOverride {
                language: Some("sql".into()),
                tab_width: Some(2),
                spaces: Some(true),
            },
        );
        config.save().unwrap();
        let (mut reloaded, _) = Config::load(&path);
        let entry = reloaded.file_override("/w/report.txt");
        assert_eq!(entry.language.as_deref(), Some("sql"));
        assert_eq!(entry.tab_width, Some(2));
        assert_eq!(entry.spaces, Some(true));

        // Saying nothing about a file forgets it, so a document that
        // goes back to what its name implies leaves nothing behind.
        reloaded.set_file_override("/w/report.txt", &FileOverride::default());
        assert_eq!(reloaded.file_override("/w/report.txt"), FileOverride::default());
        assert!(reloaded.file_overrides().is_empty());
    }

    #[test]
    fn the_least_recently_set_override_drops_out() {
        let path = temp_path("file-override-cache.json");
        let (mut config, _) = Config::load(&path);
        for index in 0..FILE_OVERRIDE_MEMORY + 20 {
            config.set_file_override(
                &format!("/w/{index}.txt"),
                &FileOverride { language: Some("sql".into()), ..Default::default() },
            );
        }
        let stored = config.file_overrides();
        assert_eq!(stored.len(), FILE_OVERRIDE_MEMORY, "the list is capped");
        // The newest is kept and the oldest is gone.
        assert_eq!(stored[0].0, format!("/w/{}.txt", FILE_OVERRIDE_MEMORY + 19));
        assert_eq!(config.file_override("/w/0.txt"), FileOverride::default());

        // Setting one again makes it the newest, so it survives the
        // next round of eviction.
        let survivor = format!("/w/{}.txt", FILE_OVERRIDE_MEMORY);
        config.set_file_override(
            &survivor,
            &FileOverride { tab_width: Some(8), ..Default::default() },
        );
        assert_eq!(config.file_overrides()[0].0, survivor);
    }

    #[test]
    fn autosave_is_off_until_asked_for() {
        let path = temp_path("autosave.json");
        let (mut config, _) = Config::load(&path);
        assert_eq!(config.autosave_seconds(), 0);
        config.set_autosave_seconds(30);
        config.save().unwrap();
        let (mut reloaded, _) = Config::load(&path);
        assert_eq!(reloaded.autosave_seconds(), 30);
        reloaded.set_autosave_seconds(0);
        assert_eq!(reloaded.autosave_seconds(), 0);
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
