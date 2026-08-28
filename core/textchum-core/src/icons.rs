//! File icons from a VS Code icon pack.
//!
//! The file tree draws an icon per row. Without a pack it is whatever
//! the desktop offers for the file's type, which knows Python from
//! Markdown and almost nothing else — and has never heard of a file
//! called `Dockerfile`. A pack knows hundreds, and people arrive with
//! one they already use.
//!
//! ## The format
//!
//! A VS Code icon theme is one JSON file plus the images it names:
//!
//! ```json
//! {
//!   "iconDefinitions": { "_rust": { "iconPath": "./icons/rust.svg" } },
//!   "fileExtensions": { "rs": "_rust" },
//!   "fileNames": { "cargo.toml": "_cargo" },
//!   "languageIds": { "rust": "_rust" },
//!   "file": "_default",
//!   "light": { "fileExtensions": { "rs": "_rust_light" } }
//! }
//! ```
//!
//! Lookup order is VS Code's, most specific first: the whole file name,
//! then the longest extension that matches (`component.test.ts` tries
//! `test.ts` before `ts`), then the language the editor decided the
//! file is, then the default. Names and extensions are matched
//! lowercased, as VS Code matches them.
//!
//! `light` overrides any of those for a light appearance, one lookup at
//! a time — a pack that only redraws a handful for light backgrounds
//! keeps the rest.
//!
//! ## What is not read
//!
//! Folder icons: the tree draws its own, and a pack's folder art is a
//! second decision that has nothing to do with knowing a `.rs` from a
//! `.toml`.
//!
//! Font-based definitions — `{"fontCharacter": "\\e001", "fontId": ...}`,
//! which is how Seti and its descendants work — need the icon font
//! installed and a text run where an image goes. A pack with nothing
//! but those is refused with that as the reason, rather than loaded to
//! draw nothing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// One appearance's worth of lookups.
#[derive(Debug, Default, Clone)]
struct Lookups {
    /// Whole file names, lowercased.
    names: HashMap<String, String>,
    /// Extensions without the dot, lowercased.
    extensions: HashMap<String, String>,
    /// Language names as the editor knows them.
    languages: HashMap<String, String>,
    /// The icon for a file nothing else matched.
    default_file: Option<String>,
}

impl Lookups {
    fn read(value: &serde_json::Value) -> Self {
        Self {
            names: string_map(value.get("fileNames"), true),
            extensions: string_map(value.get("fileExtensions"), true),
            languages: string_map(value.get("languageIds"), false),
            default_file: value
                .get("file")
                .and_then(|id| id.as_str())
                .map(str::to_owned),
        }
    }
}

/// A loaded pack: the lookups, and where each definition's image is.
#[derive(Debug, Clone)]
pub struct IconPack {
    /// The pack's own file, for reporting which one is loaded.
    pub path: PathBuf,
    /// Definition id to the image on disk. Font-only definitions are
    /// absent, so an entry pointing at one falls through to the next
    /// lookup and finally to nothing.
    images: HashMap<String, PathBuf>,
    dark: Lookups,
    light: Lookups,
    /// How many definitions named a font character instead of an image.
    pub font_only: usize,
}

impl IconPack {
    /// Reads the pack whose JSON is at `path`. Image paths are resolved
    /// against the JSON's own directory, which is where a pack keeps
    /// them.
    pub fn load(path: &Path) -> Result<Self, String> {
        let path = resolve_pack_path(path)?;
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&crate::theme_import::strip_jsonc(&text))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let root = path.parent().unwrap_or(Path::new(".")).to_owned();

        let mut images = HashMap::new();
        let mut font_only = 0;
        if let Some(definitions) = value.get("iconDefinitions").and_then(|d| d.as_object()) {
            for (id, definition) in definitions {
                match definition.get("iconPath").and_then(|p| p.as_str()) {
                    Some(relative) => {
                        images.insert(id.clone(), root.join(relative.trim_start_matches("./")));
                    }
                    None => {
                        if definition.get("fontCharacter").is_some() {
                            font_only += 1;
                        }
                    }
                }
            }
        }
        if images.is_empty() {
            return Err(if font_only > 0 {
                format!(
                    "{} draws its {font_only} icons with an icon font rather than images, \
                     which Textchum cannot render",
                    path.display()
                )
            } else {
                format!("{} has no icon definitions", path.display())
            });
        }

