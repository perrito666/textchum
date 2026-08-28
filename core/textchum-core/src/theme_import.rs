//! Reading another editor's theme.
//!
//! Someone arriving from VS Code or TextMate already has colours they
//! like. Both editors describe them the same way underneath — a colour
//! per **TextMate scope**, `entity.name.function` and its relatives —
//! and differ only in the file that carries it: JSON with comments in
//! one, an XML property list in the other.
//!
//! What comes out is a theme of the kind [`crate::theme`] already
//! reads, written to the same directory as a hand-written one. An
//! imported theme is not a second kind of theme with its own bugs.
//!
//! ## Scopes to captures
//!
//! Scopes are a dotted hierarchy and a rule claims a whole subtree:
//! `keyword` covers `keyword.control.conditional`. [`SCOPE_MAP`] goes
//! the other way, naming the capture each scope prefix belongs to, and
//! the longest prefix that matches a scope wins — so a theme that
//! bothers to colour `keyword.control.loop` separately gets its
//! `repeat` colour, and one that only says `keyword` colours all of
//! them alike.
//!
//! ## One appearance at a time
//!
//! A theme from either editor is written for a light background or a
//! dark one, never both. Textchum's are written for both, so an import
//! fills the side the source declares and leaves the other at the
//! default palette. [`Imported::appearance`] says which side was
//! filled, so the shell can say so too rather than leaving someone to
//! discover it by switching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::fsutil::write_atomically;
// Only the tests hold the scope table against the capture table; the
// import itself works from the scope table alone.
#[cfg(test)]
use crate::syntax::theme::CAPTURES;

/// Which side of a Textchum theme an import can fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// A theme read from another editor, ready to be written out.
#[derive(Debug, Clone)]
pub struct Imported {
    pub name: String,
    /// The side of the palette this theme is written for.
    pub appearance: Appearance,
    /// Capture name to (`#RRGGBB`, bold, italic), for the captures the
    /// source had something to say about.
    pub styles: Vec<(String, ImportedStyle)>,
    /// Scopes the source coloured that nothing here answers to. Shown
    /// rather than swallowed: a scope with nowhere to go is a gap in
    /// [`SCOPE_MAP`], and a silent one never gets closed.
    pub unmapped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedStyle {
    pub color: Option<String>,
    pub bold: bool,
    pub italic: bool,
}

impl Imported {
    /// The theme as Textchum's own JSON: the imported side filled in,
    /// the other side left for the default palette to supply.
    pub fn to_theme_json(&self) -> String {
        let mut styles = serde_json::Map::new();
        for (capture, style) in &self.styles {
            let mut entry = serde_json::Map::new();
            if let Some(color) = &style.color {
                entry.insert(self.appearance.name().into(), color.clone().into());
            }
            entry.insert("bold".into(), style.bold.into());
            entry.insert("italic".into(), style.italic.into());
            styles.insert(capture.clone(), entry.into());
        }
        let mut theme = serde_json::Map::new();
        theme.insert("name".into(), self.name.clone().into());
        theme.insert("styles".into(), styles.into());
        serde_json::to_string_pretty(&serde_json::Value::Object(theme))
            .expect("theme serializes")
    }
}

/// Which editor a file came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    VsCode,
    TextMate,
}

impl Source {
    /// The name a shell shows in its menu and its file chooser.
    pub fn label(self) -> &'static str {
        match self {
            Self::VsCode => "VS Code",
            Self::TextMate => "TextMate",
        }
    }

    /// The extensions a theme of this kind is kept in.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::VsCode => &["json"],
            Self::TextMate => &["tmTheme", "thTheme"],
        }
    }
}

/// What an import did, for the shell to report.
#[derive(Debug, Default, Clone)]
pub struct Outcome {
    /// The names of the themes written, ready to be chosen.
    pub written: Vec<String>,
    /// Which side of the palette each one filled.
    pub appearances: Vec<String>,
    /// Scopes no capture answers to, across every theme read. A gap in
    /// [`SCOPE_MAP`] rather than a fault in the file, and shown so it
    /// can be closed.
    pub unmapped: Vec<String>,
    /// One line per file that could not be read, saying why.
    pub errors: Vec<String>,
}

impl Outcome {
    /// The outcome as JSON, which is how it crosses the FFI.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "written": self.written,
            "appearances": self.appearances,
            "unmapped": self.unmapped,
            "errors": self.errors,
        })
        .to_string()
    }
}

/// Imports every theme at `path` into `themes_dir`, one JSON file per
/// theme, named after the theme itself.
///
/// `path` is a theme file, or a folder to look inside — a VS Code
/// extension directory contributes its themes through `package.json`,
/// and a `.tmbundle` keeps them in `Themes/`. A folder holding several
/// brings all of them in: someone who installed a pack of six wanted
/// the pack, and picking one from a second dialog is a dialog nobody
/// asked for.
pub fn import_into(path: &Path, source: Source, themes_dir: &Path) -> Outcome {
    let mut outcome = Outcome::default();
    let files = discover(path, source);
    if files.is_empty() {
        outcome.errors.push(format!(
            "no {} theme found at {}",
            source.label(),
            path.display()
        ));
        return outcome;
    }
    for file in files {
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(error) => {
                outcome
                    .errors
                    .push(format!("{}: {error}", file.display()));
                continue;
            }
        };
        let imported = match source {
            Source::VsCode => from_vscode(&text),
            Source::TextMate => from_textmate(&text),
        };
        let imported = match imported {
            Ok(imported) => imported,
            Err(error) => {
                outcome
                    .errors
                    .push(format!("{}: {error}", file.display()));
                continue;
            }
        };
        let name = safe_file_name(&imported.name);
        let destination = themes_dir.join(format!("{name}.json"));
        if let Err(error) = std::fs::create_dir_all(themes_dir)
            .and_then(|()| write_atomically(&destination, imported.to_theme_json().as_bytes()))
        {
            outcome
                .errors
                .push(format!("{}: {error}", destination.display()));
            continue;
        }
        outcome.written.push(imported.name.clone());
        outcome
            .appearances
            .push(imported.appearance.name().to_owned());
        for scope in imported.unmapped {
            if !outcome.unmapped.contains(&scope) {
                outcome.unmapped.push(scope);
            }
        }
    }
    outcome.unmapped.sort();
    outcome
}

