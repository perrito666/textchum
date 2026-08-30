//! Keyboard shortcuts: the bindings, and the profiles that set them.
//!
//! `config.json`'s `keys` is an action-to-shortcut map, and it was
//! hand-edited only. People arrive from another editor with its
//! shortcuts in their fingers, so the ones those editors are known for
//! are bundled here and can be picked instead of retyped.
//!
//! A profile is not a replacement for the editor's own bindings: it
//! names the ones that differ, and everything it says nothing about
//! keeps the shortcut it had. `keys` still wins over whichever profile
//! is chosen, so a single binding can be changed without leaving one.
//!
//! Shortcut specs are the same on both platforms: `cmd` is the primary
//! modifier — Command on macOS, Ctrl on Linux — and `ctrl` is Control
//! on both.

use std::collections::BTreeMap;

use serde_json::Value;

/// A named set of bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    /// Stable identifier, stored in the configuration.
    pub id: &'static str,
    /// What to call it on screen.
    pub name: &'static str,
    /// Action name to shortcut spec, for the actions this profile
    /// moves. Actions absent from it keep their own shortcut.
    pub bindings: &'static [(&'static str, &'static str)],
}

/// The profiles that ship with the editor: the shortcuts of the three
/// editors people arrive from.
pub fn bundled() -> &'static [Profile] {
    BUNDLED
}

/// A bundled profile by id.
pub fn bundled_by_id(id: &str) -> Option<&'static Profile> {
    BUNDLED.iter().find(|profile| profile.id == id)
}

static BUNDLED: &[Profile] = &[
    // Taken from Visual Studio Code's own printable reference for
    // macOS: ⇧⌘P palette, ⌘P go to file, ⌃G go to line, ⇧⌘O go to
    // symbol, ⇧⌘M problems, F12 definition, ⇧F12 references, F2
    // rename, ⇧⌥F format, ⌘B side bar, ⌥⌘[ and ⌥⌘] fold and unfold,
    // ⌥← and ⌥→ back and forward, ⇧⌘T reopen, ⌃Space suggestions,
    // ⌘G and ⇧⌘G find next and previous, ⌘\ split.
    Profile {
        id: "vscode",
        name: "Visual Studio Code",
        bindings: &[
            ("new", "cmd+n"),
            ("openQuickly", "cmd+p"),
            ("commandPalette", "cmd+shift+p"),
            ("find", "cmd+f"),
            ("findNext", "cmd+g"),
            ("findPrevious", "cmd+shift+g"),
            ("findAndReplace", "cmd+alt+f"),
            ("findInProject", "cmd+shift+f"),
            ("jumpToDefinition", "f12"),
            ("findReferences", "shift+f12"),
            ("renameSymbol", "f2"),
            ("codeActions", "cmd+period"),
            ("formatDocument", "shift+alt+f"),
            ("goToLine", "ctrl+g"),
            ("toggleNavigator", "cmd+b"),
            ("diagnosticList", "cmd+shift+m"),
            ("documentOutline", "cmd+shift+o"),
            ("revealInTree", "cmd+shift+e"),
            ("goBack", "alt+left"),
            ("goForward", "alt+right"),
            ("fold", "cmd+alt+bracketleft"),
            ("unfoldAll", "cmd+alt+bracketright"),
            ("reopenClosed", "cmd+shift+t"),
            ("newColumn", "cmd+backslash"),
            ("complete", "ctrl+space"),
        ],
    },
    // Sublime Text's documented defaults: ⌘P goto anything, ⇧⌘P
    // palette, ⌘F find, ⇧⌘F find in files, ⌃G goto line, ⌘R goto
    // symbol, ⇧⌘T reopen, ⌥⌘[ and ⌥⌘] fold and unfold. Sublime has no
    // language-server commands of its own, so definition, references
    // and rename keep the shortcuts Textchum gives them.
    Profile {
        id: "sublime",
        name: "Sublime Text",
        bindings: &[
            ("new", "cmd+n"),
            ("openQuickly", "cmd+p"),
            ("commandPalette", "cmd+shift+p"),
            ("find", "cmd+f"),
            ("findNext", "cmd+g"),
            ("findPrevious", "cmd+shift+g"),
            ("findAndReplace", "cmd+alt+f"),
            ("findInProject", "cmd+shift+f"),
            ("goToLine", "ctrl+g"),
            ("documentOutline", "cmd+r"),
            ("fold", "cmd+alt+bracketleft"),
            ("unfoldAll", "cmd+alt+bracketright"),
            ("reopenClosed", "cmd+shift+t"),
            ("complete", "ctrl+space"),
        ],
    },
    // IntelliJ IDEA's macOS keymap: ⇧⌘O go to file, ⇧⌘A find action,
    // ⌘B declaration, ⌥F7 find usages, ⇧F6 rename, ⌥⌘L reformat, ⌘L
    // go to line, ⌘F find, ⌘G and ⇧⌘G next and previous, ⇧⌘F find in
    // path, ⌘F12 file structure, ⌘[ and ⌘] back and forward, ⌘1 the
    // project window, ⌃Space basic completion.
    Profile {
        id: "intellij",
        name: "IntelliJ IDEA",
        bindings: &[
            ("openQuickly", "cmd+shift+o"),
            ("commandPalette", "cmd+shift+a"),
            ("jumpToDefinition", "cmd+b"),
            ("findReferences", "alt+f7"),
            ("renameSymbol", "shift+f6"),
            ("codeActions", "alt+enter"),
            ("formatDocument", "cmd+alt+l"),
            ("goToLine", "cmd+l"),
            ("find", "cmd+f"),
            ("findNext", "cmd+g"),
            ("findPrevious", "cmd+shift+g"),
            ("findAndReplace", "cmd+r"),
            ("findInProject", "cmd+shift+f"),
            ("documentOutline", "cmd+f12"),
            ("toggleNavigator", "cmd+1"),
            ("goBack", "cmd+bracketleft"),
            ("goForward", "cmd+bracketright"),
            ("reopenClosed", "cmd+shift+t"),
            ("complete", "ctrl+space"),
        ],
    },
];