        Ok(Self {
            dark: Lookups::read(&value),
            light: value
                .get("light")
                .map(Lookups::read)
                .unwrap_or_default(),
            path,
            images,
            font_only,
        })
    }

    /// The icon for a file, or `None` when the pack has nothing to say.
    ///
    /// `language` is what the editor decided the file is, which catches
    /// the files a pack lists by language rather than by name — and the
    /// ones a reader told Textchum about through File Properties.
    pub fn icon_for(&self, filename: &str, language: Option<&str>, light: bool) -> Option<&Path> {
        let name = filename.to_lowercase();
        for lookups in self.order(light) {
            if let Some(image) = lookups
                .names
                .get(&name)
                .and_then(|id| self.images.get(id))
            {
                return Some(image);
            }
        }
        // `component.test.ts` before `ts`: a pack that distinguishes
        // tests said so with the longer one.
        for extension in extensions_of(&name) {
            for lookups in self.order(light) {
                if let Some(image) = lookups
                    .extensions
                    .get(extension)
                    .and_then(|id| self.images.get(id))
                {
                    return Some(image);
                }
            }
        }
        if let Some(language) = language {
            for lookups in self.order(light) {
                if let Some(image) = lookups
                    .languages
                    .get(language)
                    .and_then(|id| self.images.get(id))
                {
                    return Some(image);
                }
            }
        }
        self.order(light)
            .into_iter()
            .find_map(|lookups| {
                lookups
                    .default_file
                    .as_ref()
                    .and_then(|id| self.images.get(id))
            })
            .map(PathBuf::as_path)
    }

    /// The lookups to consult, in order: the light overrides first when
    /// the appearance is light, then the pack's own.
    fn order(&self, light: bool) -> Vec<&Lookups> {
        if light {
            vec![&self.light, &self.dark]
        } else {
            vec![&self.dark]
        }
    }

    /// How many files the pack can name, for saying what was loaded.
    pub fn entry_count(&self) -> usize {
        self.dark.names.len() + self.dark.extensions.len() + self.dark.languages.len()
    }
}

/// The JSON to read for a pack at `path`: the file itself, or the one a
/// VS Code extension directory says it contributes.
fn resolve_pack_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_file() {
        return Ok(path.to_owned());
    }
    if !path.is_dir() {
        return Err(format!("{} is not there", path.display()));
    }
    let manifest = path.join("package.json");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|_| format!("{} holds no icon theme", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&crate::theme_import::strip_jsonc(&text))
        .map_err(|error| format!("{}: {error}", manifest.display()))?;
    value
        .get("contributes")
        .and_then(|c| c.get("iconThemes"))
        .and_then(|themes| themes.as_array())
        .and_then(|themes| themes.first())
        .and_then(|theme| theme.get("path")?.as_str())
        .map(|relative| path.join(relative.trim_start_matches("./")))
        .filter(|file| file.is_file())
        .ok_or_else(|| format!("{} contributes no icon theme", path.display()))
}

/// The extensions of a file name, longest first: `a.test.ts` gives
/// `test.ts` then `ts`.
fn extensions_of(name: &str) -> Vec<&str> {
    let mut extensions = Vec::new();
    let mut rest = name;
    // A leading dot is part of the name (`.gitignore`), not an
    // extension marker.
    let start = usize::from(name.starts_with('.'));
    while let Some(at) = rest[start..].find('.') {
        rest = &rest[start + at + 1..];
        if !rest.is_empty() {
            extensions.push(rest);
        }
    }
    extensions
}