/// The theme files at `path`: the file itself, or the ones a folder
/// holds.
pub fn discover(path: &Path, source: Source) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_owned()];
    }
    if !path.is_dir() {
        return Vec::new();
    }
    match source {
        // An extension says what it contributes; the folder layout is
        // the extension author's business, not a convention to guess.
        Source::VsCode => {
            let manifest = path.join("package.json");
            let Ok(text) = std::fs::read_to_string(&manifest) else {
                return files_with_extensions(path, source);
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&strip_jsonc(&text)) else {
                return files_with_extensions(path, source);
            };
            let contributed: Vec<PathBuf> = value
                .get("contributes")
                .and_then(|c| c.get("themes"))
                .and_then(|themes| themes.as_array())
                .map(|themes| {
                    themes
                        .iter()
                        .filter_map(|theme| theme.get("path")?.as_str())
                        .map(|relative| path.join(relative.trim_start_matches("./")))
                        .filter(|file| file.is_file())
                        .collect()
                })
                .unwrap_or_default();
            if contributed.is_empty() {
                files_with_extensions(path, source)
            } else {
                contributed
            }
        }
        // A .tmbundle keeps its themes in one place.
        Source::TextMate => {
            let themes = path.join("Themes");
            if themes.is_dir() {
                files_with_extensions(&themes, source)
            } else {
                files_with_extensions(path, source)
            }
        }
    }
}

/// Files directly inside `directory` with one of the source's
/// extensions, in a stable order.
fn files_with_extensions(directory: &Path, source: Source) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().is_some_and(|extension| {
                    source
                        .extensions()
                        .iter()
                        .any(|wanted| extension.eq_ignore_ascii_case(wanted))
                })
        })
        .collect();
    files.sort();
    files
}

/// A theme's own name as a file name: a theme is chosen by the name of
/// the file holding it, so the file has to be able to carry the name,
/// and a name carrying a path separator has to not become a path.
fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "Imported".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Reads a VS Code colour theme: JSON, by convention with comments and
/// the occasional trailing comma, carrying `tokenColors` rules.
pub fn from_vscode(text: &str) -> Result<Imported, String> {
    let value: serde_json::Value =
        serde_json::from_str(&strip_jsonc(text)).map_err(|error| error.to_string())?;

    let name = value
        .get("name")
        .and_then(|name| name.as_str())
        .unwrap_or("Imported")
        .to_owned();
    let rules = value
        .get("tokenColors")
        .and_then(|rules| rules.as_array())
        .ok_or_else(|| "no \"tokenColors\" array — is this a colour theme?".to_owned())?;

    // `type` is the theme's own word for which background it expects.
    // Without one, the editor background it asks for decides, and
    // failing that a theme is assumed dark, as most are.
    let appearance = match value.get("type").and_then(|kind| kind.as_str()) {
        Some("light") => Appearance::Light,
        Some("dark") => Appearance::Dark,
        _ => value
            .get("colors")
            .and_then(|colors| colors.get("editor.background"))
            .and_then(|color| color.as_str())
            .map(appearance_of_background)
            .unwrap_or(Appearance::Dark),
    };

    let mut collector = Collector::default();
    for rule in rules {
        let settings = rule.get("settings");
        let color = settings
            .and_then(|s| s.get("foreground"))
            .and_then(|c| c.as_str());
        let font_style = settings
            .and_then(|s| s.get("fontStyle"))
            .and_then(|s| s.as_str())
            .unwrap_or("");
        // A scope is one string, a list of strings, or one string
        // holding a comma-separated list. All three occur in the wild.
        let scopes: Vec<String> = match rule.get("scope") {
            Some(serde_json::Value::String(scope)) => {
                scope.split(',').map(|s| s.trim().to_owned()).collect()
            }
            Some(serde_json::Value::Array(scopes)) => scopes
                .iter()
                .filter_map(|s| s.as_str())
                .map(|s| s.trim().to_owned())
                .collect(),
            _ => continue,
        };
        for scope in scopes {
            collector.add(&scope, color, font_style);
        }
    }
    Ok(collector.finish(name, appearance))
}

/// Reads a TextMate `.tmTheme`: an XML property list whose `settings`
/// array is the same idea in another shape.
pub fn from_textmate(text: &str) -> Result<Imported, String> {
    let plist = plist::parse(text)?;
    let root = plist.as_dict().ok_or_else(|| "not a plist dictionary".to_owned())?;
    let name = root
        .get("name")
        .and_then(plist::Value::as_str)
        .unwrap_or("Imported")
        .to_owned();
    let rules = root
        .get("settings")
        .and_then(plist::Value::as_array)
        .ok_or_else(|| "no \"settings\" array — is this a .tmTheme?".to_owned())?;

    // The first rule carries no scope: it is the editor's own colours,
    // and its background says which side of the palette this is.
    let appearance = rules
        .iter()
        .find(|rule| rule.as_dict().is_some_and(|d| !d.contains_key("scope")))
        .and_then(|rule| rule.as_dict())
        .and_then(|rule| rule.get("settings"))
        .and_then(plist::Value::as_dict)
        .and_then(|settings| settings.get("background"))
        .and_then(plist::Value::as_str)
        .map(appearance_of_background)
        .unwrap_or(Appearance::Dark);

    let mut collector = Collector::default();
    for rule in rules {
        let Some(rule) = rule.as_dict() else { continue };
        let Some(scope) = rule.get("scope").and_then(plist::Value::as_str) else {
            continue;
        };
        let settings = rule.get("settings").and_then(plist::Value::as_dict);
        let color = settings
            .and_then(|s| s.get("foreground"))
            .and_then(plist::Value::as_str);
        let font_style = settings
            .and_then(|s| s.get("fontStyle"))
            .and_then(plist::Value::as_str)
            .unwrap_or("");
        for scope in scope.split(',') {
            collector.add(scope.trim(), color, font_style);
        }
    }
    Ok(collector.finish(name, appearance))
}