/// The bindings a profile name resolves to.
///
/// `saved` is the configuration's `key_profiles` object — the profiles
/// someone saved themselves, which may also redefine a bundled id.
/// An unknown name resolves to nothing, which is the editor's own
/// bindings.
pub fn profile_bindings(name: &str, saved: &str) -> BTreeMap<String, String> {
    if name.is_empty() {
        return BTreeMap::new();
    }
    if let Some(bindings) = saved_profile(name, saved) {
        return bindings;
    }
    bundled_by_id(name)
        .map(|profile| {
            profile
                .bindings
                .iter()
                .map(|(action, spec)| ((*action).to_string(), (*spec).to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// The bindings that actually apply: the profile's, with `keys` on top.
///
/// An override wins over the profile, so one shortcut can be changed
/// without leaving the profile it came from.
pub fn effective(profile: &str, saved: &str, overrides: &str) -> BTreeMap<String, String> {
    let mut bindings = profile_bindings(profile, saved);
    let Ok(Value::Object(overrides)) = serde_json::from_str::<Value>(overrides) else {
        return bindings;
    };
    for (action, spec) in overrides {
        if let Some(spec) = spec.as_str() {
            bindings.insert(action, spec.to_string());
        }
    }
    bindings
}

/// Every profile that can be chosen, as `(id, name)` — the bundled ones
/// and the saved ones, a saved profile replacing the bundled id it
/// reuses.
pub fn choices(saved: &str) -> Vec<(String, String)> {
    let mut choices: Vec<(String, String)> = BUNDLED
        .iter()
        .map(|profile| (profile.id.to_string(), profile.name.to_string()))
        .collect();
    let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(saved) else {
        return choices;
    };
    for name in saved.keys() {
        if !choices.iter().any(|(id, _)| id == name) {
            choices.push((name.clone(), name.clone()));
        }
    }
    choices
}

/// The bindings as JSON, for the shells.
pub fn to_json(bindings: &BTreeMap<String, String>) -> String {
    serde_json::to_string(bindings).unwrap_or_else(|_| "{}".into())
}

fn saved_profile(name: &str, saved: &str) -> Option<BTreeMap<String, String>> {
    let parsed = serde_json::from_str::<Value>(saved).ok()?;
    let entry = parsed.get(name)?.as_object()?;
    Some(
        entry
            .iter()
            .filter_map(|(action, spec)| {
                spec.as_str().map(|spec| (action.clone(), spec.to_string()))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundled_profile_resolves_to_its_bindings() {
        let bindings = profile_bindings("vscode", "{}");
        assert_eq!(bindings.get("openQuickly").map(String::as_str), Some("cmd+p"));
        // A profile names what it moves; the rest keep their own.
        assert!(!bindings.contains_key("toggleLineNumbers"));
    }

    #[test]
    fn no_profile_is_the_editors_own_bindings() {
        assert!(profile_bindings("", "{}").is_empty());
        assert!(profile_bindings("nonexistent", "{}").is_empty());
    }

    #[test]
    fn a_saved_profile_can_redefine_a_bundled_id() {
        let saved = r#"{"vscode": {"openQuickly": "cmd+shift+p"}}"#;
        let bindings = profile_bindings("vscode", saved);
        assert_eq!(
            bindings.get("openQuickly").map(String::as_str),
            Some("cmd+shift+p")
        );
        // Wholly replaced, not merged: what the profile says is what it
        // is, so a saved one can take a binding away.
        assert_eq!(bindings.len(), 1);
    }

    #[test]
    fn an_override_wins_over_the_profile() {
        let bindings = effective("vscode", "{}", r#"{"openQuickly": "cmd+t"}"#);
        assert_eq!(bindings.get("openQuickly").map(String::as_str), Some("cmd+t"));
        // And the rest of the profile still applies.
        assert_eq!(bindings.get("renameSymbol").map(String::as_str), Some("f2"));
    }

    #[test]
    fn overrides_apply_without_a_profile() {
        let bindings = effective("", "{}", r#"{"goToLine": "cmd+g"}"#);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings.get("goToLine").map(String::as_str), Some("cmd+g"));
    }

    #[test]
    fn broken_json_is_no_bindings_rather_than_a_panic() {
        assert!(effective("", "{", "not json").is_empty());
    }

    #[test]
    fn the_choices_are_the_bundled_ones_plus_the_saved() {
        let choices = choices(r#"{"mine": {"find": "cmd+f"}}"#);
        assert!(choices.iter().any(|(id, _)| id == "vscode"));
        assert!(choices.iter().any(|(id, _)| id == "mine"));
        // A saved profile reusing a bundled id appears once.
        let choices = choices_len(r#"{"vscode": {}}"#);
        assert_eq!(choices, BUNDLED.len());
    }

    fn choices_len(saved: &str) -> usize {
        choices(saved).len()
    }
}
