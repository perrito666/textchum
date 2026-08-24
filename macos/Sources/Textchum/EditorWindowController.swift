import AppKit
import TextchumKit
import UniformTypeIdentifiers

/// One editor window: a text view kept in lockstep with a core document.
///
/// The synchronization protocol — the most delicate piece of the app — is:
///
/// 1. The core document is the source of truth; the text view's storage is
///    a display cache.
/// 2. Every change AppKit is about to make (typing, paste, drop — they all
///    funnel through `shouldChangeTextIn`) is applied to the core document
///    *first*, as the same UTF-16 range edit. If the core rejects it, the
///    view change is refused too, so the two sides can only move together.
/// 3. Undo and redo run in the opposite direction: the core pops its
///    history and reports the edit it performed; the window replays it on
///    the text view's storage directly (which bypasses the delegate, so it
///    is not routed to the core a second time).
/// 4. Debug builds assert both sides are byte-identical after every change.
final class EditorWindowController: NSWindowController {
    // Named to avoid NSWindowController's own `document` property.
    let coreDocument: CoreDocument
    private var textView: NSTextView?
    /// True while the next selection change is caused by an edit we already
    /// know about, so it should not break undo coalescing.
    private var selectionChangeIsFromEditing = false

    init(document: CoreDocument, settings: EditorSettings? = nil) {
        self.coreDocument = document

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 720, height: 480),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.center()
        window.tabbingMode = .automatic
        window.tabbingIdentifier = "textchum-editor"
        super.init(window: window)
        window.delegate = self

        let scrollView = NSTextView.scrollableTextView()
        let textView = scrollView.documentView as! NSTextView
        textView.font = .monospacedSystemFont(ofSize: 13, weight: .regular)
        textView.isRichText = false
        // The core owns history; AppKit's own undo stack stays out of play.
        textView.allowsUndo = false
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.delegate = self
        self.textView = textView

        textView.string = coreDocument.text
        window.contentView = scrollView
        if let settings {
            apply(settings: settings)
        }
        updateChrome()
    }

    /// Applies configuration-derived settings to the view: the font, and
    /// tab stops sized to the configured width in that font.
    func apply(settings: EditorSettings) {
        guard let textView else { return }
        let paragraphStyle = NSMutableParagraphStyle()
        let spaceWidth = (" " as NSString).size(withAttributes: [.font: settings.font]).width
        paragraphStyle.tabStops = []
        paragraphStyle.defaultTabInterval = spaceWidth * CGFloat(settings.tabWidth)

        textView.font = settings.font
        textView.defaultParagraphStyle = paragraphStyle
        textView.typingAttributes = [
            .font: settings.font,
            .paragraphStyle: paragraphStyle,
            .foregroundColor: NSColor.textColor,
        ]
        // Restyle existing text too; the document is plain text, so
        // uniform attributes are correct by definition.
        if let storage = textView.textStorage {
            storage.setAttributes(
                textView.typingAttributes,
                range: NSRange(location: 0, length: storage.length)
            )
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("EditorWindowController is created in code")
    }

    /// Refreshes everything the window shows about the document: title,
    /// edited marker, represented file, and the encoding/size subtitle.
    private func updateChrome() {
        guard let window else { return }
        if let path = coreDocument.path {
            window.representedURL = URL(fileURLWithPath: path)
            window.title = URL(fileURLWithPath: path).lastPathComponent
        } else {
            window.representedURL = nil
            window.title = "Untitled"
        }
        window.subtitle = "\(coreDocument.encodingName) · \(coreDocument.lengthInBytes) bytes"
        window.isDocumentEdited = coreDocument.isDirty
    }

    /// Debug-only invariant check: the display cache must equal the core.
    private func assertInSync() {
        #if DEBUG
            if let textView, coreDocument.text != textView.string {
                assertionFailure("core document and text view diverged")
            }
        #endif
    }

    // MARK: Undo / redo

    @objc func performUndo(_ sender: Any?) {
        if let edit = coreDocument.undo() {
            replay(edit)
        }
    }

    @objc func performRedo(_ sender: Any?) {
        if let edit = coreDocument.redo() {
            replay(edit)
        }
    }

    /// Applies a core-reported edit to the display cache. Storage mutations
    /// do not go through the text view delegate, so this cannot echo back
    /// into the core.
    private func replay(_ edit: CoreDocument.AppliedEdit) {
        guard let textView, let storage = textView.textStorage else { return }
        storage.beginEditing()
        storage.replaceCharacters(in: edit.range, with: edit.replacement)
        let insertedLength = (edit.replacement as NSString).length
        if insertedLength > 0 {
            storage.setAttributes(
                textView.typingAttributes,
                range: NSRange(location: edit.range.location, length: insertedLength)
            )
        }
        storage.endEditing()

        // Put the caret at the end of the replayed change and reveal it.
        selectionChangeIsFromEditing = true
        let caret = edit.range.location + insertedLength
        textView.setSelectedRange(NSRange(location: caret, length: 0))
        textView.scrollRangeToVisible(NSRange(location: caret, length: 0))

        updateChrome()
        assertInSync()
    }

    // MARK: Saving

    /// Saves, asking for a location if the document has none. Returns
    /// whether the document ended up saved.
    @discardableResult
    func saveInteractively() -> Bool {
        guard coreDocument.path != nil else { return saveAsInteractively() }
        do {
            try coreDocument.save()
            updateChrome()
            return true
        } catch {
            presentError("Could not save the document.", details: "\(error)")
            return false
        }
    }

    /// Runs a save panel, then saves. Returns whether a save happened.
    @discardableResult
    func saveAsInteractively() -> Bool {
        let panel = NSSavePanel()
        panel.canCreateDirectories = true
        panel.nameFieldStringValue = window?.title == "Untitled" ? "Untitled.txt" : window!.title
        guard panel.runModal() == .OK, let url = panel.url else { return false }
        do {
            try coreDocument.save(to: url.path)
            updateChrome()
            return true
        } catch {
            presentError("Could not save to \(url.lastPathComponent).", details: "\(error)")
            return false
        }
    }

    @objc func saveDocument(_ sender: Any?) {
        saveInteractively()
    }

    @objc func saveDocumentAs(_ sender: Any?) {
        saveAsInteractively()
    }

    private func presentError(_ message: String, details: String) {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = message
        alert.informativeText = details
        alert.runModal()
    }
}