/// Gathers rules, keeping for each capture the one whose scope was most
/// specific. Ties go to the later rule, which is how both editors
/// resolve them.
#[derive(Default)]
struct Collector {
    /// capture -> (specificity, style)
    best: HashMap<&'static str, (usize, ImportedStyle)>,
    unmapped: Vec<String>,
}

impl Collector {
    fn add(&mut self, scope: &str, color: Option<&str>, font_style: &str) {
        // A descendant selector ("meta.function entity.name") is scoped
        // by context; the part that names the thing is the last one.
        let scope = scope.split_whitespace().next_back().unwrap_or(scope);
        // A leading `-` excludes rather than selects.
        if scope.is_empty() || scope.starts_with('-') {
            return;
        }
        let Some((prefix, captures)) = longest_match(scope) else {
            if !self.unmapped.iter().any(|seen| seen == scope) {
                self.unmapped.push(scope.to_owned());
            }
            return;
        };
        let Some(color) = normalize_color(color) else { return };
        let style = ImportedStyle {
            color: Some(color),
            bold: font_style.contains("bold"),
            italic: font_style.contains("italic"),
        };
        let specificity = prefix.matches('.').count() + 1;
        for capture in captures {
            match self.best.get(capture) {
                Some((seen, _)) if *seen > specificity => {}
                _ => {
                    self.best.insert(capture, (specificity, style.clone()));
                }
            }
        }
    }

