//! The commands a shortcut can name, by the names the configuration
//! uses.
//!
//! `keys` and the keyboard profiles are written against names shared
//! with the macOS build — `jumpToDefinition`, not `win.definition` —
//! so that a profile means the same thing on both. This is where the
//! two vocabularies meet, and where each command gets the title it
//! wears in Preferences.

/// A command: the name the configuration uses, its title on screen, and
/// the GTK action it drives.
static COMMANDS: &[(&str, &str, &str)] = &[
    ("new", "New Tab", "win.new-tab"),
    ("newWithFormat", "New with Format", "win.new-format-picker"),
    ("open", "Open", "win.open"),
    ("openQuickly", "Open Quickly", "win.quick-open"),
    ("save", "Save", "win.save"),
    ("saveAs", "Save As", "win.save-as"),
    ("revertToSaved", "Revert to Saved", "win.revert"),
    ("close", "Close Tab", "win.close-tab"),
    ("fold", "Fold", "win.fold"),
    ("foldAll", "Fold All", "win.fold-all"),
    ("unfoldAll", "Unfold All", "win.unfold-all"),
    ("splitEditor", "Split Editor", "win.split"),
    ("closeSplit", "Close Split", "win.unsplit"),
    ("reopenClosed", "Reopen Closed Tab", "win.reopen-tab"),
    ("fileProperties", "File Properties", "win.file-properties"),
    ("undo", "Undo", "win.undo"),
    ("redo", "Redo", "win.redo"),
    ("find", "Find in File", "win.find"),
    ("findInProject", "Find in Project", "win.find-in-project"),
    ("jumpToDefinition", "Jump to Definition", "win.definition"),
    ("findReferences", "Find References", "win.references"),
    ("codeActions", "Code Actions", "win.code-actions"),
    ("renameSymbol", "Rename Symbol", "win.rename"),
    ("formatDocument", "Format Document", "win.format"),
    ("runPreprocessors", "Run Save Preprocessors", "win.preprocess"),
    ("documentOutline", "Document Outline", "win.outline"),
    ("goBack", "Go Back", "win.back"),
    ("goForward", "Go Forward", "win.forward"),
    ("goToLine", "Go to Line", "win.goto-line"),
    ("goToBlockStart", "Go to Block Start", "win.block-start"),
    ("goToBlockEnd", "Go to Block End", "win.block-end"),
    ("blameLine", "Blame Line", "win.blame"),
    ("showDiagnostic", "Show Diagnostic for Line", "win.diagnostic"),
    ("diagnosticList", "Diagnostics", "win.diagnostic-list"),
    ("showHover", "Show Documentation for Symbol", "win.hover"),
    ("commandPalette", "Command Palette", "win.palette"),
    ("toggleNavigator", "Toggle Navigator", "win.sidebar"),
    ("togglePreview", "Toggle Preview", "win.preview"),
    ("togglePathDisplay", "Toggle Full Paths", "win.paths"),
    ("redraw", "Redraw Document", "win.redraw"),
    ("settings", "Preferences", "win.preferences"),
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