// MARK: - Window lifecycle

extension EditorWindowController: NSWindowDelegate {
    /// Standard dirty-document close flow: Save / Cancel / Don't Save.
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard coreDocument.isDirty else { return true }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Do you want to save the changes made to “\(sender.title)”?"
        alert.informativeText = "Your changes will be lost if you don’t save them."
        alert.addButton(withTitle: "Save")
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Don’t Save")
        switch alert.runModal() {
        case .alertFirstButtonReturn:
            return saveInteractively()
        case .alertThirdButtonReturn:
            return true
        default:
            return false
        }
    }
}

// MARK: - Text view synchronization

extension EditorWindowController: NSTextViewDelegate {
    func textView(
        _ textView: NSTextView,
        shouldChangeTextIn affectedCharRange: NSRange,
        replacementString: String?
    ) -> Bool {
        // A nil replacement is an attribute-only change; no text moves.
        guard let replacementString else { return true }
        do {
            try coreDocument.replace(utf16Range: affectedCharRange, with: replacementString)
            selectionChangeIsFromEditing = true
            return true
        } catch {
            // Core refused: refuse the view edit as well so neither side
            // moves. Rejections here indicate a sync bug worth surfacing.
            NSSound.beep()
            NSLog("edit rejected by core: \(error)")
            return false
        }
    }

    func textDidChange(_ notification: Notification) {
        updateChrome()
        assertInSync()
    }

    func textViewDidChangeSelection(_ notification: Notification) {
        // A caret move that is not part of an edit (click, arrow keys) ends
        // the current typing run for undo purposes.
        if selectionChangeIsFromEditing {
            selectionChangeIsFromEditing = false
        } else {
            coreDocument.breakUndoCoalescing()
        }
    }
}

// MARK: - Menu validation

extension EditorWindowController: NSMenuItemValidation {
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        switch menuItem.action {
        case #selector(performUndo(_:)):
            return coreDocument.canUndo
        case #selector(performRedo(_:)):
            return coreDocument.canRedo
        default:
            return true
        }
    }
}