    /// Fills the captures the source drew no distinction for from the
    /// ones it did. No VS Code theme colours `if` differently from
    /// `while`, or a boolean differently from any other literal —
    /// TextMate scopes stop where Textchum's captures keep going. Left
    /// alone, an imported theme would paint half the keywords in its
    /// own colour and the other half in Textchum's, which reads as a
    /// bug rather than as a theme.
    ///
    /// [`FALLBACKS`] reads "a conditional is a kind of keyword", and it
    /// is followed both ways. Downwards is the obvious one: a theme
    /// that colours `keyword` colours every kind of keyword. Upwards
    /// matters just as much, because which member of a family a theme
    /// happens to name varies — one colours `constant`, the next only
    /// `constant.numeric`, and a family whose general term went unnamed
    /// would otherwise keep Textchum's colour beside the source's.
    fn inherit(&mut self) {
        // A fixpoint: filling a parent from a child gives that parent's
        // other children something to take, and the other way about.
        // Bounded by the table's depth, and by the table being finite.
        for _ in 0..FALLBACKS.len() {
            let mut changed = false;
            for (capture, parent) in FALLBACKS {
                if !self.best.contains_key(capture) {
                    if let Some((_, style)) = self.best.get(parent) {
                        // Specificity 0: an inherited colour gives way
                        // to any rule the source actually wrote.
                        let style = style.clone();
                        self.best.insert(capture, (0, style));
                        changed = true;
                    }
                } else if !self.best.contains_key(parent) {
                    if let Some((_, style)) = self.best.get(capture) {
                        let style = style.clone();
                        self.best.insert(parent, (0, style));
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn finish(mut self, name: String, appearance: Appearance) -> Imported {
        self.inherit();
        // Alphabetical, so re-importing the same theme writes the same
        // file and a diff between two imports is about the colours.
        let mut styles: Vec<(String, ImportedStyle)> = self
            .best
            .into_iter()
            .map(|(capture, (_, style))| (capture.to_owned(), style))
            .collect();
        styles.sort_by(|a, b| a.0.cmp(&b.0));
        let mut unmapped = self.unmapped;
        unmapped.sort();
        Imported {
            name,
            appearance,
            styles,
            unmapped,
        }
    }
}

/// The capture (or captures) each TextMate scope prefix belongs to.
/// Ordered by prefix so a reader can find one; matching is by length,
/// not by position.
///
/// Some scopes feed two captures: TextMate has no separate idea of a
/// heading in prose and a title, and no theme distinguishes them, so
/// both take the same colour rather than one of them going unstyled.
pub static SCOPE_MAP: &[(&str, &[&str])] = &[
    ("comment", &["comment"]),
    ("constant", &["constant"]),
    ("constant.character", &["character"]),
    ("constant.character.escape", &["escape"]),
    ("constant.language", &["constant.builtin"]),
    ("constant.language.boolean", &["boolean"]),
    ("constant.numeric", &["number"]),
    ("constant.numeric.float", &["float"]),
    ("constant.other.color", &["constant"]),
    ("entity.name.class", &["type"]),
    ("entity.name.function", &["function"]),
    ("entity.name.function.constructor", &["constructor"]),
    ("entity.name.function.preprocessor", &["include"]),
    ("entity.name.label", &["label"]),
    ("entity.name.module", &["module"]),
    ("entity.name.namespace", &["namespace"]),
    ("entity.name.section", &["markup.heading", "text.title"]),
    ("entity.name.tag", &["tag"]),
    ("entity.name.type", &["type"]),
    ("entity.other.attribute-name", &["attribute"]),
    ("entity.other.inherited-class", &["type"]),
    ("invalid", &["error"]),
    ("invalid.deprecated", &["text.warning"]),
    ("keyword", &["keyword"]),
    ("keyword.control", &["keyword"]),
    ("keyword.control.at-rule.charset", &["charset"]),
    ("keyword.control.at-rule.keyframes", &["keyframes"]),
    ("keyword.control.at-rule.media", &["media"]),
    ("keyword.control.at-rule.supports", &["supports"]),
    ("keyword.control.conditional", &["conditional"]),
    ("keyword.control.exception", &["exception"]),
    ("keyword.control.flow", &["exception"]),
    ("keyword.control.import", &["include"]),
    ("keyword.control.loop", &["repeat"]),
    ("keyword.control.trycatch", &["exception"]),
    ("keyword.operator", &["operator"]),
    ("markup.bold", &["text.strong"]),
    ("markup.deleted", &["text.danger"]),
    ("markup.heading", &["markup.heading", "text.title"]),
    ("markup.inline.raw", &["text.literal"]),
    ("markup.italic", &["text.emphasis"]),
    ("markup.list", &["punctuation"]),
    ("markup.quote", &["text.note"]),
    ("markup.raw", &["text.literal"]),
    ("markup.underline.link", &["markup.link", "text.uri"]),
    ("meta.decorator", &["attribute"]),
    ("meta.object-literal.key", &["property"]),
    ("meta.preprocessor", &["include"]),
    ("punctuation", &["punctuation"]),
    ("punctuation.definition.template-expression", &["punctuation.special"]),
    ("punctuation.section.embedded", &["punctuation.special"]),
    ("punctuation.separator", &["delimiter"]),
    ("punctuation.terminator", &["delimiter"]),
    ("storage", &["storageclass"]),
    ("storage.modifier", &["storageclass"]),
    ("storage.type", &["type"]),
    ("string", &["string"]),
    ("string.regexp", &["string.special"]),
    ("support.class", &["type.builtin"]),
    ("support.constant", &["constant.builtin"]),
    ("support.function", &["function.builtin"]),
    ("support.type", &["type.builtin"]),
    ("support.type.property-name", &["property"]),
    ("support.variable", &["variable.builtin"]),
    ("variable", &["variable"]),
    ("variable.language", &["variable.builtin"]),
    ("variable.other.constant", &["constant"]),
    ("variable.other.member", &["field"]),
    ("variable.other.property", &["property"]),
    ("variable.parameter", &["parameter", "variable.parameter"]),
];

/// Where a capture takes its colour from when the source theme drew no
/// distinction: the capture it is a special case of. Read as "a
/// conditional is a kind of keyword".
pub static FALLBACKS: &[(&str, &str)] = &[
    ("boolean", "constant.builtin"),
    ("character", "constant"),
    ("charset", "keyword"),
    ("conditional", "keyword"),
    ("constant.builtin", "constant"),
    ("constructor", "function"),
    ("delimiter", "punctuation"),
    ("escape", "string.special"),
    ("exception", "keyword"),
    ("field", "property"),
    ("float", "number"),
    ("function.builtin", "function"),
    ("include", "keyword"),
    ("keyframes", "keyword"),
    ("label", "keyword"),
    ("media", "keyword"),
    ("module", "namespace"),
    ("namespace", "type"),
    ("number", "constant"),
    ("operator", "keyword"),
    ("parameter", "variable"),
    ("property", "variable"),
    ("punctuation.special", "punctuation"),
    ("repeat", "keyword"),
    ("storageclass", "keyword"),
    ("string.special", "string"),
    ("supports", "keyword"),
    ("text.danger", "error"),
    ("text.literal", "string"),
    ("text.note", "comment"),
    ("text.reference", "markup.link"),
    ("text.title", "markup.heading"),
    ("text.uri", "markup.link"),
    ("text.warning", "error"),
    ("type.builtin", "type"),
    ("variable.builtin", "variable"),
    ("variable.parameter", "parameter"),
];

/// The longest entry in [`SCOPE_MAP`] that `scope` sits under.
fn longest_match(scope: &str) -> Option<(&'static str, &'static [&'static str])> {
    SCOPE_MAP
        .iter()
        .filter(|(prefix, _)| {
            scope == *prefix
                || (scope.starts_with(prefix)
                    && scope.as_bytes().get(prefix.len()) == Some(&b'.'))
        })
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(prefix, captures)| (*prefix, *captures))
}

/// `#RGB`, `#RRGGBB` and `#RRGGBBAA` all become `#RRGGBB`: Textchum
/// keeps an alpha channel, but a syntax colour that is partly
/// transparent over an unknown background is a colour nobody chose.
fn normalize_color(color: Option<&str>) -> Option<String> {
    let text = color?.trim().strip_prefix('#')?;
    if !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let expanded = match text.len() {
        3 => text.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => text[..6].to_owned(),
        _ => return None,
    };
    Some(format!("#{}", expanded.to_uppercase()))
}

/// Whether a background colour is a light one, by perceived brightness.
fn appearance_of_background(color: &str) -> Appearance {
    let Some(hex) = normalize_color(Some(color)) else {
        return Appearance::Dark;
    };
    let value = u32::from_str_radix(&hex[1..], 16).unwrap_or(0);
    let (r, g, b) = (
        ((value >> 16) & 0xFF) as f32,
        ((value >> 8) & 0xFF) as f32,
        (value & 0xFF) as f32,
    );
    // Rec. 601 luma: green carries most of what the eye calls
    // brightness, blue almost none.
    if 0.299 * r + 0.587 * g + 0.114 * b > 128.0 {
        Appearance::Light
    } else {
        Appearance::Dark
    }
}

/// Removes comments and trailing commas from JSON-with-comments, which
/// is what VS Code writes and `serde_json` will not read.
pub(crate) fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        out.push(escaped);
                    }
                }
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                for c in chars.by_ref() {
                    if previous == '*' && c == '/' {
                        break;
                    }
                    previous = c;
                }
            }
            _ => out.push(c),
        }
    }
    remove_trailing_commas(&out)
}

