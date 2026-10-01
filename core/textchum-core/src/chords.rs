//! The chord window's menu: commands by one key each.
//!
//! A shortcut of three or four keys has to be remembered and then
//! reached for. The alternative modal editors arrived at is a leader:
//! one gesture opens a panel that lists what can be done, one key per
//! command, grouped so that each list is short enough to read, and the
//! next key does it. Here the gesture is two modifier keys pressed
//! together, and this is the list.
//!
//! It is a table in the core so both shells show the same keys. Every
//! entry names an action by the name the shortcut system already uses
//! for it (`save`, `findInProject`, …); each shell knows how to run
//! one of those, and its tests check that it knows all of these.

use serde_json::Value;

use crate::i18n::{n_, tr};

/// What a key in the window does.
pub enum Target {
    /// Runs the action with this name and closes the window.
    Action(&'static str),
    /// Opens a further list.
    Group(Vec<Entry>),
}

/// One line of the window: a key, what it is called, what it does.
pub struct Entry {
    pub key: char,
    pub label: String,
    pub target: Target,
}

fn action(key: char, label: &str, name: &'static str) -> Entry {
    Entry { key, label: tr(label), target: Target::Action(name) }
}

fn group(key: char, label: &str, entries: Vec<Entry>) -> Entry {
    Entry { key, label: tr(label), target: Target::Group(entries) }
}

/// The menu, top level first. Keys are lower-case letters, digits or
/// punctuation that needs no shift, and are mnemonic within a group.
///
/// A command goes by the name it has in the menus and in Preferences,
/// which is already a phrase the translations carry; only the two
/// groups that are no menu's name are marked here for extraction.
pub fn menu() -> Vec<Entry> {
    vec![
        group(
            'f',
            "File",
            vec![
                action('n', "New Tab", "new"),
                action('o', "Open", "open"),
                action('q', "Open Quickly", "openQuickly"),
                action('s', "Save", "save"),
                action('a', "Save As", "saveAs"),
                action('r', "Revert to Saved", "revertToSaved"),
                action('w', "Close Tab", "close"),
                action('t', "Reopen Closed Tab", "reopenClosed"),
                action('c', "Changed in Branch", "changedFiles"),
                action('p', "File Properties", "fileProperties"),
            ],
        ),
        group(
            'c',
            n_("Code"),
            vec![
                action('d', "Jump to Definition", "jumpToDefinition"),
                action('r', "Find References", "findReferences"),
                action('c', "Show Callers", "showCallers"),
                action('a', "Code Actions", "codeActions"),
                action('n', "Rename Symbol", "renameSymbol"),
                action('f', "Format Document", "formatDocument"),
                action('p', "Run Save Preprocessors", "runPreprocessors"),
                action('h', "Show Documentation for Symbol", "showHover"),
                action('e', "Show Diagnostic for Line", "showDiagnostic"),
                action('l', "Diagnostics", "diagnosticList"),
            ],
        ),
        group(
            'g',
            "Go",
            vec![
                action('l', "Go to Line", "goToLine"),
                action('s', "Go to Symbol in Project", "projectSymbols"),
                action('o', "Document Outline", "documentOutline"),
                action('b', "Go Back", "goBack"),
                action('f', "Go Forward", "goForward"),
                action('[', "Go to Block Start", "goToBlockStart"),
                action(']', "Go to Block End", "goToBlockEnd"),
                action('t', "Reveal in Tree", "revealInTree"),
            ],
        ),
        group(
            's',
            n_("Search"),
            vec![
                action('p', "Find in Project", "findInProject"),
                action('r', "Find and Replace", "findAndReplace"),
            ],
        ),
        group(
            'v',
            "View",
            vec![
                action('n', "Toggle Navigator", "toggleNavigator"),
                action('p', "Toggle Preview", "togglePreview"),
                action('c', "New Column", "newColumn"),
                action('x', "Close Column", "closeColumn"),
                action('v', "Second View", "secondView"),
                action('k', "Close View", "closeView"),
                action('o', "Next Pane", "nextPane"),
                action('f', "Fold", "fold"),
                action('a', "Fold All", "foldAll"),
                action('u', "Unfold All", "unfoldAll"),
                action('h', "Toggle Full Paths", "togglePathDisplay"),
            ],
        ),
        // The ones reached for most, without a group between.
        action('p', "Command Palette", "commandPalette"),
        action('b', "Blame Line", "blameLine"),
        action(',', "Preferences", "settings"),
    ]
}

/// Every action the menu can run, for a shell to check it knows them.
pub fn actions() -> Vec<&'static str> {
    fn collect(entries: &[Entry], into: &mut Vec<&'static str>) {
        for entry in entries {
            match &entry.target {
                Target::Action(name) => into.push(name),
                Target::Group(inner) => collect(inner, into),
            }
        }
    }
    let mut names = Vec::new();
    collect(&menu(), &mut names);
    names
}

