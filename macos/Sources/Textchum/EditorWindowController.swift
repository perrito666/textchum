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
    /// Watches the document's file for changes made by other programs.
    private var fileWatcher: DispatchSourceFileSystemObject?
    /// Our own atomic saves rename over the watched file; events until this
    /// instant are ours, not an external change.
    private var watcherSuppressedUntil = Date.distantPast
    /// Prevents stacking reload prompts when events arrive in bursts.
    private var isPresentingReloadPrompt = false
    /// Re-colors on system appearance changes (theme colors differ).
    private var appearanceObservation: NSKeyValueObservation?

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
        // Native find bar: find/replace UI, regex option, ⌘G navigation.
        // Replacements route through the standard delegate path, so they
        // synchronize with the core like any other edit.
        textView.usesFindBar = true
        textView.isIncrementalSearchingEnabled = true
        textView.delegate = self
        self.textView = textView

        textView.string = coreDocument.text
        window.contentView = scrollView
        if let settings {
            apply(settings: settings)
        }
        updateChrome()
        startWatchingFile()
        applyHighlights()
        appearanceObservation = NSApp.observe(\.effectiveAppearance) { [weak self] _, _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.applyHighlights() }
            }
        }
    }

    deinit {
        fileWatcher?.cancel()
    }

    // MARK: Syntax highlighting

    /// Documents beyond this size (UTF-16 units) are left uncolored until a
    /// viewport-scoped pass exists; the editor itself stays fast.
    private static let highlightSizeCap = 256 * 1024

    /// Paints the core's styled spans as TextKit 2 rendering attributes —
    /// a color-only overlay that never invalidates layout, so coloring is
    /// cheap and cannot disturb the edit pipeline. (Bold/italic style
    /// flags are ignored for now: font changes would invalidate layout.)
    private func applyHighlights() {
        guard let textView,
            let layoutManager = textView.textLayoutManager,
            let contentManager = layoutManager.textContentManager
        else { return }
        let documentRange = layoutManager.documentRange
        layoutManager.removeRenderingAttribute(.foregroundColor, for: documentRange)

        let length = coreDocument.lengthInUTF16
        guard length > 0, length <= Self.highlightSizeCap else { return }
        let spans = coreDocument.highlights(in: NSRange(location: 0, length: length))
        guard !spans.isEmpty else { return }

        let darkAppearance =
            (window?.effectiveAppearance ?? NSApp.effectiveAppearance)
                .bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        for span in spans {
            guard
                let color = HighlightPalette.color(
                    forStyle: span.styleIndex, darkAppearance: darkAppearance)
            else { continue }
            guard
                let start = contentManager.location(
                    documentRange.location, offsetBy: span.range.location),
                let end = contentManager.location(start, offsetBy: span.range.length),
                let range = NSTextRange(location: start, end: end)
            else { continue }
            // `set` replaces within the range, so later spans win — the
            // ordering contract the core's span list is built around.
            layoutManager.setRenderingAttributes([.foregroundColor: color], for: range)
        }
    }

    // MARK: External changes

    /// (Re)arms the file watcher on the document's current path. Called at
    /// init, after saves (each atomic save is a new inode), and from the
    /// event handler itself after rename/delete events leave the old
    /// source watching a dead inode.
    private func startWatchingFile() {
        fileWatcher?.cancel()
        fileWatcher = nil
        guard let path = coreDocument.path else { return }
        let descriptor = open(path, O_EVTONLY)
        guard descriptor >= 0 else { return }
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .extend, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            self?.fileDidChangeOnDisk()
        }
        source.setCancelHandler {
            _ = Darwin.close(descriptor)
        }
        source.resume()
        fileWatcher = source
    }

    private func fileDidChangeOnDisk() {
        startWatchingFile()
        guard Date() >= watcherSuppressedUntil else { return }
        guard let path = coreDocument.path, FileManager.default.fileExists(atPath: path) else {
            // Deleted or moved away: keep the buffer; a save will recreate
            // the file.
            updateChrome()
            return
        }
        if coreDocument.isDirty {
            guard !isPresentingReloadPrompt else { return }
            isPresentingReloadPrompt = true
            defer { isPresentingReloadPrompt = false }
            let alert = NSAlert()
            alert.alertStyle = .warning
            alert.messageText = "“\(window?.title ?? "Document")” changed on disk."
            alert.informativeText =
                "The file was modified by another program, and you have unsaved changes. "
                + "Reloading will discard your changes (one Undo brings them back)."
            alert.addButton(withTitle: "Keep My Changes")
            alert.addButton(withTitle: "Reload From Disk")
            if alert.runModal() == .alertSecondButtonReturn {
                reloadFromDisk()
            }
        } else {
            // Clean documents follow the disk silently.
            reloadFromDisk()
        }
    }

    private func reloadFromDisk() {
        let selection = textView?.selectedRange()
        do {
            guard let edit = try coreDocument.reload() else { return }
            replay([edit])
            // A whole-document replace should not fling the caret to the
            // end; keep the previous position, clamped.
            if let selection, let textView {
                let length = (textView.string as NSString).length
                let caret = NSRange(location: min(selection.location, length), length: 0)
                textView.setSelectedRange(caret)
                textView.scrollRangeToVisible(caret)
            }
        } catch {
            presentError("Could not reload the document.", details: "\(error)")
        }
        updateChrome()
    }

    /// Marks a window of time in which file events are our own save.
    private func noteOwnSave() {
        watcherSuppressedUntil = Date().addingTimeInterval(1.0)
        startWatchingFile()
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
        var subtitle = "\(coreDocument.encodingName) · \(coreDocument.lengthInBytes) bytes"
        if let language = coreDocument.languageName {
            subtitle += " · \(language)"
        }
        window.subtitle = subtitle
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
        replay(coreDocument.undo())
    }

    @objc func performRedo(_ sender: Any?) {
        replay(coreDocument.redo())
    }

    /// Applies core-reported edits to the display cache, in order. Storage
    /// mutations do not go through the text view delegate, so this cannot
    /// echo back into the core.
    private func replay(_ edits: [CoreDocument.AppliedEdit]) {
        guard !edits.isEmpty, let textView, let storage = textView.textStorage else { return }
        var caret = 0
        storage.beginEditing()
        for edit in edits {
            storage.replaceCharacters(in: edit.range, with: edit.replacement)
            let insertedLength = (edit.replacement as NSString).length
            if insertedLength > 0 {
                storage.setAttributes(
                    textView.typingAttributes,
                    range: NSRange(location: edit.range.location, length: insertedLength)
                )
            }
            caret = edit.range.location + insertedLength
        }
        storage.endEditing()

        // Put the caret at the end of the last replayed change, clamped in
        // case the document shrank, and reveal it.
        selectionChangeIsFromEditing = true
        caret = min(caret, (textView.string as NSString).length)
        textView.setSelectedRange(NSRange(location: caret, length: 0))
        textView.scrollRangeToVisible(NSRange(location: caret, length: 0))

        updateChrome()
        applyHighlights()
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
            noteOwnSave()
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
            noteOwnSave()
            updateChrome()
            // An untitled document may just have gained a language from
            // its new extension.
            applyHighlights()
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

    /// Multi-range variant, used by the find bar's Replace All (and any
    /// future multi-selection editing). All ranges arrive in the
    /// coordinates of the current text; the view applies them at once
    /// after we return true, while the core applies sequentially — so the
    /// core takes them back-to-front to keep earlier offsets valid, and
    /// records them as one undo step.
    func textView(
        _ textView: NSTextView,
        shouldChangeTextInRanges affectedRanges: [NSValue],
        replacementStrings: [String]?
    ) -> Bool {
        guard let replacementStrings else { return true }
        guard affectedRanges.count == replacementStrings.count else { return false }
        // Pre-validate so the group below cannot fail halfway through.
        let length = coreDocument.lengthInUTF16
        let pairs = zip(affectedRanges.map(\.rangeValue), replacementStrings)
        guard affectedRanges.allSatisfy({ NSMaxRange($0.rangeValue) <= length }) else {
            NSSound.beep()
            return false
        }
        coreDocument.beginEditGroup()
        do {
            for (range, replacement) in pairs.sorted(by: { $0.0.location > $1.0.location }) {
                try coreDocument.replace(utf16Range: range, with: replacement)
            }
            coreDocument.endEditGroup()
            selectionChangeIsFromEditing = true
            return true
        } catch {
            // Should be unreachable after pre-validation; restore the core
            // (the view never changed) and refuse the edit.
            coreDocument.endEditGroup()
            _ = coreDocument.undo()
            NSSound.beep()
            NSLog("multi-range edit rejected by core: \(error)")
            return false
        }
    }

    func textDidChange(_ notification: Notification) {
        updateChrome()
        applyHighlights()
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
