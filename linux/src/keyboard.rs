//! The commands a shortcut can name, by the names the configuration
//! uses.
//!
//! `keys` and the keyboard profiles are written against names shared
//! with the macOS build — `jumpToDefinition`, not `win.definition` —
//! so that a profile means the same thing on both. This is where the
//! two vocabularies meet, and where each command gets the title it
//! wears in Preferences.

use textchum_core::i18n::n_;


/// A command: the name the configuration uses, its title on screen, and
/// the GTK action it drives.
static COMMANDS: &[(&str, &str, &str)] = &[
    ("new", n_("New Tab"), "win.new-tab"),
    ("newWithFormat", n_("New with Format"), "win.new-format-picker"),
    ("open", n_("Open"), "win.open"),
    ("openQuickly", n_("Open Quickly"), "win.quick-open"),
    ("changedFiles", n_("Changed in Branch"), "win.changed-files"),
    ("save", n_("Save"), "win.save"),
    ("saveAs", n_("Save As"), "win.save-as"),
    ("revertToSaved", n_("Revert to Saved"), "win.revert"),
    ("close", n_("Close Tab"), "win.close-tab"),
    ("fold", n_("Fold"), "win.fold"),
    ("foldAll", n_("Fold All"), "win.fold-all"),
    ("unfoldAll", n_("Unfold All"), "win.unfold-all"),
    ("newColumn", n_("New Column"), "win.new-column"),
    ("closeColumn", n_("Close Column"), "win.close-column"),
    ("secondView", n_("Second View"), "win.add-view"),
    ("closeView", n_("Close View"), "win.close-view"),
    ("nextPane", n_("Next Pane"), "win.focus-other-group"),
    ("reopenClosed", n_("Reopen Closed Tab"), "win.reopen-tab"),
    ("fileProperties", n_("File Properties"), "win.file-properties"),
    ("undo", n_("Undo"), "win.undo"),
    ("redo", n_("Redo"), "win.redo"),
    ("find", n_("Find in File"), "win.find"),
    ("findNext", n_("Find Next"), "win.find-next"),
    ("findPrevious", n_("Find Previous"), "win.find-previous"),
    ("findAndReplace", n_("Find and Replace"), "win.find-replace"),
    ("revealInTree", n_("Reveal in Tree"), "win.reveal-in-tree"),
    ("complete", n_("Complete"), "win.complete"),
    ("findInProject", n_("Find in Project"), "win.find-in-project"),
    ("jumpToDefinition", n_("Jump to Definition"), "win.definition"),
    ("findReferences", n_("Find References"), "win.references"),
    ("codeActions", n_("Code Actions"), "win.code-actions"),
    ("renameSymbol", n_("Rename Symbol"), "win.rename"),
    ("formatDocument", n_("Format Document"), "win.format"),
    ("runPreprocessors", n_("Run Save Preprocessors"), "win.preprocess"),
    ("documentOutline", n_("Document Outline"), "win.outline"),
    ("goBack", n_("Go Back"), "win.back"),
    ("goForward", n_("Go Forward"), "win.forward"),
    ("goToLine", n_("Go to Line"), "win.goto-line"),
    ("goToBlockStart", n_("Go to Block Start"), "win.block-start"),
    ("goToBlockEnd", n_("Go to Block End"), "win.block-end"),
    ("blameLine", n_("Blame Line"), "win.blame"),
    ("showDiagnostic", n_("Show Diagnostic for Line"), "win.diagnostic"),
    ("diagnosticList", n_("Diagnostics"), "win.diagnostic-list"),
    ("showHover", n_("Show Documentation for Symbol"), "win.hover"),
    ("commandPalette", n_("Command Palette"), "win.palette"),
    ("toggleNavigator", n_("Toggle Navigator"), "win.sidebar"),
    ("togglePreview", n_("Toggle Preview"), "win.preview"),
    ("togglePathDisplay", n_("Toggle Full Paths"), "win.paths"),
    ("redraw", n_("Redraw Document"), "win.redraw"),
    ("settings", n_("Preferences"), "win.preferences"),
];

/// Every command, as (configuration name, title).
pub fn commands() -> impl Iterator<Item = (&'static str, &'static str)> {
    COMMANDS.iter().map(|(name, title, _)| (*name, *title))
}

/// The GTK action a configuration name drives.
pub fn gtk_action(name: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .find(|(command, _, _)| *command == name)
        .map(|(_, _, action)| *action)
}

/// The shortcut a command comes with, as a spec — what the field shows
/// when no profile and no override has anything to say about it.
pub fn default_spec(name: &str) -> Option<String> {
    let action = gtk_action(name)?;
    crate::default_accel(action).and_then(spec_from_accel)
}

/// `"<Ctrl><Shift>f"` → `"cmd+shift+f"`: the inverse of the parser the
/// accelerators are installed through.
fn spec_from_accel(accel: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut rest = accel;
    while let Some(end) = rest.strip_prefix('<').and_then(|rest| rest.find('>')) {
        let modifier = &rest[1..end + 1];
        parts.push(
            match modifier.to_lowercase().as_str() {
                "ctrl" | "primary" => "cmd",
                "shift" => "shift",
                "alt" => "alt",
                _ => return None,
            }
            .to_string(),
        );
        rest = &rest[end + 2..];
    }
    if rest.is_empty() {
        return None;
    }
    parts.push(rest.to_lowercase());
    Some(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_vocabularies_line_up() {
        assert_eq!(gtk_action("jumpToDefinition"), Some("win.definition"));
        assert_eq!(gtk_action("nonexistent"), None);
    }

    #[test]
    fn an_accelerator_reads_back_as_a_spec() {
        assert_eq!(spec_from_accel("<Ctrl><Shift>f").as_deref(), Some("cmd+shift+f"));
        assert_eq!(spec_from_accel("F12").as_deref(), Some("f12"));
        assert_eq!(spec_from_accel("<Alt>Left").as_deref(), Some("alt+left"));
        assert_eq!(spec_from_accel("<Ctrl>"), None);
    }
}
