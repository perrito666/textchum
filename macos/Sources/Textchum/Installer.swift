import AppKit
import TextchumKit

/// Installing what a document needs and does not have: the language
/// servers, debuggers and save formatters found missing, each with the
/// ways this machine can install it (`CoreInstall`). The editor does not
/// tell the person to run a command it can run itself (6.4): the install
/// runs in the run panel, and when it succeeds the servers that were
/// missing are started without being asked.
extension AppDelegate {
    /// Textchum → Install Missing Tools…, and the status bar's Install
    /// button.
    @objc func installMissingTools(_ sender: Any?) {
        guard let workbench = Workbench.all.first(where: { $0.window?.isKeyWindow == true }) ?? Workbench.all.first
        else { return }
        offerInstalls(in: workbench)
    }

    /// Lists what is missing for the document in front of `workbench`,
    /// and the ways to install each, the one that suits the project
    /// first; Return installs the first.
    func offerInstalls(in workbench: Workbench) {
        let document = workbench.focusedDocument
        guard let path = document?.coreDocument.path ?? document?.projectRoot else {
            workbench.showNotice(t("Nothing is missing: no document is open"))
            return
        }
        let offers = CoreInstall.offers(forPath: path)
        guard !offers.isEmpty else {
            if let document {
                document.showHint(t("Nothing this document needs is missing."))
            } else {
                workbench.showNotice(t("Nothing this document needs is missing."))
            }
            workbench.refreshInstallOffers()
            return
        }
        let (rows, choices) = Self.installRows(offers)
        installPanel.show(
            rows: rows, over: workbench.window, title: t("Install Missing Tools"), monospaced: false
        ) { [weak self, weak workbench] index in
            guard let self, let workbench, choices.indices.contains(index) else { return }
            let (offer, method) = choices[index]
            self.install(offer, with: method, in: workbench)
        }
    }

    /// The list's rows: each missing program under a heading that says
    /// what it was for, its ways to install it as items, or what it
    /// comes with when nothing here can install it. Beside the rows, what
    /// each item installs and how.
    static func installRows(
        _ offers: [CoreInstall.Offer]
    ) -> ([ListPanel.Row], [(CoreInstall.Offer, CoreInstall.Method)]) {
        var rows: [ListPanel.Row] = []
        var choices: [(CoreInstall.Offer, CoreInstall.Method)] = []
        for offer in offers {
            rows.append(.heading("\(offer.program) — \(offer.purpose)"))
            for method in offer.methods {
                // The command is what will run, set apart from the
                // words about where it goes.
                let styled = NSMutableAttributedString(
                    string: method.title + "   ",
                    attributes: [.font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
                styled.append(
                    NSAttributedString(
                        string: method.command,
                        attributes: [
                            .font: NSFont.monospacedSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular),
                            .foregroundColor: NSColor.secondaryLabelColor,
                        ]))
                rows.append(.item("\(method.title): \(method.command)", styled: styled))
                choices.append((offer, method))
            }
            if offer.methods.isEmpty {
                let notes = offer.notes.isEmpty ? [t("nothing on this machine can install it")] : offer.notes
                for note in notes {
                    rows.append(.heading("   " + note))
                }
            }
        }
        return (rows, choices)
    }

    /// Runs `method` for `offer` in the workbench's run panel, or, for
    /// what needs an administrator, hands the command over.
    func install(_ offer: CoreInstall.Offer, with method: CoreInstall.Method, in workbench: Workbench) {
        guard method.runs else {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(method.command, forType: .string)
            notify(
                t(
                    "Copied “{}”: it needs an administrator's password, which the run panel cannot ask for. Paste it into a terminal.",
                    method.command))
            return
        }
        run(method.command, in: workbench, cwd: method.directory)
        if let id = workbench.runPanel.runID {
            pendingInstalls[id] = offer
        }
    }

    /// A run ended: if it was an install, says how it went, and on
    /// success starts what was missing.
    func installFinished(run id: UInt64, in workbench: Workbench) {
        guard let offer = pendingInstalls.removeValue(forKey: id) else { return }
        if workbench.runPanel.exitCode == 0 {
            CoreInstall.forget(program: offer.program)
            notify(t("{} is installed", offer.program))
            startWhatWasMissing()
            retryAfterInstall.removeValue(forKey: offer.program)?()
        } else {
            notify(t("Installing {} failed; the run panel has what the installer said.", offer.program))
        }
        for workbench in Workbench.all {
            workbench.refreshInstallOffers()
        }
    }
}
