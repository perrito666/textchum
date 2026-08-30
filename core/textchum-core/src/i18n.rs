//! The interface in another language.
//!
//! One catalogue per language, keyed by the English text itself, so a
//! string that has no translation reads as what it says rather than as
//! a missing key. The catalogues live here rather than in either shell:
//! both shells say the same things, and so does the core, and a phrase
//! translated once should be translated once.
//!
//! ```
//! use textchum_core::i18n;
//! i18n::set_language("es");
//! assert_eq!(i18n::tr("Close Tab"), "Cerrar pestaña");
//! i18n::set_language("en");
//! assert_eq!(i18n::tr("Close Tab"), "Close Tab");
//! ```
//!
//! A user catalogue in the profile is read over the built-in one, so a
//! phrase can be corrected, or a language added, without a build.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// The languages the build carries, in the order they are offered.
pub static LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("es", "Español"),
    ("fr", "Français"),
];

static SPANISH: &str = include_str!("../i18n/es.json");
static FRENCH: &str = include_str!("../i18n/fr.json");

struct State {
    language: String,
    catalogue: HashMap<String, String>,
    /// Where user catalogues are read from, when the shell has said.
    dir: Option<PathBuf>,
}

static STATE: RwLock<Option<State>> = RwLock::new(None);

/// Chooses the interface language: `en`, `es`, `fr`, or a tag like
/// `es-AR`, of which only the first part is used. Anything else reads
/// as English.
pub fn set_language(tag: &str) {
    let language = normalize(tag);
    let mut catalogue = built_in(&language);
    if let Some(dir) = STATE.read().ok().and_then(|state| {
        state.as_ref().and_then(|state| state.dir.clone())
    }) {
        merge_user_catalogue(&mut catalogue, &dir, &language);
    }
    if let Ok(mut state) = STATE.write() {
        let dir = state.as_ref().and_then(|state| state.dir.clone());
        *state = Some(State {
            language,
            catalogue,
            dir,
        });
    }
}

/// Where to look for user catalogues: `<dir>/<language>.json`, read
/// over the built-in one.
pub fn set_catalogue_dir(dir: &Path) {
    let language = language();
    let mut catalogue = built_in(&language);
    merge_user_catalogue(&mut catalogue, dir, &language);
    if let Ok(mut state) = STATE.write() {
        *state = Some(State {
            language,
            catalogue,
            dir: Some(dir.to_path_buf()),
        });
    }
}

/// The language in use, as a two-letter tag.
pub fn language() -> String {
    STATE
        .read()
        .ok()
        .and_then(|state| state.as_ref().map(|state| state.language.clone()))
        .unwrap_or_else(|| "en".to_string())
}

/// `text` in the interface language, or `text` itself when the
/// catalogue has nothing to say about it.
pub fn tr(text: &str) -> String {
    STATE
        .read()
        .ok()
        .and_then(|state| {
            state
                .as_ref()
                .and_then(|state| state.catalogue.get(text).cloned())
        })
        .unwrap_or_else(|| text.to_string())
}

/// The whole catalogue in use, as JSON. The shells read it once and
/// look up in their own language afterwards, rather than crossing the
/// bridge for every label.
pub fn catalogue_json() -> String {
    let map = STATE
        .read()
        .ok()
        .and_then(|state| state.as_ref().map(|state| state.catalogue.clone()))
        .unwrap_or_default();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
}

/// The tag a system locale asks for: `es_AR.UTF-8` is Spanish.
pub fn language_from_locale(locale: &str) -> String {
    normalize(locale)
}

fn normalize(tag: &str) -> String {
    let head = tag
        .split(['-', '_', '.'])
        .next()
        .unwrap_or("en")
        .to_ascii_lowercase();
    if LANGUAGES.iter().any(|(code, _)| *code == head) {
        head
    } else {
        "en".to_string()
    }
}

fn built_in(language: &str) -> HashMap<String, String> {
    let source = match language {
        "es" => SPANISH,
        "fr" => FRENCH,
        // English is the text in the source, so its catalogue is empty.
        _ => return HashMap::new(),
    };
    serde_json::from_str(source).unwrap_or_default()
}

/// A catalogue in the profile wins over the built-in one, phrase by
/// phrase: correcting one line should not mean copying the rest.
fn merge_user_catalogue(catalogue: &mut HashMap<String, String>, dir: &Path, language: &str) {
    let path = dir.join(format!("{language}.json"));
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let Ok(extra) = serde_json::from_str::<HashMap<String, String>>(&text) else {
        return;
    };
    for (key, value) in extra {
        catalogue.insert(key, value);
    }
}

/// `tr` with arguments: `t!("Save changes to {}?", name)`.
#[macro_export]
macro_rules! t {
    ($text:expr) => {
        $crate::i18n::tr($text)
    };
    ($text:expr, $($arg:expr),+ $(,)?) => {{
        let mut out = $crate::i18n::tr($text);
        $(
            if let Some(at) = out.find("{}") {
                out.replace_range(at..at + 2, &format!("{}", $arg));
            }
        )+
        out
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phrase_with_no_translation_reads_as_itself() {
        set_language("es");
        assert_eq!(tr("Not a phrase anybody has translated"), "Not a phrase anybody has translated");
        set_language("en");
    }

    #[test]
    fn the_catalogues_answer_in_their_own_language() {
        set_language("es");
        assert_eq!(tr("Close Tab"), "Cerrar pestaña");
        set_language("fr");
        assert_eq!(tr("Close Tab"), "Fermer l'onglet");
        set_language("en");
        assert_eq!(tr("Close Tab"), "Close Tab");
    }

    #[test]
    fn a_locale_names_a_language_and_anything_else_is_english() {
        assert_eq!(language_from_locale("es_AR.UTF-8"), "es");
        assert_eq!(language_from_locale("fr-CA"), "fr");
        assert_eq!(language_from_locale("de_DE.UTF-8"), "en");
        assert_eq!(language_from_locale(""), "en");
    }

    #[test]
    fn arguments_go_where_the_braces_are() {
        set_language("en");
        assert_eq!(t!("Save changes to {}?", "main.rs"), "Save changes to main.rs?");
    }

    #[test]
    fn every_catalogue_carries_the_same_phrases() {
        // A phrase translated into one language and forgotten in the
        // other is the failure this catches: both catalogues are
        // written from the same list.
        let spanish: HashMap<String, String> = serde_json::from_str(SPANISH).unwrap();
        let french: HashMap<String, String> = serde_json::from_str(FRENCH).unwrap();
        let mut missing: Vec<&String> = spanish.keys().filter(|key| !french.contains_key(*key)).collect();
        missing.extend(french.keys().filter(|key| !spanish.contains_key(*key)));
        assert!(missing.is_empty(), "not in both catalogues: {missing:?}");
    }
}
