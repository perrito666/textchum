import AppKit
import TextchumKit
import UniformTypeIdentifiers

/// Textchum ▸ Import Theme: bringing colours over from the editor
/// someone used before this one.
///
/// Both editors describe a theme the same way underneath — a colour per
/// TextMate scope — so the reading is the core's job and this is the
/// part macOS owns: which file, and what to say afterwards.
///
/// The chooser takes a folder as readily as a file. A VS Code theme
/// lives inside an extension directory that may contribute several, and
/// a TextMate bundle keeps its own in `Themes/`; someone who installed a
/// pack of six wanted the pack.
extension AppDelegate {
    @objc func importVSCodeTheme(_ sender: Any?) {
        importTheme(from: .vsCode)
    }

    @objc func importTextMateTheme(_ sender: Any?) {
        importTheme(from: .textMate)
    }

    private func importTheme(from source: CoreTheme.ImportSource) {
        let panel = NSOpenPanel()
        panel.message = "Choose a \(source.label) theme file, or a folder holding some."
        panel.prompt = "Import"
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = source.extensions.compactMap {
            UTType(filenameExtension: $0)
        }
        // A folder is a legitimate choice, and a panel restricted to
        // JSON refuses to select one otherwise.
        panel.treatsFilePackagesAsDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else { return }

        let directory = ThemeFiles.directory
        let outcome = CoreTheme.importThemes(
            at: url.path, from: source, into: directory.path)
        report(outcome, from: source)
        // Wearing the theme is the point; the first one read is the one
        // to put on.
        if let first = outcome.written.first {
            selectTheme(named: first)
        }
    }

    /// Says what happened, and says the parts that are easy to miss:
    /// which side of the palette was filled, and any scope that had
    /// nowhere to go.
    private func report(_ outcome: CoreTheme.ImportOutcome, from source: CoreTheme.ImportSource) {
        let alert = NSAlert()
        if outcome.written.isEmpty {
            alert.alertStyle = .warning
            alert.messageText = "Nothing was imported"
            alert.informativeText =
                outcome.errors.first ?? "No \(source.label) theme was found there."
            alert.runModal()
            return
        }

        alert.alertStyle = .informational
        alert.messageText =
            outcome.written.count == 1
            ? "Imported “\(outcome.written[0])”"
            : "Imported \(outcome.written.count) themes"

        var lines: [String] = []
        for (name, appearance) in zip(outcome.written, outcome.appearances) {
            lines.append(
                "\(name) — written for a \(appearance) background, so its \(appearance) "
                    + "colours are set and the other side keeps Textchum's.")
        }
        if !outcome.unmapped.isEmpty {
            let shown = outcome.unmapped.prefix(6).joined(separator: ", ")
            let rest = outcome.unmapped.count - min(6, outcome.unmapped.count)
            lines.append(
                "Nothing here answers to \(shown)\(rest > 0 ? ", and \(rest) more" : "")"
                    + " — those colours are unused.")
        }
        if !outcome.errors.isEmpty {
            lines.append(contentsOf: outcome.errors)
        }
        alert.informativeText = lines.joined(separator: "\n\n")
        alert.runModal()
    }
}
