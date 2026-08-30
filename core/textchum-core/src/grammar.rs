//! Grammars loaded from a library at runtime.
//!
//! The built-in set is whatever the release was compiled with, so a
//! language missing from it had no way in short of a new build. A
//! grammar named in the configuration is opened here instead:
//!
//! ```json
//! {"languages": {"zig": {
//!   "grammar": "~/.local/share/textchum/grammars/libtree-sitter-zig.dylib",
//!   "highlights": "~/.local/share/textchum/grammars/zig/highlights.scm",
//!   "extensions": ["zig", "zon"],
//!   "aliases": ["ziglang"],
//!   "injections": "…/injections.scm",
//!   "symbol": "tree_sitter_zig"
//! }}}
//! ```
//!
//! `symbol` is optional: `tree_sitter_<name>` is the convention every
//! grammar follows, with dashes turned into underscores.
//!
//! The library is never unloaded. A syntax tree points into the
//! grammar's static tables, so closing the library under a live tree
//! ends the process — and a grammar is wanted for as long as a document
//! that uses it is open, which is until the editor quits.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tree_sitter::Language;

use crate::syntax::languages::{LanguageSource, LanguageSpec};

/// Loads every grammar named in the configuration's `languages`
/// section, and answers with what went wrong — one line per entry that
/// could not be loaded, for the shell to show.
///
/// An entry that fails is skipped rather than fatal: a stale path in a
/// configuration file should cost that one language, not the editor.
pub fn load_configured(config_json: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<Value>(config_json) else {
        return vec!["languages: the configuration is not JSON".to_string()];
    };
    let Some(entries) = root.get("languages").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut problems = Vec::new();
    for (name, entry) in entries {
        if let Err(problem) = load_entry(name, entry) {
            problems.push(format!("{name}: {problem}"));
        }
    }
    problems
}

fn load_entry(name: &str, entry: &Value) -> Result<(), String> {
    let library = entry
        .get("grammar")
        .and_then(Value::as_str)
        .ok_or("no grammar library named (\"grammar\")")?;
    let highlights_path = entry
        .get("highlights")
        .and_then(Value::as_str)
        .ok_or("no highlights query named (\"highlights\")")?;
    let symbol = entry
        .get("symbol")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("tree_sitter_{}", name.replace(['-', '.'], "_")));
    let extensions: Vec<String> = strings(entry.get("extensions"));
    let aliases: Vec<String> = strings(entry.get("aliases"));
    let filenames: Vec<String> = strings(entry.get("filenames"));

    let language = open(&expand(library), &symbol)?;
    let highlights = read(&expand(highlights_path))?;
    let injections = match entry.get("injections").and_then(Value::as_str) {
        Some(path) => Some(read(&expand(path))?),
        None => None,
    };
    // A query that does not compile against this grammar is the mistake
    // worth catching here: the alternative is a language that loads and
    // then paints nothing, with no way to tell why.
    tree_sitter::Query::new(&language, &highlights)
        .map_err(|error| format!("the highlights query does not compile: {error}"))?;
    if let Some(source) = &injections {
        tree_sitter::Query::new(&language, source)
            .map_err(|error| format!("the injections query does not compile: {error}"))?;
    }

    crate::syntax::languages::register_loaded(LanguageSpec {
        name: leak(name.to_string()),
        aliases: leak_all(aliases),
        extensions: leak_all(extensions),
        filenames: leak_all(filenames),
        language: LanguageSource::Loaded(language),
        highlights: leak(highlights),
        highlights_extra: None,
        injections: injections.map(leak).map(|source| source as &'static str),
    });
    Ok(())
}

/// Opens the library and asks it for its grammar.
fn open(path: &Path, symbol: &str) -> Result<Language, String> {
    if !path.exists() {
        return Err(format!("{} is not there", path.display()));
    }
    // Safety: loading a library runs its initializers, and the symbol is
    // called as a grammar constructor. Both are what the configuration
    // asked for; a wrong path is a wrong path either way.
    unsafe {
        let library = libloading::Library::new(path)
            .map_err(|error| format!("{} could not be opened: {error}", path.display()))?;
        let constructor: libloading::Symbol<unsafe extern "C" fn() -> Language> = library
            .get(symbol.as_bytes())
            .map_err(|error| format!("{symbol} is not in {}: {error}", path.display()))?;
        let language = constructor();
        // Deliberate: see the module note. The trees outlive any point
        // at which the library could be closed.
        std::mem::forget(library);
        let version = language.abi_version();
        if !(tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
            .contains(&version)
        {
            return Err(format!(
                "the grammar speaks ABI {version}; this build speaks {}–{}",
                tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
                tree_sitter::LANGUAGE_VERSION
            ));
        }
        Ok(language)
    }
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))
}

/// `~` is what a configuration file shared between machines can say.
fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn leak_all(items: Vec<String>) -> &'static [&'static str] {
    let leaked: Vec<&'static str> = items.into_iter().map(leak).collect();
    Box::leak(leaked.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configuration_without_languages_asks_for_nothing() {
        assert!(load_configured(r#"{"editor":{}}"#).is_empty());
    }

    #[test]
    fn an_entry_says_what_it_is_missing() {
        let problems = load_configured(r#"{"languages":{"zig":{}}}"#);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("grammar"), "{problems:?}");
    }

    #[test]
    fn a_library_that_is_not_there_is_one_language_lost_and_not_a_crash() {
        let problems = load_configured(
            r#"{"languages":{"zig":{"grammar":"/nowhere/libzig.dylib",
                 "highlights":"/nowhere/highlights.scm"}}}"#,
        );
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("is not there"), "{problems:?}");
    }

    #[test]
    fn the_symbol_follows_the_name_when_it_is_not_given() {
        // The convention every grammar follows, with the punctuation a
        // language name can carry turned into underscores.
        assert_eq!(
            "tree_sitter_my_lang",
            format!("tree_sitter_{}", "my-lang".replace(['-', '.'], "_"))
        );
    }

    #[test]
    fn a_tilde_means_home() {
        std::env::set_var("HOME", "/home/someone");
        assert_eq!(expand("~/g/x.so"), PathBuf::from("/home/someone/g/x.so"));
        assert_eq!(expand("/abs/x.so"), PathBuf::from("/abs/x.so"));
    }
}