/// Reads a `{string: string}` object, lowercasing the keys when asked.
fn string_map(value: Option<&serde_json::Value>, lowercase: bool) -> HashMap<String, String> {
    let Some(object) = value.and_then(|v| v.as_object()) else {
        return HashMap::new();
    };
    object
        .iter()
        .filter_map(|(key, id)| {
            let id = id.as_str()?.to_owned();
            Some((if lowercase { key.to_lowercase() } else { key.clone() }, id))
        })
        .collect()
}

/// The pack in effect, if one is. Kept here for the same reason the
/// theme is: both shells ask the same question and must get the same
/// answer.
static ACTIVE: RwLock<Option<IconPack>> = RwLock::new(None);

/// Loads the pack at `path` and makes it the one [`icon_for`] answers
/// from. Returns what to say about it, or why it could not be used.
pub fn set_active_from(path: &Path) -> Result<String, String> {
    let pack = IconPack::load(path)?;
    let summary = format!(
        "{} file types from {}",
        pack.entry_count(),
        pack.path.display()
    );
    let note = if pack.font_only > 0 {
        format!(
            "{summary} ({} of its icons are drawn with an icon font and are left out)",
            pack.font_only
        )
    } else {
        summary
    };
    *ACTIVE.write().expect("icon pack lock") = Some(pack);
    Ok(note)
}

/// Forgets the pack, returning the tree to the desktop's own icons.
pub fn clear_active() {
    *ACTIVE.write().expect("icon pack lock") = None;
}

/// Whether a pack is loaded.
pub fn is_active() -> bool {
    ACTIVE.read().expect("icon pack lock").is_some()
}

