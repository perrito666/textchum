import AppKit
import TextchumKit
import UniformTypeIdentifiers
import WebKit

/// The Markdown preview's web view: its context menu can save the
/// rendered page as a PDF, which is often the deliverable — a note to
/// send, a document to attach.
final class PreviewWebView: WKWebView {
    /// What the save panel proposes: the document's name with its
    /// extension traded for .pdf.
    var suggestedPDFName = "document.pdf"

    override func willOpenMenu(_ menu: NSMenu, with event: NSEvent) {
        super.willOpenMenu(menu, with: event)
        menu.addItem(.separator())
        let item = NSMenuItem(
            title: t("Save as PDF…"), action: #selector(saveAsPDF(_:)), keyEquivalent: "")
        item.target = self
        menu.addItem(item)
    }

    @objc private func saveAsPDF(_ sender: Any?) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.pdf]
        panel.nameFieldStringValue = suggestedPDFName
        guard let window else { return }
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url, let self else { return }
            // A PDF is paper: it renders in the light palette whatever
            // the window looks like, or dark mode would hand over
            // near-white text on a white page.
            let previous = self.appearance
            self.appearance = NSAppearance(named: .aqua)
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                self.renderPDF(to: url, restoring: previous)
            }
        }
    }

    private func renderPDF(to url: URL, restoring previous: NSAppearance?) {
        createPDF { [weak self] result in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.appearance = previous }
            }
                switch result {
                case .success(let data):
                    do {
                        try data.write(to: url)
                    } catch {
                        NSSound.beep()
                        NSLog("preview pdf: \(error)")
                    }
                case .failure(let error):
                    NSSound.beep()
                    NSLog("preview pdf: \(error)")
                }
        }
    }
}
