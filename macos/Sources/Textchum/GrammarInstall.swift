import AppKit
import TextchumKit

/// Textchum ▸ Install Grammar…: a language the build does not carry,
/// from a checkout of its tree-sitter grammar.
///
/// The compiling and the query rewriting are the core's; this is the
/// part macOS owns: which folder, the seconds a compile takes kept off
/// the main thread, the entry saved as the app's own change, and what
/// to say afterwards.
extension AppDelegate {
    @objc func installGrammar(_ sender: Any?) {
        let panel = NSOpenPanel()
        panel.message = t(
            "Choose a tree-sitter grammar's repository: a folder with src/parser.c and queries/highlights.scm."
        )
        panel.prompt = t("Install")
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let repository = url.path
        let directory = AppPaths.grammarsDirectory.path
        DispatchQueue.global(qos: .userInitiated).async {
            let outcome = CoreLanguages.build(repository: repository, into: directory)
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self.finishGrammarInstall(outcome) }
            }
        }
    }

    private func finishGrammarInstall(_ outcome: CoreLanguages.BuildOutcome) {
        // Reports, not dialogs (6.2): the first line in the status bar,
        // the whole of it in the notification area's list.
        if let error = outcome.error {
            notify(t("The grammar was not installed") + "\n" + error)
            return
        }
        guard let config else { return }
        // Loaded here rather than by the file watcher, which ignores
        // the app's own saves; the documents are told the way a reload
        // tells them, so a file of that type already open is coloured.
        let name = outcome.name
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        let problems = CoreLanguages.load(
            grammarsJSON: "{\"languages\":{\"\(name)\":\(outcome.entryJSON)}}")
        finishGrammarLoading(problems: problems, relearn: true)

        config.setLanguageEntry(name: outcome.name, json: outcome.entryJSON)
        saveConfigAsOwn()
        var lines = [
            "Installed “\(outcome.name)”",
            t("Files of its types open as it from now on, and the ones already open are coloured."),
        ]
        lines.append(contentsOf: problems)
        lines.append(contentsOf: outcome.warnings)
        notify(lines.joined(separator: "\n\n"))
    }
}