fn remove_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(escaped) = chars.get(index + 1) {
                    out.push(*escaped);
                    index += 2;
                    continue;
                }
            } else if c == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            index += 1;
            continue;
        }
        if c == ',' {
            // A comma with nothing but space before the close is one
            // the writer left behind.
            let next = chars[index + 1..]
                .iter()
                .find(|c| !c.is_whitespace())
                .copied();
            if matches!(next, Some(']') | Some('}')) {
                index += 1;
                continue;
            }
        }
        out.push(c);
        index += 1;
    }
    out
}

/// Enough of an XML property list to read a `.tmTheme`: nested
/// dictionaries and arrays of strings. Data, dates and numbers appear
/// in plists at large but not in themes, and are read as strings or
/// skipped rather than pretended to be understood.
#[allow(dead_code)]
mod plist {
    // A reader that knew only the shapes a theme happens to use would
    // abort halfway through any file carrying one it did not, so the
    // rest are read and left for a caller that never asks.
    use std::collections::HashMap;

    #[derive(Debug, Clone)]
    pub enum Value {
        String(String),
        Bool(bool),
        Dict(HashMap<String, Value>),
        Array(Vec<Value>),
    }

    impl Value {
        pub fn as_str(&self) -> Option<&str> {
            match self {
                Self::String(text) => Some(text),
                _ => None,
            }
        }

        pub fn as_dict(&self) -> Option<&HashMap<String, Value>> {
            match self {
                Self::Dict(dict) => Some(dict),
                _ => None,
            }
        }

        pub fn as_bool(&self) -> Option<bool> {
            match self {
                Self::Bool(value) => Some(*value),
                _ => None,
            }
        }

        pub fn as_array(&self) -> Option<&Vec<Value>> {
            match self {
                Self::Array(items) => Some(items),
                _ => None,
            }
        }
    }

    pub fn parse(text: &str) -> Result<Value, String> {
        let mut parser = Parser {
            chars: text.chars().collect(),
            at: 0,
        };
        parser.skip_to_body()?;
        parser.value().ok_or_else(|| "no value in the plist".to_owned())
    }

    struct Parser {
        chars: Vec<char>,
        at: usize,
    }

    impl Parser {
        /// Skips the declaration, the doctype and the `<plist>` tag.
        fn skip_to_body(&mut self) -> Result<(), String> {
            let rest: String = self.chars[self.at..].iter().collect();
            let start = rest
                .find("<plist")
                .and_then(|at| rest[at..].find('>').map(|end| at + end + 1))
                .ok_or_else(|| "no <plist> element".to_owned())?;
            self.at += rest[..start].chars().count();
            Ok(())
        }

        fn value(&mut self) -> Option<Value> {
            let tag = self.next_tag()?;
            match tag.as_str() {
                "dict" => {
                    let mut dict = HashMap::new();
                    loop {
                        let tag = self.next_tag()?;
                        if tag == "/dict" {
                            return Some(Value::Dict(dict));
                        }
                        if tag != "key" {
                            return Some(Value::Dict(dict));
                        }
                        let key = self.text_until_close();
                        let value = self.value()?;
                        dict.insert(key, value);
                    }
                }
                "array" => {
                    let mut items = Vec::new();
                    loop {
                        let checkpoint = self.at;
                        let tag = self.next_tag()?;
                        if tag == "/array" {
                            return Some(Value::Array(items));
                        }
                        self.at = checkpoint;
                        items.push(self.value()?);
                    }
                }
                "string" => Some(Value::String(self.text_until_close())),
                "true/" => Some(Value::Bool(true)),
                "false/" => Some(Value::Bool(false)),
                "integer" | "real" | "data" | "date" => {
                    Some(Value::String(self.text_until_close()))
                }
                _ => None,
            }
        }

        /// The name of the next tag, with `<` and `>` stripped; a
        /// self-closing tag keeps its trailing slash.
        fn next_tag(&mut self) -> Option<String> {
            while self.at < self.chars.len() && self.chars[self.at] != '<' {
                self.at += 1;
            }
            self.at += 1;
            let start = self.at;
            while self.at < self.chars.len() && self.chars[self.at] != '>' {
                self.at += 1;
            }
            if self.at >= self.chars.len() {
                return None;
            }
            let tag: String = self.chars[start..self.at].iter().collect();
            self.at += 1;
            Some(tag.trim().to_owned())
        }

        /// Everything up to the next `<`, entities resolved, with the
        /// closing tag consumed.
        fn text_until_close(&mut self) -> String {
            let start = self.at;
            while self.at < self.chars.len() && self.chars[self.at] != '<' {
                self.at += 1;
            }
            let text: String = self.chars[start..self.at].iter().collect();
            let _ = self.next_tag();
            unescape(&text)
        }
    }

    fn unescape(text: &str) -> String {
        text.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&")
    }
}