/// The icon for a file from the active pack, as a path to an image.
pub fn icon_for(filename: &str, language: Option<&str>, light: bool) -> Option<PathBuf> {
    ACTIVE
        .read()
        .expect("icon pack lock")
        .as_ref()
        .and_then(|pack| pack.icon_for(filename, language, light))
        .map(Path::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_pack(tag: &str, json: &str, images: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "textchum-icons-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("icons")).unwrap();
        for image in images {
            std::fs::write(dir.join("icons").join(image), "<svg/>").unwrap();
        }
        let file = dir.join("icons.json");
        std::fs::write(&file, json).unwrap();
        file
    }

    const PACK: &str = r#"{
        "iconDefinitions": {
            "_rust": {"iconPath": "./icons/rust.svg"},
            "_test": {"iconPath": "./icons/test.svg"},
            "_cargo": {"iconPath": "icons/cargo.svg"},
            "_docker": {"iconPath": "./icons/docker.svg"},
            "_default": {"iconPath": "./icons/default.svg"},
            "_light_rust": {"iconPath": "./icons/rust-light.svg"},
            "_fonted": {"fontCharacter": "", "fontId": "seti"}
        },
        "fileExtensions": {"rs": "_rust", "test.ts": "_test", "TS": "_fonted"},
        "fileNames": {"cargo.toml": "_cargo", "Dockerfile": "_docker"},
        "languageIds": {"toml": "_cargo"},
        "file": "_default",
        "light": {"fileExtensions": {"rs": "_light_rust"}}
    }"#;

    /// Each test gets its own directory: they run at once, and a
    /// shared one gets wiped out from under whichever is slower.
    fn loaded(tag: &str) -> IconPack {
        let file = temp_pack(
            tag,
            PACK,
            &[
                "rust.svg",
                "test.svg",
                "cargo.svg",
                "docker.svg",
                "default.svg",
                "rust-light.svg",
            ],
        );
        IconPack::load(&file).unwrap()
    }

    fn name_of(path: Option<&Path>) -> Option<&str> {
        path.and_then(|p| p.file_name()).and_then(|n| n.to_str())
    }

    #[test]
    fn an_extension_finds_its_icon() {
        let pack = loaded("an_extension_finds_its_icon");
        assert_eq!(name_of(pack.icon_for("main.rs", None, false)), Some("rust.svg"));
    }

    #[test]
    fn a_whole_file_name_beats_its_extension() {
        let pack = loaded("a_whole_file_name_beats_its_extension");
        // Cargo.toml would match no extension rule, but does match a
        // name rule — and matching is case-insensitive.
        assert_eq!(
            name_of(pack.icon_for("Cargo.toml", None, false)),
            Some("cargo.svg")
        );
        // A file with no extension at all is still known by name.
        assert_eq!(
            name_of(pack.icon_for("dockerfile", None, false)),
            Some("docker.svg")
        );
    }

    #[test]
    fn the_longest_extension_wins() {
        let pack = loaded("the_longest_extension_wins");
        assert_eq!(
            name_of(pack.icon_for("button.test.ts", None, false)),
            Some("test.svg")
        );
    }

    #[test]
    fn the_language_answers_when_the_name_does_not() {
        let pack = loaded("the_language_answers_when_the_name_does_not");
        // A file the reader told Textchum is TOML, whatever it is
        // called.
        assert_eq!(
            name_of(pack.icon_for("config.conf", Some("toml"), false)),
            Some("cargo.svg")
        );
    }

    #[test]
    fn nothing_matching_falls_to_the_pack_s_own_default() {
        let pack = loaded("nothing_matching_falls_to_the_pack_s_own_default");
        assert_eq!(
            name_of(pack.icon_for("notes.xyz", None, false)),
            Some("default.svg")
        );
    }

    #[test]
    fn a_light_appearance_takes_the_light_override_and_keeps_the_rest() {
        let pack = loaded("a_light_appearance_takes_the_light_override_and_keeps_the_rest");
        assert_eq!(
            name_of(pack.icon_for("main.rs", None, true)),
            Some("rust-light.svg")
        );
        // Not overridden for light, so the pack's own icon stands.
        assert_eq!(
            name_of(pack.icon_for("Cargo.toml", None, true)),
            Some("cargo.svg")
        );
    }

    #[test]
    fn an_entry_drawn_with_a_font_falls_through_rather_than_drawing_nothing() {
        let pack = loaded("an_entry_drawn_with_a_font_falls_through_rather_than_drawing_nothing");
        // `.ts` points at a font-only definition; the default answers
        // instead of an empty square.
        assert_eq!(
            name_of(pack.icon_for("plain.ts", None, false)),
            Some("default.svg")
        );
        assert_eq!(pack.font_only, 1);
    }

    #[test]
    fn a_pack_of_nothing_but_font_icons_is_refused_with_that_as_the_reason() {
        let file = temp_pack(
            "fonted",
            r#"{"iconDefinitions": {"_a": {"fontCharacter": ""}},
                "fileExtensions": {"rs": "_a"}}"#,
            &[],
        );
        let error = IconPack::load(&file).unwrap_err();
        assert!(error.contains("icon font"), "{error}");
    }

    #[test]
    fn an_extension_folder_is_asked_what_it_contributes() {
        let file = temp_pack("extension", PACK, &["rust.svg", "default.svg"]);
        let dir = file.parent().unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"contributes": {"iconThemes": [
                {"id": "pack", "path": "./icons.json"}]}}"#,
        )
        .unwrap();
        let pack = IconPack::load(dir).unwrap();
        assert_eq!(name_of(pack.icon_for("main.rs", None, false)), Some("rust.svg"));
    }

    #[test]
    fn a_folder_with_no_icon_theme_says_so() {
        let file = temp_pack("bare", PACK, &["rust.svg"]);
        let dir = file.parent().unwrap().join("nothing-here");
        std::fs::create_dir_all(&dir).unwrap();
        let error = IconPack::load(&dir).unwrap_err();
        assert!(error.contains("no icon theme"), "{error}");
    }

    #[test]
    fn extensions_come_out_longest_first() {
        assert_eq!(extensions_of("a.test.ts"), vec!["test.ts", "ts"]);
        assert_eq!(extensions_of("plain"), Vec::<&str>::new());
        // A dotfile's name is the whole thing, not an extension.
        assert_eq!(extensions_of(".gitignore"), Vec::<&str>::new());
        assert_eq!(extensions_of(".eslintrc.json"), vec!["json"]);
    }
}