/// The list on show after the groups with these keys were opened in
/// turn, and the names of those groups for a heading. A key that opens
/// no group ends the walk where it stands.
pub fn list_at(path: &[char]) -> (Vec<Entry>, Vec<String>) {
    let mut entries = menu();
    let mut trail = Vec::new();
    for key in path {
        let Some(at) = entries
            .iter()
            .position(|entry| entry.key == *key && matches!(entry.target, Target::Group(_)))
        else {
            break;
        };
        let entry = entries.swap_remove(at);
        if let Target::Group(inner) = entry.target {
            trail.push(entry.label);
            entries = inner;
        }
    }
    (entries, trail)
}

/// The menu as JSON, for a shell across the C boundary: an array of
/// `{key, label, action}` and `{key, label, items}`.
pub fn menu_json() -> String {
    fn encode(entries: &[Entry]) -> Value {
        Value::Array(
            entries
                .iter()
                .map(|entry| {
                    let key = entry.key.to_string();
                    match &entry.target {
                        Target::Action(name) => {
                            serde_json::json!({"key": key, "label": entry.label, "action": name})
                        }
                        Target::Group(inner) => serde_json::json!({
                            "key": key, "label": entry.label, "items": encode(inner),
                        }),
                    }
                })
                .collect(),
        )
    }
    encode(&menu()).to_string()
}

/// The pairs of modifier keys that can open the window, as the
/// configuration spells them. `cmd` is not what it is in a shortcut,
/// where it stands for Ctrl on Linux: a pair is two keys held, so here
/// it is the key itself, Command on macOS and Super on Linux.
pub const MODIFIER_PAIRS: &[&str] = &["ctrl+alt", "alt+cmd", "ctrl+cmd", "ctrl+shift", "alt+shift"];

/// A pair as the configuration should hold it, or `None` for anything
/// that is not one: the window is off unless a known pair is named.
pub fn modifier_pair(spelled: &str) -> Option<&'static str> {
    let mut parts: Vec<String> = spelled
        .split('+')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect();
    parts.sort();
    MODIFIER_PAIRS.iter().copied().find(|pair| {
        let mut wanted: Vec<&str> = pair.split('+').collect();
        wanted.sort_unstable();
        wanted == parts
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_are_distinct(entries: &[Entry], path: &str) {
        let mut seen = Vec::new();
        for entry in entries {
            assert!(
                !seen.contains(&entry.key),
                "{path}: the key {:?} is given twice",
                entry.key
            );
            assert!(
                entry.key.is_ascii() && !entry.key.is_ascii_uppercase() && !entry.key.is_whitespace(),
                "{path}: {:?} cannot be typed without shift",
                entry.key
            );
            seen.push(entry.key);
            if let Target::Group(inner) = &entry.target {
                assert!(!inner.is_empty(), "{path}{}: an empty group", entry.key);
                keys_are_distinct(inner, &format!("{path}{} ", entry.key));
            }
        }
    }

    #[test]
    fn every_key_is_one_thing_in_its_list_and_easy_to_type() {
        keys_are_distinct(&menu(), "");
    }

    #[test]
    fn a_path_of_group_keys_leads_to_that_group() {
        let (top, trail) = list_at(&[]);
        assert_eq!(top.len(), menu().len());
        assert!(trail.is_empty());

        let (file, trail) = list_at(&['f']);
        assert_eq!(trail.len(), 1);
        assert!(file.iter().any(|entry| matches!(entry.target, Target::Action("save"))));

        // `p` is a command at the top, not a group: the walk stays put.
        let (still_top, trail) = list_at(&['p']);
        assert_eq!(still_top.len(), menu().len());
        assert!(trail.is_empty());
    }

    #[test]
    fn an_action_is_offered_once() {
        let mut names = actions();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn the_json_carries_groups_and_actions() {
        let parsed: Value = serde_json::from_str(&menu_json()).unwrap();
        let file = &parsed[0];
        assert_eq!(file["key"], "f");
        assert!(file["items"].as_array().unwrap().iter().any(|item| item["action"] == "save"));
        assert!(parsed.as_array().unwrap().iter().any(|item| item["action"] == "commandPalette"));
    }

    #[test]
    fn a_pair_is_two_known_modifiers_in_any_order() {
        assert_eq!(modifier_pair("ctrl+alt"), Some("ctrl+alt"));
        assert_eq!(modifier_pair("Alt + Ctrl"), Some("ctrl+alt"));
        assert_eq!(modifier_pair("cmd+alt"), Some("alt+cmd"));
        assert_eq!(modifier_pair("ctrl"), None);
        assert_eq!(modifier_pair("ctrl+alt+shift"), None);
        assert_eq!(modifier_pair(""), None);
    }
}