/// Every capture a scope maps to must be one the theme table knows;
/// a typo here would drop a colour silently.
#[cfg(test)]
fn scope_map_targets_are_real_captures() -> Result<(), String> {
    for (prefix, captures) in SCOPE_MAP {
        for capture in *captures {
            if !CAPTURES.contains(capture) {
                return Err(format!("{prefix} maps to unknown capture {capture}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mapped_capture_exists() {
        scope_map_targets_are_real_captures().unwrap();
    }

    #[test]
    fn the_scope_map_is_alphabetical() {
        let mut sorted: Vec<&str> = SCOPE_MAP.iter().map(|(prefix, _)| *prefix).collect();
        let written = sorted.clone();
        sorted.sort_unstable();
        assert_eq!(written, sorted, "SCOPE_MAP is read by humans in order");
    }

    #[test]
    fn every_fallback_names_real_captures_and_terminates() {
        for (capture, parent) in FALLBACKS {
            assert!(CAPTURES.contains(capture), "{capture} is not a capture");
            assert!(CAPTURES.contains(parent), "{parent} is not a capture");
        }
        let mut sorted: Vec<&str> = FALLBACKS.iter().map(|(c, _)| *c).collect();
        let written = sorted.clone();
        sorted.sort_unstable();
        assert_eq!(written, sorted, "FALLBACKS is read by humans in order");
        // No capture may reach itself by following parents.
        for (capture, _) in FALLBACKS {
            let mut at = *capture;
            for _ in 0..FALLBACKS.len() {
                let Some((_, parent)) = FALLBACKS.iter().find(|(name, _)| name == &at) else {
                    break;
                };
                assert_ne!(*parent, *capture, "{capture} is its own ancestor");
                at = parent;
            }
        }
    }

    #[test]
    fn distinctions_the_source_never_drew_are_inherited() {
        // A theme that colours `keyword` and nothing under it: every
        // kind of keyword should still come out that colour.
        let json = r##"{"name": "T", "type": "dark", "tokenColors": [
            {"scope": "keyword", "settings": {"foreground": "#AA00AA"}},
            {"scope": "keyword.control.loop", "settings": {"foreground": "#00AA00"}}
        ]}"##;
        let imported = from_vscode(json).unwrap();
        let styles: HashMap<&str, &ImportedStyle> = imported
            .styles
            .iter()
            .map(|(c, s)| (c.as_str(), s))
            .collect();
        assert_eq!(styles["conditional"].color.as_deref(), Some("#AA00AA"));
        assert_eq!(styles["exception"].color.as_deref(), Some("#AA00AA"));
        // A distinction the source did draw survives inheritance.
        assert_eq!(styles["repeat"].color.as_deref(), Some("#00AA00"));
    }

    #[test]
    fn inheritance_walks_more_than_one_step() {
        // float -> number -> constant, none of them written directly.
        let json = r##"{"name": "T", "type": "dark", "tokenColors": [
            {"scope": "constant", "settings": {"foreground": "#334455"}}
        ]}"##;
        let imported = from_vscode(json).unwrap();
        let styles: HashMap<&str, &ImportedStyle> = imported
            .styles
            .iter()
            .map(|(c, s)| (c.as_str(), s))
            .collect();
        assert_eq!(styles["float"].color.as_deref(), Some("#334455"));
        assert_eq!(styles["boolean"].color.as_deref(), Some("#334455"));
    }

    #[test]
    fn a_theme_of_the_usual_scopes_fills_the_whole_palette() {
        // The scopes a mainstream theme actually colours. Every capture
        // must come out with a colour, directly or by inheritance — a
        // new capture with no scope and no fallback would otherwise
        // show in Textchum's colours inside someone else's theme.
        let scopes = [
            "comment", "string", "string.regexp", "constant.numeric",
            "constant.language", "constant.character.escape", "constant.other",
            "variable", "variable.parameter", "variable.language",
            "variable.other.constant", "variable.other.property",
            "keyword", "keyword.control", "keyword.operator",
            "storage", "storage.type", "storage.modifier",
            "entity.name.function", "entity.name.type", "entity.name.class",
            "entity.name.tag", "entity.name.namespace", "entity.name.section",
            "entity.other.attribute-name", "entity.other.inherited-class",
            "support.function", "support.class", "support.type",
            "support.constant", "support.variable", "support.type.property-name",
            "punctuation.definition.tag", "punctuation.separator",
            "invalid", "invalid.deprecated",
            "markup.bold", "markup.italic", "markup.heading",
            "markup.underline.link", "markup.inline.raw", "markup.quote",
            "meta.object-literal.key", "meta.preprocessor",
        ];
        let rules: Vec<String> = scopes
            .iter()
            .map(|scope| {
                format!(
                    r##"{{"scope": "{scope}", "settings": {{"foreground": "#123456"}}}}"##
                )
            })
            .collect();
        let json = format!(
            r##"{{"name": "Coverage", "type": "dark", "tokenColors": [{}]}}"##,
            rules.join(",")
        );
        let imported = from_vscode(&json).unwrap();
        let filled: Vec<&str> = imported.styles.iter().map(|(c, _)| c.as_str()).collect();
        let missing: Vec<&&str> =
            CAPTURES.iter().filter(|c| !filled.contains(c)).collect();
        assert!(missing.is_empty(), "no colour for {missing:?}");
    }

    #[test]
    fn a_family_named_only_by_a_special_case_still_gets_a_colour() {
        // Rust tags an integer literal `constant.builtin`, and a theme
        // that says `constant.numeric` and nothing broader would leave
        // it in Textchum's colour beside the source's own.
        let json = r##"{"name": "T", "type": "dark", "tokenColors": [
            {"scope": "constant.numeric", "settings": {"foreground": "#D19A66"}}
        ]}"##;
        let imported = from_vscode(json).unwrap();
        let styles: HashMap<&str, &ImportedStyle> = imported
            .styles
            .iter()
            .map(|(c, s)| (c.as_str(), s))
            .collect();
        assert_eq!(styles["number"].color.as_deref(), Some("#D19A66"));
        assert_eq!(styles["constant"].color.as_deref(), Some("#D19A66"));
        assert_eq!(styles["constant.builtin"].color.as_deref(), Some("#D19A66"));
        assert_eq!(styles["boolean"].color.as_deref(), Some("#D19A66"));
    }

    #[test]
    fn inheritance_never_overwrites_what_the_source_wrote() {
        let json = r##"{"name": "T", "type": "dark", "tokenColors": [
            {"scope": "constant.numeric", "settings": {"foreground": "#111111"}},
            {"scope": "constant.language", "settings": {"foreground": "#222222"}}
        ]}"##;
        let imported = from_vscode(json).unwrap();
        let styles: HashMap<&str, &ImportedStyle> = imported
            .styles
            .iter()
            .map(|(c, s)| (c.as_str(), s))
            .collect();
        assert_eq!(styles["number"].color.as_deref(), Some("#111111"));
        assert_eq!(styles["constant.builtin"].color.as_deref(), Some("#222222"));
    }

    #[test]
    fn the_longest_matching_prefix_wins() {
        assert_eq!(longest_match("keyword.control.loop.for").unwrap().1, &["repeat"]);
        assert_eq!(longest_match("keyword.control.other").unwrap().1, &["keyword"]);
        assert_eq!(longest_match("keyword").unwrap().1, &["keyword"]);
        // A prefix must end on a dot, or `stringify` would count as a
        // string.
        assert!(longest_match("stringify").is_none());
    }

    #[test]
    fn a_vscode_theme_becomes_captures() {
        let json = r##"{
            // A theme, with the comments VS Code allows.
            "name": "Nightly",
            "type": "dark",
            "tokenColors": [
                {"scope": "comment", "settings": {"foreground": "#6a6a6a",
                                                  "fontStyle": "italic"}},
                {"scope": ["keyword.control.loop", "keyword.control.conditional"],
                 "settings": {"foreground": "#ff0080"}},
                {"scope": "entity.name.function", "settings": {"foreground": "#0af"}},
                {"scope": "string", "settings": {"foreground": "#00FF00FF"}},
            ]
        }"##;
        let imported = from_vscode(json).unwrap();
        assert_eq!(imported.name, "Nightly");
        assert_eq!(imported.appearance, Appearance::Dark);
        let styles: HashMap<&str, &ImportedStyle> = imported
            .styles
            .iter()
            .map(|(capture, style)| (capture.as_str(), style))
            .collect();
        assert_eq!(styles["comment"].color.as_deref(), Some("#6A6A6A"));
        assert!(styles["comment"].italic);
        assert_eq!(styles["repeat"].color.as_deref(), Some("#FF0080"));
        assert_eq!(styles["conditional"].color.as_deref(), Some("#FF0080"));
        assert_eq!(styles["function"].color.as_deref(), Some("#00AAFF"));
        // The alpha channel is dropped, not carried into the palette.
        assert_eq!(styles["string"].color.as_deref(), Some("#00FF00"));
    }

    #[test]
    fn a_more_specific_scope_beats_a_broader_one_whatever_the_order() {
        let json = r##"{"name": "T", "type": "dark", "tokenColors": [
            {"scope": "keyword.control.loop", "settings": {"foreground": "#111111"}},
            {"scope": "keyword", "settings": {"foreground": "#222222"}}
        ]}"##;
        let imported = from_vscode(json).unwrap();
        let styles: HashMap<&str, &ImportedStyle> = imported
            .styles
            .iter()
            .map(|(c, s)| (c.as_str(), s))
            .collect();
        assert_eq!(styles["repeat"].color.as_deref(), Some("#111111"));
        assert_eq!(styles["keyword"].color.as_deref(), Some("#222222"));
    }

    #[test]
    fn a_light_theme_fills_the_light_side() {
        let json = r##"{"name": "Day", "colors": {"editor.background": "#FFFFFF"},
                       "tokenColors": [{"scope": "comment",
                                        "settings": {"foreground": "#808080"}}]}"##;
        let imported = from_vscode(json).unwrap();
        assert_eq!(imported.appearance, Appearance::Light);
        let written: serde_json::Value =
            serde_json::from_str(&imported.to_theme_json()).unwrap();
        assert_eq!(written["styles"]["comment"]["light"], "#808080");
        assert!(written["styles"]["comment"].get("dark").is_none());
    }

    #[test]
    fn scopes_with_nowhere_to_go_are_reported() {
        let json = r##"{"name": "T", "type": "dark", "tokenColors": [
            {"scope": "meta.brace.round", "settings": {"foreground": "#111111"}}
        ]}"##;
        assert_eq!(from_vscode(json).unwrap().unmapped, vec!["meta.brace.round"]);
    }

    #[test]
    fn a_theme_without_token_colors_is_refused_with_a_reason() {
        let error = from_vscode(r#"{"name": "T"}"#).unwrap_err();
        assert!(error.contains("tokenColors"), "{error}");
    }

    #[test]
    fn a_textmate_theme_becomes_captures() {
        let xml = r##"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>name</key>
  <string>Mono &amp; Blue</string>
  <key>settings</key>
  <array>
    <dict>
      <key>settings</key>
      <dict>
        <key>background</key>
        <string>#1B1B1B</string>
        <key>foreground</key>
        <string>#DDDDDD</string>
      </dict>
    </dict>
    <dict>
      <key>name</key>
      <string>Comment</string>
      <key>scope</key>
      <string>comment, punctuation.definition.comment</string>
      <key>settings</key>
      <dict>
        <key>fontStyle</key>
        <string>italic</string>
        <key>foreground</key>
        <string>#777</string>
      </dict>
    </dict>
    <dict>
      <key>scope</key>
      <string>meta.function entity.name.function</string>
      <key>settings</key>
      <dict>
        <key>foreground</key>
        <string>#4488CC</string>
        <key>fontStyle</key>
        <string>bold</string>
      </dict>
    </dict>
  </array>
</dict>
</plist>"##;
        let imported = from_textmate(xml).unwrap();
        assert_eq!(imported.name, "Mono & Blue");
        assert_eq!(imported.appearance, Appearance::Dark);
        let styles: HashMap<&str, &ImportedStyle> = imported
            .styles
            .iter()
            .map(|(c, s)| (c.as_str(), s))
            .collect();
        // #777 expands, and the italic comes through.
        assert_eq!(styles["comment"].color.as_deref(), Some("#777777"));
        assert!(styles["comment"].italic);
        // A descendant selector is named by its last part.
        assert_eq!(styles["function"].color.as_deref(), Some("#4488CC"));
        assert!(styles["function"].bold);
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "textchum-import-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_extension_folder_brings_in_every_theme_it_contributes() {
        let dir = temp_dir("extension");
        let themes = dir.join("themes");
        std::fs::create_dir_all(&themes).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r##"{"contributes": {"themes": [
                {"label": "Night", "path": "./themes/night.json"},
                {"label": "Day", "path": "themes/day.json"}
            ]}}"##,
        )
        .unwrap();
        for (file, name, kind) in [
            ("night.json", "Night", "dark"),
            ("day.json", "Day", "light"),
        ] {
            std::fs::write(
                themes.join(file),
                format!(
                    r##"{{"name": "{name}", "type": "{kind}", "tokenColors": [
                        {{"scope": "comment", "settings": {{"foreground": "#777777"}}}}]}}"##
                ),
            )
            .unwrap();
        }

        let destination = temp_dir("extension-out");
        let outcome = import_into(&dir, Source::VsCode, &destination);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        // The manifest's order, which is the extension author's.
        assert_eq!(outcome.written, vec!["Night".to_owned(), "Day".to_owned()]);
        assert_eq!(outcome.appearances, vec!["dark".to_owned(), "light".to_owned()]);
        // Each theme is a file named after itself, which is how the
        // editor lists and chooses one.
        assert!(destination.join("Night.json").is_file());
        assert!(destination.join("Day.json").is_file());
    }

    #[test]
    fn a_theme_named_like_a_path_does_not_become_one() {
        let dir = temp_dir("naming");
        std::fs::write(
            dir.join("theme.json"),
            r##"{"name": "../../escape", "type": "dark", "tokenColors": [
                {"scope": "comment", "settings": {"foreground": "#777777"}}]}"##,
        )
        .unwrap();
        let destination = temp_dir("naming-out");
        let outcome = import_into(&dir.join("theme.json"), Source::VsCode, &destination);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert_eq!(
            std::fs::read_dir(&destination).unwrap().count(),
            1,
            "the theme went somewhere other than the themes directory"
        );
    }

    #[test]
    fn a_folder_with_no_theme_in_it_says_so() {
        let dir = temp_dir("empty");
        let outcome = import_into(&dir, Source::TextMate, &temp_dir("empty-out"));
        assert!(outcome.written.is_empty());
        assert_eq!(outcome.errors.len(), 1);
        assert!(outcome.errors[0].contains("no TextMate theme"), "{:?}", outcome.errors);
    }

    #[test]
    fn a_bundle_keeps_its_themes_in_a_subdirectory() {
        let bundle = temp_dir("bundle");
        let themes = bundle.join("Themes");
        std::fs::create_dir_all(&themes).unwrap();
        std::fs::write(
            themes.join("Solar.tmTheme"),
            r#"<?xml version="1.0"?>
<plist version="1.0"><dict>
  <key>name</key><string>Solar</string>
  <key>settings</key><array>
    <dict><key>scope</key><string>comment</string>
      <key>settings</key><dict><key>foreground</key><string>#888888</string></dict></dict>
  </array>
</dict></plist>"#,
        )
        .unwrap();
        let outcome = import_into(&bundle, Source::TextMate, &temp_dir("bundle-out"));
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert_eq!(outcome.written, vec!["Solar".to_owned()]);
    }

    #[test]
    fn the_plist_reader_handles_the_shapes_a_theme_uses() {
        let xml = r#"<?xml version="1.0"?>
<plist version="1.0">
<dict>
  <key>uuid</key><string>A &amp; B</string>
  <key>semanticClass</key><true/>
  <key>hidden</key><false/>
  <key>list</key><array><string>one</string><string>two</string></array>
</dict>
</plist>"#;
        let value = plist::parse(xml).unwrap();
        let dict = value.as_dict().unwrap();
        assert_eq!(dict["uuid"].as_str(), Some("A & B"));
        assert_eq!(dict["semanticClass"].as_bool(), Some(true));
        assert_eq!(dict["hidden"].as_bool(), Some(false));
        assert_eq!(dict["list"].as_array().unwrap().len(), 2);
        assert_eq!(dict["list"].as_array().unwrap()[1].as_str(), Some("two"));
    }

    #[test]
    fn a_file_that_is_not_a_theme_is_refused_with_a_reason() {
        let error = from_textmate("<?xml version=\"1.0\"?><nope/>").unwrap_err();
        assert!(error.contains("plist"), "{error}");
    }

    #[test]
    fn an_imported_theme_is_one_the_editor_can_read_back() {
        let json = r##"{"name": "Round Trip", "type": "dark", "tokenColors": [
            {"scope": "comment", "settings": {"foreground": "#445566",
                                              "fontStyle": "italic bold"}}
        ]}"##;
        let imported = from_vscode(json).unwrap();
        let written = imported.to_theme_json();
        let theme = crate::syntax::theme::Theme::from_json(&written).unwrap();
        assert_eq!(theme.name, "Round Trip");
        let value: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(value["styles"]["comment"]["dark"], "#445566");
        assert_eq!(value["styles"]["comment"]["bold"], true);
        assert_eq!(value["styles"]["comment"]["italic"], true);
    }

    #[test]
    fn jsonc_comments_and_trailing_commas_come_out() {
        let text = "{\n  // a line comment\n  \"a\": \"//not a comment\", /* block */\n  \"b\": [1, 2,],\n}";
        let value: serde_json::Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(value["a"], "//not a comment");
        assert_eq!(value["b"], serde_json::json!([1, 2]));
    }
}
