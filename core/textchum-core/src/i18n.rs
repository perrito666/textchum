//! The interface in another language.
//!
//! Catalogues are gettext: `i18n/<language>.po` is the source of truth,
//! which is what translators and their tools speak, and the build
//! compiles each into the binary `.mo` the editor reads. Both shells
//! and the core share them — the three say the same things, and a
//! phrase translated once should be translated once.
//!
//! ```
//! use textchum_core::i18n;
//! i18n::set_language("es");
//! assert_eq!(i18n::tr("Close Tab"), "Cerrar pestaña");
//! i18n::set_language("en");
//! assert_eq!(i18n::tr("Close Tab"), "Close Tab");
//! ```
//!
//! Strings are keyed by the English text itself, so one with no
//! translation reads as what it says rather than as a missing key.
//! A `.mo` in the profile is read instead of the built-in one, so a
//! catalogue can be corrected, or a language added, without a build:
//!
//! ```text
//! msgfmt -o ~/.config/textchum/translations/es.mo es.po
//! ```
//!
//! Extracting the strings to translate is the standard incantation,
//! with the two names this codebase calls the lookup by:
//!
//! ```text
//! xgettext --keyword=tr --keyword=t --keyword=t! \
//!     --keyword=tr_n:1,2 --keyword=t_n:1,2 \
//!     --from-code=UTF-8 -o core/textchum-core/i18n/textchum.pot \
//!     $(git ls-files '*.rs' '*.swift')
//! msgmerge --update core/textchum-core/i18n/es.po \
//!     core/textchum-core/i18n/textchum.pot
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use gettext::Catalog;

/// The languages the build carries, in the order they are offered.
pub static LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("es", "Español"),
    ("fr", "Français"),
];

/// The compiled catalogues, built from `i18n/*.po`.
static SPANISH: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/es.mo"));
static FRENCH: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fr.mo"));

struct State {
    language: String,
    catalogue: Catalog,
    /// Where user catalogues are read from, when the shell has said.
    dir: Option<PathBuf>,
}

static STATE: RwLock<Option<State>> = RwLock::new(None);

/// Chooses the interface language: `en`, `es`, `fr`, or a tag like
/// `es-AR`, of which only the first part is used. Anything else reads
/// as English.
pub fn set_language(tag: &str) {
    let language = normalize(tag);
    let dir = STATE
        .read()
        .ok()
        .and_then(|state| state.as_ref().and_then(|state| state.dir.clone()));
    let catalogue = catalogue_for(&language, dir.as_deref());
    if let Ok(mut state) = STATE.write() {
        *state = Some(State {
            language,
            catalogue,
            dir,
        });
    }
}

/// Where to look for catalogues of one's own: `<dir>/<language>.mo`,
/// read instead of the one the build carries.
pub fn set_catalogue_dir(dir: &Path) {
    let language = language();
    let catalogue = catalogue_for(&language, Some(dir));
    if let Ok(mut state) = STATE.write() {
        *state = Some(State {
            language,
            catalogue,
            dir: Some(dir.to_path_buf()),
        });
    }
}

/// The catalogue for a language: the one in `dir` when there is one,
/// the built-in otherwise, and an empty one for English, whose text is
/// the source itself.
fn catalogue_for(language: &str, dir: Option<&Path>) -> Catalog {
    if let Some(dir) = dir {
        let path = dir.join(format!("{language}.mo"));
        if let Ok(file) = std::fs::File::open(&path) {
            match Catalog::parse(file) {
                Ok(catalogue) => return catalogue,
                Err(error) => eprintln!("textchum: {}: {error}", path.display()),
            }
        }
    }
    let bytes: &[u8] = match language {
        "es" => SPANISH,
        "fr" => FRENCH,
        _ => return Catalog::parse(&b""[..]).unwrap_or_else(|_| empty_catalogue()),
    };
    Catalog::parse(bytes).unwrap_or_else(|_| empty_catalogue())
}

/// A catalogue with nothing in it, which answers every lookup with the
/// text it was asked about.
fn empty_catalogue() -> Catalog {
    // The header-only catalogue every `.mo` starts as.
    const EMPTY: [u8; 28] = [
        0xde, 0x12, 0x04, 0x95, 0, 0, 0, 0, 0, 0, 0, 0, 28, 0, 0, 0, 28, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0,
    ];
    Catalog::parse(&EMPTY[..]).expect("an empty catalogue parses")
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
                .map(|state| state.catalogue.gettext(text).to_string())
        })
        .unwrap_or_else(|| text.to_string())
}

/// One or many, in the interface language and by its own rules: two
/// forms in English, and however many the catalogue's `Plural-Forms`
/// says elsewhere.
pub fn tr_n(one: &str, many: &str, count: usize) -> String {
    STATE
        .read()
        .ok()
        .and_then(|state| {
            state.as_ref().map(|state| {
                state.catalogue.ngettext(one, many, count as u64).to_string()
            })
        })
        .unwrap_or_else(|| if count == 1 { one.to_string() } else { many.to_string() })
}

/// The catalogue in use, as JSON. The shells read it once and look up
/// in it afterwards, rather than crossing the bridge for every label.
pub fn catalogue_json() -> String {
    let language = language();
    let dir = STATE
        .read()
        .ok()
        .and_then(|state| state.as_ref().and_then(|state| state.dir.clone()));
    let map = entries(&language, dir.as_deref());
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
}

/// Every singular message of a catalogue, by its English text. The
/// `gettext` crate looks messages up but does not hand them all over,
/// and the shells want the lot in one crossing of the bridge.
fn entries(language: &str, dir: Option<&Path>) -> HashMap<String, String> {
    let owned;
    let bytes: &[u8] = match dir.map(|dir| dir.join(format!("{language}.mo"))) {
        Some(path) if path.exists() => match std::fs::read(&path) {
            Ok(read) => {
                owned = read;
                &owned
            }
            Err(_) => return HashMap::new(),
        },
        _ => match language {
            "es" => SPANISH,
            "fr" => FRENCH,
            _ => return HashMap::new(),
        },
    };
    read_mo(bytes)
}

/// The `.mo` format: a magic number saying the byte order, a count, and
/// two tables of (length, offset) pairs. Plural forms are separated by
/// NUL and a context by EOT; only the singular of each is wanted here,
/// and an entry with a context is left to the lookup.
fn read_mo(bytes: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if bytes.len() < 28 {
        return map;
    }
    let swapped = match bytes[0..4] {
        [0xde, 0x12, 0x04, 0x95] => false,
        [0x95, 0x04, 0x12, 0xde] => true,
        _ => return map,
    };
    let word = |at: usize| -> u32 {
        let raw = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        if swapped {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        }
    };
    let count = word(8) as usize;
    let ids_at = word(12) as usize;
    let texts_at = word(16) as usize;
    for index in 0..count {
        let id_entry = ids_at + index * 8;
        let text_entry = texts_at + index * 8;
        if text_entry + 8 > bytes.len() {
            break;
        }
        let take = |at: usize| -> Option<&str> {
            let length = word(at) as usize;
            let offset = word(at + 4) as usize;
            std::str::from_utf8(bytes.get(offset..offset + length)?).ok()
        };
        let (Some(id), Some(text)) = (take(id_entry), take(text_entry)) else {
            continue;
        };
        // The header is the entry with an empty id.
        if id.is_empty() {
            continue;
        }
        let id = id.split('\u{4}').next_back().unwrap_or(id);
        let id = id.split('\0').next().unwrap_or(id);
        let text = text.split('\0').next().unwrap_or(text);
        if text.is_empty() {
            continue;
        }
        map.insert(id.to_string(), text.to_string());
    }
    map
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

/// Marks a string for translation without translating it yet — the
/// tables that name every command, whose titles are looked up later
/// through a variable that no extractor can follow. C spells this
/// `N_()`; the extractor is told about both.
pub const fn n_(text: &'static str) -> &'static str {
    text
}

/// Puts `arguments` where the `{}` are, after the lookup. Formatting a
/// translated string is the C idiom for the same reason it is here: the
/// extractor has to see a plain string in the call.
pub fn fill(text: &str, arguments: &[&str]) -> String {
    let mut out = text.to_string();
    for argument in arguments {
        let Some(at) = out.find("{}") else { break };
        out.replace_range(at..at + 2, argument);
    }
    out
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

/// `t_n!("{} file", "{} files", n)`: the count goes where the braces
/// are, and the catalogue decides which form its language wants.
#[macro_export]
macro_rules! t_n {
    ($one:expr, $many:expr, $count:expr) => {{
        let count = $count;
        let mut out = $crate::i18n::tr_n($one, $many, count);
        if let Some(at) = out.find("{}") {
            out.replace_range(at..at + 2, &format!("{}", count));
        }
        out
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The language is one setting for the whole process, so the tests
    /// that change it take turns. Without this they race, and the one
    /// that loses reads the language another test just set.
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_phrase_with_no_translation_reads_as_itself() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        set_language("es");
        assert_eq!(
            tr("Not a phrase anybody has translated"),
            "Not a phrase anybody has translated"
        );
        set_language("en");
    }

    #[test]
    fn the_catalogues_answer_in_their_own_language() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        set_language("es");
        assert_eq!(tr("Close Tab"), "Cerrar pestaña");
        set_language("fr");
        assert_eq!(tr("Close Tab"), "Fermer l'onglet");
        set_language("en");
        assert_eq!(tr("Close Tab"), "Close Tab");
    }

    #[test]
    fn one_file_is_not_one_files() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        // The reason plurals go through the catalogue rather than a
        // format argument: Spanish and French each have their own rule,
        // and English "1 files" was what a single string gave.
        set_language("es");
        assert_eq!(t_n!("{} file", "{} files", 1), "1 archivo");
        assert_eq!(t_n!("{} file", "{} files", 4), "4 archivos");
        set_language("fr");
        assert_eq!(t_n!("{} file", "{} files", 1), "1 fichier");
        assert_eq!(t_n!("{} file", "{} files", 4), "4 fichiers");
        set_language("en");
        assert_eq!(t_n!("{} file", "{} files", 1), "1 file");
        assert_eq!(t_n!("{} file", "{} files", 2), "2 files");
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
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        set_language("en");
        assert_eq!(t!("Save changes to {}?", "main.rs"), "Save changes to main.rs?");
    }

    #[test]
    fn every_catalogue_carries_the_same_phrases() {
        // A phrase translated into one language and forgotten in the
        // other is the failure this catches: both catalogues are
        // written from the same list.
        let spanish = read_mo(SPANISH);
        let french = read_mo(FRENCH);
        let mut missing: Vec<&String> =
            spanish.keys().filter(|key| !french.contains_key(*key)).collect();
        missing.extend(french.keys().filter(|key| !spanish.contains_key(*key)));
        assert!(missing.is_empty(), "not in both catalogues: {missing:?}");
        assert!(spanish.len() > 300, "the catalogue is {} phrases", spanish.len());
    }
}
