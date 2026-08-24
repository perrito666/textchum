import AppKit
import SwiftUI
import TextchumKit
import UniformTypeIdentifiers
import WebKit

/// Forwards script messages to a weak target, so the web view's user
/// content controller (which retains its handlers) cannot create a cycle.
private final class ScriptMessageProxy: NSObject, WKScriptMessageHandler {
    weak var target: EditorWindowController?

    func userContentController(
        _ controller: WKUserContentController, didReceive message: WKScriptMessage
    ) {
        target?.previewDidScroll(message: message)
    }
}

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
/// Everything a window needs to host its navigation drawer.
struct SidebarConfiguration {
    /// Shared explorer state, so expansion follows between tabs.
    let treeState: FileTreeState
    /// Settings-aware project-root resolution (workspace toggles apply).
    let resolveProjectRoot: (String) -> String?
    /// The configuration's current `workspace` section, for flags the
    /// editor resolves itself (the ctags fallback).
    var workspaceSettingsJSON: () -> String = { "{}" }
    let selectDocument: (ObjectIdentifier) -> Void
    let openFile: (String) -> Void
    /// Moves the given documents' windows into a window of their own…
    var splitGroup: ([ObjectIdentifier]) -> Void = { _ in }
    /// …or into the chosen target window (second argument) as tabs.
    var mergeGroup: ([ObjectIdentifier], ObjectIdentifier) -> Void = { _, _ in }
    /// Destinations for "Gather Into": one entry per tab group, the
    /// asking window's own group listed first as "This Window".
    var windowTargets: (ObjectIdentifier) -> [WindowTarget] = { _ in [] }
}

/// One "Gather Into" destination: a window standing for its tab group.
struct WindowTarget: Identifiable {
    /// The representative editor controller's identity.
    let id: ObjectIdentifier
    let title: String
}

/// Per-window observable state feeding the sidebar's folder tree.
@MainActor
final class WindowSidebarContext: ObservableObject {
    @Published var projectRoot: String?
}

final class EditorWindowController: NSWindowController {
    // Named to avoid NSWindowController's own `document` property.
    let coreDocument: CoreDocument
    /// This window's sidebar state (buffer list scoped to its tab group).
    let sidebarModel = SidebarModel()
    /// This document's project root (nearest root marker), cached and
    /// refreshed when the path changes.
    private(set) var projectRoot: String?
    private let sidebarContext = WindowSidebarContext()
    /// The (title, dirty, path) triple last published to the sidebar, to
    /// avoid rebuilding it on every keystroke.
    private var publishedState: (String, Bool, String?) = ("", false, nil)
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
    /// The core app handle, used for language-server notifications.
    private let lspApp: CoreApp?
    /// The path this window has announced as open to the server pool.
    private var lspOpenPath: String?
    /// Debounces didChange notifications while typing.
    private var lspChangeTimer: Timer?
    /// The latest language-server findings for this document.
    private var diagnostics: [CoreDiagnostic] = []
    /// The completion popup and its trigger machinery.
    private let completionPopup = CompletionPopup()
    private var completionTimer: Timer?
    /// The most recent replacement the user typed (single keystroke or
    /// paste), for completion auto-triggering.
    private var lastTypedText = ""
    /// Debounces hover requests while the mouse moves.
    private var hoverTimer: Timer?
    /// The popover currently showing hover content, if any.
    private var hoverPopover: NSPopover?
    /// The window's split view controller (sidebar · editor · preview).
    private var splitController: NSSplitViewController?
    /// The line-number gutter (a sibling of the scroll view, not a ruler).
    private var lineRuler: LineNumberGutterView?
    /// The Markdown preview pane, present while the preview is shown.
    private var previewItem: NSSplitViewItem?
    private var previewWebView: WKWebView?
    private var previewUpdateTimer: Timer?
    /// Suppresses scroll-sync echo: which side drove the last sync, when.
    private var lastScrollSync: (fromPreview: Bool, at: Date) = (false, .distantPast)

    /// Opens (or fronts) a file at a position — cross-file navigation,
    /// provided by the app.
    private let openLocation: ((String, Int, Int) -> Void)?
    /// Settings-aware project-root resolution from the app.
    private let resolveProjectRoot: (String) -> String?
    /// The configuration's live `workspace` section, for flag lookups.
    private let workspaceSettingsJSON: () -> String

    init(
        document: CoreDocument,
        settings: EditorSettings? = nil,
        sidebar: SidebarConfiguration? = nil,
        lspApp: CoreApp? = nil,
        openLocation: ((String, Int, Int) -> Void)? = nil
    ) {
        self.coreDocument = document
        self.lspApp = lspApp
        self.openLocation = openLocation
        self.resolveProjectRoot =
            sidebar?.resolveProjectRoot ?? { CoreWorkspace.projectRoot(forPath: $0) }
        self.workspaceSettingsJSON = sidebar?.workspaceSettingsJSON ?? { "{}" }

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

        // Gutter + scroll view side by side in one container.
        let gutter = LineNumberGutterView(textView: textView)
        gutter.setVisible(settings?.lineNumbers ?? true)
        self.lineRuler = gutter
        let editorContainer = NSView()
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        editorContainer.addSubview(gutter)
        editorContainer.addSubview(scrollView)
        NSLayoutConstraint.activate([
            gutter.leadingAnchor.constraint(equalTo: editorContainer.leadingAnchor),
            gutter.topAnchor.constraint(equalTo: editorContainer.topAnchor),
            gutter.bottomAnchor.constraint(equalTo: editorContainer.bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: gutter.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: editorContainer.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: editorContainer.bottomAnchor),
            scrollView.trailingAnchor.constraint(equalTo: editorContainer.trailingAnchor),
        ])
        // The gutter follows every scroll.
        let clipView = scrollView.contentView
        clipView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(editorDidScroll(_:)),
            name: NSView.boundsDidChangeNotification,
            object: clipView
        )

        // Mouse-move tracking feeds language-server hover.
        textView.addTrackingArea(
            NSTrackingArea(
                rect: .zero,
                options: [.mouseMoved, .activeInKeyWindow, .inVisibleRect],
                owner: self,
                userInfo: nil
            ))

        textView.string = coreDocument.text

        if let sidebar {
            // Sidebar + editor in a split view controller; the sidebar item
            // brings native collapse behavior and the toggleSidebar action.
            let editorController = NSViewController()
            editorController.view = editorContainer

            let splitController = NSSplitViewController()
            let sidebarView = SidebarView(
                model: sidebarModel,
                currentDocumentID: ObjectIdentifier(self),
                context: sidebarContext,
                treeState: sidebar.treeState,
                onSelectDocument: sidebar.selectDocument,
                onOpenFile: sidebar.openFile,
                onSplitGroup: { group in
                    sidebar.splitGroup(group.documents.map(\.id))
                },
                onMergeGroup: { group, target in
                    sidebar.mergeGroup(group.documents.map(\.id), target)
                },
                windowTargets: { [weak self] in
                    guard let self else { return [] }
                    return sidebar.windowTargets(ObjectIdentifier(self))
                }
            )
            let sidebarHost = NSHostingController(rootView: sidebarView)
            // Without this, the list inherits a phantom titlebar inset and
            // its first row starts scrolled out of view.
            sidebarHost.safeAreaRegions = []
            let sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebarHost)
            sidebarItem.minimumThickness = 180
            sidebarItem.maximumThickness = 400
            // Full-height layout slides the list under the title bar and
            // hides the first section header; keep the sidebar below it.
            sidebarItem.allowsFullHeightLayout = false
            splitController.addSplitViewItem(sidebarItem)
            splitController.addSplitViewItem(NSSplitViewItem(viewController: editorController))
            window.contentViewController = splitController
            window.setContentSize(NSSize(width: 920, height: 480))
            window.center()
            self.splitController = splitController
        } else {
            window.contentView = editorContainer
        }

        if let settings {
            apply(settings: settings)
        }
        completionPopup.onAccept = { [weak self] item in
            self?.accept(completion: item)
        }
        updateChrome()
        startWatchingFile()
        refreshDecorations()
        syncLSPOpenState()
        // Markdown documents open with the live preview beside them.
        if coreDocument.languageName == "markdown" {
            showPreview()
        }
        appearanceObservation = NSApp.observe(\.effectiveAppearance) { [weak self] _, _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.refreshDecorations() }
            }
        }
    }

    // MARK: Language servers

    /// Opens/closes this document with the server pool as its path and
    /// language come and go (open, save-as).
    private func syncLSPOpenState() {
        guard let lspApp else { return }
        let current: String? =
            (coreDocument.languageName != nil) ? coreDocument.path : nil
        guard current != lspOpenPath else { return }
        if let old = lspOpenPath {
            lspApp.lspDidClose(path: old)
        }
        if let new = current, let language = coreDocument.languageName {
            lspApp.lspDidOpen(path: new, language: language, text: coreDocument.text)
        }
        lspOpenPath = current
    }

    /// Re-announces the document after a pool restart, respawning its
    /// server under the current configuration.
    func reannounceLSP() {
        guard let lspApp, let path = lspOpenPath,
            let language = coreDocument.languageName
        else { return }
        lspApp.lspDidOpen(path: path, language: language, text: coreDocument.text)
    }

    /// Debounced full-text didChange, so servers see keystrokes in
    /// human-sized batches.
    private func scheduleLSPChange() {
        guard lspApp != nil, lspOpenPath != nil else { return }
        lspChangeTimer?.invalidate()
        lspChangeTimer = Timer.scheduledTimer(withTimeInterval: 0.3, repeats: false) {
            [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    guard let self, let path = self.lspOpenPath else { return }
                    self.lspApp?.lspDidChange(path: path, text: self.coreDocument.text)
                }
            }
        }
    }

    /// Called by the app when a server publishes findings for this path.
    func apply(diagnostics: [CoreDiagnostic]) {
        self.diagnostics = diagnostics
        renderDiagnostics()
        updateChrome()
    }

    // MARK: Hover

    /// Tracking-area callback: after the mouse rests for a beat, ask the
    /// server what is under it.
    override func mouseMoved(with event: NSEvent) {
        hoverPopover?.close()
        hoverPopover = nil
        guard lspApp != nil, lspOpenPath != nil, let textView else { return }
        let point = textView.convert(event.locationInWindow, from: nil)
        hoverTimer?.invalidate()
        hoverTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: false) {
            [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.requestHover(at: point) }
            }
        }
    }

    /// Character offset → LSP (line, UTF-16 column): walk line ranges
    /// until the one containing the offset.
    /// The caret as an LSP position, for the jump stack.
    var caretLSPPosition: (line: Int, character: Int) {
        guard let textView else { return (0, 0) }
        let text = textView.string as NSString
        let index = min(textView.selectedRange().location, text.length)
        return Self.lspPosition(ofIndex: index, in: text)
    }

    private static func lspPosition(ofIndex index: Int, in text: NSString) -> (Int, Int) {
        var line = 0
        var lineStart = 0
        var scan = 0
        while scan < text.length {
            let lineRange = text.lineRange(for: NSRange(location: scan, length: 0))
            if index < NSMaxRange(lineRange) {
                lineStart = lineRange.location
                break
            }
            scan = NSMaxRange(lineRange)
            line += 1
            lineStart = scan
        }
        return (line, index - lineStart)
    }

    private func requestHover(at point: NSPoint) {
        guard let lspApp, let path = lspOpenPath, let textView else { return }
        let text = textView.string as NSString
        let index = textView.characterIndexForInsertion(at: point)
        guard index >= 0, index <= text.length else { return }
        let (line, character) = Self.lspPosition(ofIndex: index, in: text)
        lspApp.lspHover(path: path, line: line, character: character) { [weak self] json in
            self?.showHover(resultJSON: json, at: point)
        }
    }

    // MARK: Go to definition

    /// Jumps to the definition of the symbol under the caret: the
    /// language server's answer, or the ctags index for projects that
    /// opted into the fallback (also consulted when the server has no
    /// answer).
    @objc func jumpToDefinition(_ sender: Any?) {
        guard let textView else { return }
        guard let lspApp, let path = lspOpenPath else {
            if !ctagsJump() { NSSound.beep() }
            return
        }
        let text = textView.string as NSString
        let index = min(textView.selectedRange().location, text.length)
        let (line, character) = Self.lspPosition(ofIndex: index, in: text)
        lspApp.lspDefinition(path: path, line: line, character: character) { [weak self] json in
            guard let self else { return }
            if let location = Self.firstLocation(fromResultJSON: json) {
                self.openLocation?(location.path, location.line, location.character)
            } else if !self.ctagsJump() {
                NSSound.beep()
            }
        }
    }

    // MARK: References, rename, formatting

    /// Lists every reference to the symbol under the caret.
    @objc func findReferences(_ sender: Any?) {
        guard let lspApp, let path = lspOpenPath, let textView else { return }
        let text = textView.string as NSString
        let index = min(textView.selectedRange().location, text.length)
        let (line, character) = Self.lspPosition(ofIndex: index, in: text)
        lspApp.lspReferences(path: path, line: line, character: character) { [weak self] json in
            guard let self else { return }
            let locations = Self.referenceLocations(fromResultJSON: json)
            guard !locations.isEmpty else {
                NSSound.beep()
                return
            }
            ReferencesPanel.shared.show(locations: locations, over: self.window) {
                [weak self] location in
                self?.openLocation?(location.path, location.line, location.character)
            }
        }
    }

    private static func referenceLocations(
        fromResultJSON json: String
    ) -> [ReferencesPanel.Location] {
        guard let data = json.data(using: .utf8),
            let array = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]]
        else { return [] }
        return array.compactMap { raw in
            guard
                let uri = raw["uri"] as? String,
                let path = LSPEdits.path(fromURI: uri),
                let range = raw["range"] as? [String: Any],
                let start = range["start"] as? [String: Any],
                let line = start["line"] as? Int,
                let character = start["character"] as? Int
            else { return nil }
            return ReferencesPanel.Location(path: path, line: line, character: character)
        }
        .sorted { ($0.path, $0.line) < ($1.path, $1.line) }
    }

    /// Renames the symbol under the caret everywhere the server knows
    /// about — open windows edit in place, closed files are rewritten on
    /// disk.
    @objc func renameSymbol(_ sender: Any?) {
        guard let lspApp, let path = lspOpenPath, let textView else { return }
        let current = symbolUnderCaret() ?? ""
        let alert = NSAlert()
        alert.messageText = "Rename Symbol"
        alert.informativeText =
            current.isEmpty ? "New name:" : "New name for “\(current)”:"
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.stringValue = current
        alert.accessoryView = field
        alert.addButton(withTitle: "Rename")
        alert.addButton(withTitle: "Cancel")
        alert.window.initialFirstResponder = field
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let newName = field.stringValue.trimmingCharacters(in: .whitespaces)
        guard !newName.isEmpty, newName != current else { return }
        let text = textView.string as NSString
        let index = min(textView.selectedRange().location, text.length)
        let (line, character) = Self.lspPosition(ofIndex: index, in: text)
        lspApp.lspRename(path: path, line: line, character: character, newName: newName) {
            json in
            let applied =
                (NSApp.delegate as? AppDelegate)?.applyWorkspaceEdit(resultJSON: json)
                ?? false
            if !applied {
                NSSound.beep()
            }
        }
    }

    /// Reformats the whole document through its language server.
    @objc func formatDocument(_ sender: Any?) {
        guard let lspApp, let path = lspOpenPath, let textView else { return }
        // Respect what the document already does: a tab-indented file
        // keeps tabs, everything else formats with spaces.
        let usesTabs =
            textView.string.contains("\n\t") || textView.string.hasPrefix("\t")
        lspApp.lspFormatting(path: path, tabSize: appliedTabWidth, insertSpaces: !usesTabs) {
            [weak self] json in
            guard let self else { return }
            let edits = LSPEdits.textEdits(fromResultJSON: json)
            guard !edits.isEmpty else {
                NSSound.beep()
                return
            }
            self.apply(textEdits: edits)
        }
    }

    /// Applies LSP edits to this window's document through the normal
    /// text-view path, so the core stays synchronized and undo works —
    /// bottom-up, so earlier ranges never shift.
    func apply(textEdits: [LSPEdits.TextEdit]) {
        guard let textView else { return }
        for edit in LSPEdits.bottomUp(textEdits) {
            let range = LSPEdits.nsRange(of: edit, in: textView.string as NSString)
            textView.insertText(edit.newText, replacementRange: range)
        }
    }

    /// View → Document Outline (⇧⌘O): the file's symbols from its
    /// server, fuzzy-filterable; selecting one jumps (via the jump
    /// stack, so Go Back returns here).
    @objc func showDocumentOutline(_ sender: Any?) {
        guard let lspApp, let path = lspOpenPath else { return }
        lspApp.lspDocumentSymbols(path: path) { [weak self] json in
            guard let self else { return }
            let symbols = OutlinePanel.symbols(fromResultJSON: json)
            guard !symbols.isEmpty else {
                NSSound.beep()
                return
            }
            OutlinePanel.shared.show(symbols: symbols, over: self.window) {
                [weak self] symbol in
                guard let self, let path = self.coreDocument.path else { return }
                self.openLocation?(path, symbol.line, symbol.character)
            }
        }
    }

    /// Whether this document's project opted into the ctags fallback.
    var ctagsFallbackEnabled: Bool {
        guard let projectRoot else { return false }
        return CoreWorkspace.flag(
            "ctags_fallback", root: projectRoot, settingsJSON: workspaceSettingsJSON())
    }

    /// Jump via the ctags index; false when disabled, or nothing matched.
    @discardableResult
    private func ctagsJump() -> Bool {
        guard ctagsFallbackEnabled, let projectRoot,
            let name = symbolUnderCaret(),
            let target = CtagsIndex.shared.definition(of: name, in: projectRoot)
        else { return false }
        openLocation?(target.path, target.line, 0)
        return true
    }

    /// The identifier around the caret (letters, digits, underscore).
    private func symbolUnderCaret() -> String? {
        guard let textView else { return nil }
        let text = textView.string as NSString
        var identifier = CharacterSet.alphanumerics
        identifier.insert("_")
        let isWord: (Int) -> Bool = { index in
            guard index >= 0, index < text.length,
                let scalar = UnicodeScalar(text.character(at: index))
            else { return false }
            return identifier.contains(scalar)
        }
        var start = min(textView.selectedRange().location, text.length)
        // A caret just past the last character of a word still means it.
        if !isWord(start), isWord(start - 1) { start -= 1 }
        guard isWord(start) else { return nil }
        var end = start + 1
        while isWord(start - 1) { start -= 1 }
        while isWord(end) { end += 1 }
        return text.substring(with: NSRange(location: start, length: end - start))
    }

    /// Extracts the first target from an LSP definition result: a
    /// `Location`, `Location[]`, or `LocationLink[]`.
    private static func firstLocation(
        fromResultJSON json: String
    ) -> (path: String, line: Int, character: Int)? {
        guard let data = json.data(using: .utf8),
            let parsed = try? JSONSerialization.jsonObject(with: data)
        else { return nil }
        let candidate: [String: Any]?
        if let array = parsed as? [[String: Any]] {
            candidate = array.first
        } else {
            candidate = parsed as? [String: Any]
        }
        guard let candidate else { return nil }
        // Location uses uri/range; LocationLink uses targetUri and
        // targetSelectionRange (preferred) or targetRange.
        let uri = (candidate["uri"] ?? candidate["targetUri"]) as? String
        let range =
            (candidate["range"] ?? candidate["targetSelectionRange"]
                ?? candidate["targetRange"]) as? [String: Any]
        guard let uri, uri.hasPrefix("file://"),
            let start = range?["start"] as? [String: Any],
            let line = start["line"] as? Int,
            let character = start["character"] as? Int,
            let url = URL(string: uri)
        else { return nil }
        return (url.path, line, character)
    }

    // MARK: Session position

    /// The caret (UTF-16 offset) and vertical scroll, for session saving.
    var sessionPosition: (caret: Int, scroll: Double) {
        let caret = textView?.selectedRange().location ?? 0
        let scroll = Double(textView?.enclosingScrollView?.contentView.bounds.origin.y ?? 0)
        return (caret, scroll)
    }

    /// Restores a saved caret and scroll position (clamped to the text).
    func restoreSessionPosition(caret: Int, scroll: Double) {
        guard let textView else { return }
        let length = (textView.string as NSString).length
        selectionChangeIsFromEditing = false
        textView.setSelectedRange(NSRange(location: min(caret, length), length: 0))
        if let scrollView = textView.enclosingScrollView {
            scrollView.contentView.scroll(to: NSPoint(x: 0, y: max(0, scroll)))
            scrollView.reflectScrolledClipView(scrollView.contentView)
        } else {
            textView.scrollRangeToVisible(textView.selectedRange())
        }
    }

    /// Moves the caret to an LSP position and reveals it.
    func reveal(line: Int, character: Int) {
        guard let textView else { return }
        let text = textView.string as NSString
        var index = 0
        var currentLine = 0
        while currentLine < line && index < text.length {
            index = NSMaxRange(text.lineRange(for: NSRange(location: index, length: 0)))
            currentLine += 1
        }
        let target = min(index + max(0, character), text.length)
        selectionChangeIsFromEditing = false
        textView.setSelectedRange(NSRange(location: target, length: 0))
        textView.scrollRangeToVisible(NSRange(location: target, length: 0))
        window?.makeFirstResponder(textView)
    }

    /// Extracts human-readable text from an LSP hover result: contents as
    /// MarkupContent, a bare string, or an array of either.
    private static func hoverText(fromResultJSON json: String) -> String? {
        guard let data = json.data(using: .utf8),
            let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let contents = result["contents"]
        else { return nil }
        func text(from value: Any) -> String? {
            if let string = value as? String { return string }
            if let dict = value as? [String: Any] { return dict["value"] as? String }
            if let array = value as? [Any] {
                let parts = array.compactMap(text(from:))
                return parts.isEmpty ? nil : parts.joined(separator: "\n\n")
            }
            return nil
        }
        let extracted = text(from: contents)?.trimmingCharacters(in: .whitespacesAndNewlines)
        return (extracted?.isEmpty ?? true) ? nil : extracted
    }

    private func showHover(resultJSON: String, at point: NSPoint) {
        guard let textView, let content = Self.hoverText(fromResultJSON: resultJSON) else {
            return
        }
        hoverPopover?.close()

        let label = NSTextField(wrappingLabelWithString: content)
        label.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        label.preferredMaxLayoutWidth = 480
        let controller = NSViewController()
        let container = NSView()
        container.addSubview(label)
        label.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            label.topAnchor.constraint(equalTo: container.topAnchor, constant: 10),
            label.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -10),
            label.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 12),
            label.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -12),
            container.widthAnchor.constraint(lessThanOrEqualToConstant: 520),
        ])
        controller.view = container

        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = controller
        popover.show(
            relativeTo: NSRect(origin: point, size: NSSize(width: 1, height: 1)),
            of: textView,
            preferredEdge: .maxY
        )
        hoverPopover = popover
    }

    deinit {
        fileWatcher?.cancel()
    }

    // MARK: Markdown preview

    private static let previewTemplate = """
        <!DOCTYPE html><html><head><meta charset="utf-8">
        <style>
        :root { color-scheme: light dark; }
        body { font: 15px/1.6 -apple-system, sans-serif; margin: 0;
               padding: 1.5em 2em; background: transparent; }
        @media (prefers-color-scheme: light) { body { color: #24292e; } }
        @media (prefers-color-scheme: dark)  { body { color: #dfdfe0; } }
        h1, h2 { border-bottom: 1px solid rgba(128,128,128,.3);
                 padding-bottom: .3em; }
        code { font-family: ui-monospace, monospace; font-size: .9em;
               background: rgba(128,128,128,.15); border-radius: 4px;
               padding: .1em .35em; }
        pre { background: rgba(128,128,128,.12); border-radius: 6px;
              padding: .8em 1em; overflow-x: auto; }
        pre code { background: none; padding: 0; }
        blockquote { border-left: 4px solid rgba(128,128,128,.4);
                     margin-left: 0; padding-left: 1em; opacity: .85; }
        table { border-collapse: collapse; }
        th, td { border: 1px solid rgba(128,128,128,.4);
                 padding: .3em .7em; }
        img { max-width: 100%; }
        a { color: #0b60a0; } @media (prefers-color-scheme: dark) { a { color: #6bdfff; } }
        </style></head><body><div id="content"></div>
        <script>
        function setContent(html) { document.getElementById("content").innerHTML = html; }
        function scrollToFraction(f) {
          const max = document.body.scrollHeight - window.innerHeight;
          window.__syncing = true;
          window.scrollTo(0, Math.max(0, f * max));
          setTimeout(() => { window.__syncing = false; }, 80);
        }
        addEventListener("scroll", () => {
          if (window.__syncing) { return; }
          const max = document.body.scrollHeight - window.innerHeight;
          const f = max > 0 ? window.scrollY / max : 0;
          webkit.messageHandlers.scrolled.postMessage(f);
        }, { passive: true });
        </script></body></html>
        """

    /// Shows or hides the preview pane (markdown documents only).
    @objc func togglePreview(_ sender: Any?) {
        if previewItem != nil {
            hidePreview()
        } else {
            showPreview()
        }
    }

    private func showPreview() {
        guard previewItem == nil, let splitController,
            coreDocument.languageName == "markdown"
        else { return }

        let proxy = ScriptMessageProxy()
        proxy.target = self
        let configuration = WKWebViewConfiguration()
        configuration.userContentController.add(proxy, name: "scrolled")
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.setValue(false, forKey: "drawsBackground")
        webView.navigationDelegate = self
        webView.loadHTMLString(Self.previewTemplate, baseURL: nil)

        let controller = NSViewController()
        controller.view = webView
        let item = NSSplitViewItem(viewController: controller)
        item.minimumThickness = 240
        // The editor must never be squeezed out: it keeps its space
        // (higher holding priority, real minimum); the preview yields.
        item.holdingPriority = NSLayoutConstraint.Priority(240)
        if splitController.splitViewItems.count > 1 {
            let editorItem = splitController.splitViewItems[1]
            editorItem.minimumThickness = 340
            editorItem.holdingPriority = NSLayoutConstraint.Priority(260)
        }
        splitController.addSplitViewItem(item)
        previewItem = item
        previewWebView = webView
        // Editor scrolling drives the preview via the scroll observer
        // registered at init (shared with the line-number gutter).

        // Deferred: resizing during window setup gets overridden by the
        // content controller's initial layout pass.
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                guard let window = self?.window, window.frame.width < 1200 else { return }
                window.setContentSize(
                    NSSize(width: 1360, height: max(window.frame.height, 540)))
                window.center()
            }
        }
    }

    private func hidePreview() {
        guard let previewItem, let splitController else { return }
        splitController.removeSplitViewItem(previewItem)
        self.previewItem = nil
        previewWebView = nil
    }

    /// Pushes the rendered document into the page (no reload: the DOM is
    /// patched, so the preview never flickers and keeps its scroll).
    private func updatePreview() {
        guard let previewWebView, let html = coreDocument.markdownHTML,
            let encoded = try? JSONEncoder().encode(html),
            let literal = String(data: encoded, encoding: .utf8)
        else { return }
        previewWebView.evaluateJavaScript("setContent(\(literal))")
    }

    /// Debounced preview refresh, called from every text-changing path.
    private func schedulePreviewUpdate() {
        guard previewItem != nil else { return }
        previewUpdateTimer?.invalidate()
        previewUpdateTimer = Timer.scheduledTimer(withTimeInterval: 0.15, repeats: false) {
            [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.updatePreview() }
            }
        }
    }

    @objc private func editorDidScroll(_ notification: Notification) {
        lineRuler?.needsDisplay = true
        completionPopup.dismiss()
        guard let previewWebView, let scrollView = textView?.enclosingScrollView else { return }
        // Ignore echoes of a preview-driven sync.
        if lastScrollSync.fromPreview, Date().timeIntervalSince(lastScrollSync.at) < 0.15 {
            return
        }
        let visible = scrollView.contentView.bounds
        let total = scrollView.documentView?.frame.height ?? 0
        let maximum = max(total - visible.height, 1)
        let fraction = max(0, min(1, visible.origin.y / maximum))
        lastScrollSync = (false, Date())
        previewWebView.evaluateJavaScript("scrollToFraction(\(fraction))")
    }

    /// The preview scrolled (user-driven); mirror it in the editor.
    func previewDidScroll(message: WKScriptMessage) {
        guard let fraction = message.body as? Double,
            let scrollView = textView?.enclosingScrollView
        else { return }
        if !lastScrollSync.fromPreview, Date().timeIntervalSince(lastScrollSync.at) < 0.15 {
            return
        }
        let visible = scrollView.contentView.bounds
        let total = scrollView.documentView?.frame.height ?? 0
        let target = max(0, (total - visible.height) * fraction)
        lastScrollSync = (true, Date())
        scrollView.contentView.scroll(to: NSPoint(x: 0, y: target))
        scrollView.reflectScrolledClipView(scrollView.contentView)
    }

    // MARK: Decorations (syntax colors + diagnostic underlines)

    /// Rendering attributes are one overlay: highlight spans *set*
    /// attributes (replacing anything in their range), so diagnostics must
    /// re-add their underlines afterwards. Internal because theme
    /// switches recolor every window from the app delegate.
    func refreshDecorations() {
        applyHighlights()
        renderDiagnostics()
    }

    /// Marks each finding with a tinted background: red for errors,
    /// orange for warnings, blue otherwise.
    private func renderDiagnostics() {
        guard let textView, let layoutManager = textView.textLayoutManager,
            let contentManager = layoutManager.textContentManager
        else { return }
        let documentRange = layoutManager.documentRange
        layoutManager.removeRenderingAttribute(.underlineStyle, for: documentRange)
        layoutManager.removeRenderingAttribute(.underlineColor, for: documentRange)
        layoutManager.removeRenderingAttribute(.backgroundColor, for: documentRange)
        guard !diagnostics.isEmpty else { return }

        let text = textView.string as NSString
        for diagnostic in diagnostics {
            guard let range = nsRange(of: diagnostic, in: text) else { continue }
            guard
                let start = contentManager.location(
                    documentRange.location, offsetBy: range.location),
                let end = contentManager.location(start, offsetBy: range.length),
                let textRange = NSTextRange(location: start, end: end)
            else { continue }
            let color: NSColor =
                switch diagnostic.severity {
                case 1: .systemRed
                case 2: .systemOrange
                default: .systemBlue
                }
            // The background tint is the marker TextKit 2 actually renders
            // from this layer; the underline attributes ride along for the
            // day rendering attributes honor them.
            layoutManager.addRenderingAttribute(
                .underlineStyle, value: NSUnderlineStyle.thick.rawValue, for: textRange)
            layoutManager.addRenderingAttribute(.underlineColor, value: color, for: textRange)
            layoutManager.addRenderingAttribute(
                .backgroundColor, value: color.withAlphaComponent(0.15), for: textRange)
        }
    }

    /// Converts an LSP (line, UTF-16 column) range to an `NSRange`,
    /// clamped to the current text — diagnostics can be a beat behind the
    /// buffer, and a stale position must never crash the overlay.
    private func nsRange(of diagnostic: CoreDiagnostic, in text: NSString) -> NSRange? {
        func offset(line: Int, column: Int) -> Int {
            var index = 0
            var currentLine = 0
            while currentLine < line && index < text.length {
                index = NSMaxRange(text.lineRange(for: NSRange(location: index, length: 0)))
                currentLine += 1
            }
            return min(index + max(column, 0), text.length)
        }
        let start = offset(line: diagnostic.line, column: diagnostic.character)
        let end = offset(line: diagnostic.endLine, column: diagnostic.endCharacter)
        guard end >= start else { return nil }
        // A zero-length finding still deserves a visible mark.
        let length = max(end - start, 1)
        guard start < text.length || text.length == 0 else { return nil }
        return NSRange(location: start, length: min(length, text.length - start))
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

    /// View → Redraw (⌥⌘L): rebuilds every visual layer from scratch —
    /// base text attributes, syntax colors, diagnostic marks, the
    /// gutter — for when a rendering artifact survives an edit.
    @objc func redrawDocument(_ sender: Any?) {
        guard let textView else { return }
        if let storage = textView.textStorage {
            storage.setAttributes(
                textView.typingAttributes,
                range: NSRange(location: 0, length: storage.length)
            )
        }
        refreshDecorations()
        lineRuler?.invalidateLineStarts()
        textView.needsDisplay = true
        textView.enclosingScrollView?.needsDisplay = true
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

    /// The configured tab width, remembered for formatting requests.
    private var appliedTabWidth = 4

    /// Applies configuration-derived settings to the view: the font, and
    /// tab stops sized to the configured width in that font.
    func apply(settings: EditorSettings) {
        appliedTabWidth = settings.tabWidth
        guard let textView else { return }
        let paragraphStyle = NSMutableParagraphStyle()
        let spaceWidth = (" " as NSString).size(withAttributes: [.font: settings.font]).width
        paragraphStyle.tabStops = []
        paragraphStyle.defaultTabInterval = spaceWidth * CGFloat(settings.tabWidth)

        lineRuler?.setVisible(settings.lineNumbers)
        lineRuler?.invalidateLineStarts()
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

    /// Title shown when the bare filename collides with another open
    /// document's — enough trailing path to tell them apart. Nil means
    /// the name is unique and shows as-is.
    private var displayTitle: String?

    /// Called by the sidebar rebuild, which sees every open document and
    /// therefore knows which names collide.
    func setDisplayTitle(_ title: String?) {
        guard displayTitle != title else { return }
        displayTitle = title
        guard let window, let path = coreDocument.path else { return }
        window.title = displayTitle ?? URL(fileURLWithPath: path).lastPathComponent
    }

    /// Refreshes everything the window shows about the document: title,
    /// edited marker, represented file, and the encoding/size subtitle.
    private func updateChrome() {
        guard let window else { return }
        if let path = coreDocument.path {
            window.representedURL = URL(fileURLWithPath: path)
            window.title = displayTitle ?? URL(fileURLWithPath: path).lastPathComponent
        } else {
            window.representedURL = nil
            window.title = "Untitled"
        }
        var subtitle = "\(coreDocument.encodingName) · \(coreDocument.lengthInBytes) bytes"
        if let language = coreDocument.languageName {
            subtitle += " · \(language)"
        }
        if !diagnostics.isEmpty {
            let errors = diagnostics.filter { $0.severity == 1 }.count
            let others = diagnostics.count - errors
            var parts: [String] = []
            if errors > 0 { parts.append("\(errors) error\(errors == 1 ? "" : "s")") }
            if others > 0 { parts.append("\(others) warning\(others == 1 ? "" : "s")") }
            subtitle += " · " + parts.joined(separator: ", ")
        }
        window.subtitle = subtitle
        window.isDocumentEdited = coreDocument.isDirty
        publishSidebarState()
    }

    /// Publishes title/dirty/path changes to the sidebar — but only actual
    /// changes, so per-keystroke chrome updates stay cheap.
    private func publishSidebarState() {
        let state = (window?.title ?? "Untitled", coreDocument.isDirty, coreDocument.path)
        guard state != publishedState else { return }
        if state.2 != publishedState.2 {
            projectRoot = state.2.flatMap(resolveProjectRoot)
            sidebarContext.projectRoot = projectRoot
        }
        publishedState = state
        NotificationCenter.default.post(name: .textchumDocumentsChanged, object: self)
    }

    /// Recomputes the project root under the current workspace settings
    /// (called when those settings change).
    func refreshProjectRoot() {
        projectRoot = coreDocument.path.flatMap(resolveProjectRoot)
        sidebarContext.projectRoot = projectRoot
        NotificationCenter.default.post(name: .textchumDocumentsChanged, object: self)
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
        refreshDecorations()
        scheduleLSPChange()
        schedulePreviewUpdate()
        lineRuler?.invalidateLineStarts()
        assertInSync()
    }

    // MARK: Completion

    /// The identifier prefix ending at the caret, if any.
    private func currentWordPrefix() -> (text: String, range: NSRange)? {
        guard let textView else { return nil }
        let text = textView.string as NSString
        let caret = textView.selectedRange().location
        var start = caret
        while start > 0 {
            let ch = Character(UnicodeScalar(text.character(at: start - 1)) ?? " ")
            if ch.isLetter || ch.isNumber || ch == "_" {
                start -= 1
            } else {
                break
            }
        }
        guard start < caret else { return nil }
        let range = NSRange(location: start, length: caret - start)
        return (text.substring(with: range), range)
    }

    /// Manual trigger (⌃Space) and the debug hook's entry point.
    @objc func triggerCompletion(_ sender: Any?) {
        requestCompletion()
    }

    private func scheduleCompletionRequest() {
        completionTimer?.invalidate()
        completionTimer = Timer.scheduledTimer(withTimeInterval: 0.12, repeats: false) {
            [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.requestCompletion() }
            }
        }
    }

    private func requestCompletion() {
        guard let lspApp, let path = lspOpenPath, let textView, let window else { return }
        let caret = textView.selectedRange().location
        let (line, character) = Self.lspPosition(
            ofIndex: caret, in: textView.string as NSString)
        lspApp.lspCompletion(path: path, line: line, character: character) {
            [weak self] json in
            guard let self, let textView = self.textView else { return }
            // Stale if the caret moved lines since the request.
            guard textView.selectedRange().location >= caret - 1 else { return }
            let items = CompletionPopup.parse(resultJSON: json)
            guard !items.isEmpty else {
                self.completionPopup.dismiss()
                return
            }
            let caretRect = textView.firstRect(
                forCharacterRange: NSRange(location: textView.selectedRange().location, length: 0),
                actualRange: nil
            )
            self.completionPopup.show(
                items: items,
                prefix: self.currentWordPrefix()?.text ?? "",
                below: caretRect,
                parent: window
            )
        }
    }

    /// Applies an accepted suggestion by replacing the word prefix — via
    /// `insertText`, so the edit flows through the normal synchronized
    /// path (delegate → core → history).
    private func accept(completion item: CompletionPopup.Item) {
        guard let textView else { return }
        let replacementRange =
            currentWordPrefix()?.range
            ?? NSRange(location: textView.selectedRange().location, length: 0)
        textView.insertText(item.insertText, replacementRange: replacementRange)
    }

    /// Auto-trigger after identifier characters and member access.
    private func completionAfterTyping() {
        if completionPopup.isVisible {
            if let prefix = currentWordPrefix() {
                completionPopup.filter(prefix: prefix.text)
            } else if lastTypedText != "." {
                completionPopup.dismiss()
            } else {
                scheduleCompletionRequest()
            }
            return
        }
        guard lspOpenPath != nil, lastTypedText.count == 1,
            let ch = lastTypedText.first,
            ch.isLetter || ch == "_" || ch == "."
        else { return }
        scheduleCompletionRequest()
    }

    // MARK: Block navigation

    /// Moves the caret to a UTF-16 offset and reveals it.
    private func moveCaret(to offset: Int) {
        guard let textView else { return }
        let clamped = min(max(0, offset), (textView.string as NSString).length)
        textView.setSelectedRange(NSRange(location: clamped, length: 0))
        textView.scrollRangeToVisible(NSRange(location: clamped, length: 0))
    }

    @objc func goToBlockStart(_ sender: Any?) {
        guard let textView,
            let block = coreDocument.blockBounds(at: textView.selectedRange().location)
        else {
            NSSound.beep()
            return
        }
        moveCaret(to: block.location)
    }

    @objc func goToBlockEnd(_ sender: Any?) {
        guard let textView,
            let block = coreDocument.blockBounds(at: textView.selectedRange().location)
        else {
            NSSound.beep()
            return
        }
        moveCaret(to: NSMaxRange(block))
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
        // The bare filename, not the window title — a disambiguated
        // title carries path components no filename should.
        panel.nameFieldStringValue =
            coreDocument.path.map { ($0 as NSString).lastPathComponent } ?? "Untitled.txt"
        guard panel.runModal() == .OK, let url = panel.url else { return false }
        do {
            try coreDocument.save(to: url.path)
            noteOwnSave()
            updateChrome()
            // An untitled document may just have gained a language from
            // its new extension — recolor, announce it to the pool, and
            // open the preview if it became markdown.
            refreshDecorations()
            syncLSPOpenState()
            if coreDocument.languageName == "markdown", previewItem == nil {
                showPreview()
            }
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

// MARK: - Preview navigation

extension EditorWindowController: WKNavigationDelegate {
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        // The template page is ready; push the first render.
        updatePreview()
    }
}

// MARK: - Window lifecycle

extension EditorWindowController: NSWindowDelegate {
    func windowWillClose(_ notification: Notification) {
        completionPopup.dismiss()
        completionTimer?.invalidate()
        lspChangeTimer?.invalidate()
        if let path = lspOpenPath {
            lspApp?.lspDidClose(path: path)
            lspOpenPath = nil
        }
    }

    func windowDidBecomeKey(_ notification: Notification) {
        // Tab membership may have changed (drags, merges); the per-window
        // buffer lists rebuild from it.
        NotificationCenter.default.post(name: .textchumDocumentsChanged, object: self)
    }

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
            lastTypedText = replacementString
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
        refreshDecorations()
        scheduleLSPChange()
        schedulePreviewUpdate()
        lineRuler?.invalidateLineStarts()
        completionAfterTyping()
        assertInSync()
    }

    /// Keyboard routing while the completion popup is visible: arrows
    /// navigate it, return/tab accept, escape dismisses — everything else
    /// keeps flowing to the editor. With the popup away, return picks up
    /// the auto-indent path.
    func textView(_ textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard completionPopup.isVisible else {
            if commandSelector == #selector(NSResponder.insertNewline(_:)) {
                return insertNewlineAutoIndenting(in: textView)
            }
            return false
        }
        switch commandSelector {
        case #selector(NSResponder.moveDown(_:)):
            completionPopup.moveSelection(by: 1)
            return true
        case #selector(NSResponder.moveUp(_:)):
            completionPopup.moveSelection(by: -1)
            return true
        case #selector(NSResponder.insertNewline(_:)), #selector(NSResponder.insertTab(_:)):
            completionPopup.acceptSelection()
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            completionPopup.dismiss()
            return true
        default:
            return false
        }
    }

    /// Return carries the current line's leading whitespace onto the new
    /// one, plus one more level after an opener; a line with nothing to
    /// inherit falls through to the plain newline.
    private func insertNewlineAutoIndenting(in textView: NSTextView) -> Bool {
        let selection = textView.selectedRange()
        guard
            let insertion = Self.autoIndentedNewline(
                in: textView.string as NSString,
                selection: selection,
                tabWidth: appliedTabWidth)
        else { return false }
        textView.insertText(insertion, replacementRange: selection)
        return true
    }

    /// The text return should insert at `selection`: a newline carrying
    /// the current line's leading whitespace, one level deeper when the
    /// last non-blank character before the caret is an opener (`{ [ (`
    /// or `:`) — tabs if the document indents with tabs, spaces at
    /// `tabWidth` otherwise. Nil when a plain newline will do.
    static func autoIndentedNewline(
        in text: NSString, selection: NSRange, tabWidth: Int
    ) -> String? {
        let caret = min(selection.location, text.length)
        let lineRange = text.lineRange(for: NSRange(location: caret, length: 0))

        var indent = ""
        var index = lineRange.location
        while index < caret {
            let character = text.character(at: index)
            guard character == 0x20 /* space */ || character == 0x09 /* tab */ else {
                break
            }
            indent.append(character == 0x09 ? "\t" : " ")
            index += 1
        }
        // The last non-blank character before the caret decides whether
        // the new line goes one level deeper.
        var opener: unichar = 0
        var scan = index
        while scan < caret {
            let character = text.character(at: scan)
            if character != 0x20 && character != 0x09 {
                opener = character
            }
            scan += 1
        }
        let deepens: Bool
        switch opener {
        case 0x7B /* { */, 0x5B /* [ */, 0x28 /* ( */, 0x3A /* : */:
            deepens = true
        default:
            deepens = false
        }
        guard !indent.isEmpty || deepens else { return nil }
        if deepens {
            let usesTabs = indent.contains("\t") || text.contains("\n\t")
            indent += usesTabs ? "\t" : String(repeating: " ", count: max(1, tabWidth))
        }
        return "\n" + indent
    }

    func textViewDidChangeSelection(_ notification: Notification) {
        // A caret move that is not part of an edit (click, arrow keys) ends
        // the current typing run for undo purposes.
        if selectionChangeIsFromEditing {
            selectionChangeIsFromEditing = false
        } else {
            coreDocument.breakUndoCoalescing()
            completionPopup.dismiss()
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
        case #selector(jumpToDefinition(_:)):
            return lspOpenPath != nil || ctagsFallbackEnabled
        case #selector(findReferences(_:)), #selector(renameSymbol(_:)),
            #selector(formatDocument(_:)), #selector(showDocumentOutline(_:)):
            return lspOpenPath != nil
        case #selector(goToBlockStart(_:)), #selector(goToBlockEnd(_:)):
            return coreDocument.languageName != nil
        case #selector(togglePreview(_:)):
            menuItem.state = previewItem != nil ? .on : .off
            return coreDocument.languageName == "markdown"
        case #selector(copyFileName(_:)), #selector(copyRelativePath(_:)),
            #selector(copyAbsolutePath(_:)):
            return coreDocument.path != nil
        case #selector(copyForgeURL(_:)):
            return coreDocument.path.map(PathActions.isInGitRepository) ?? false
        default:
            return true
        }
    }
}

// MARK: - Copy path actions (File → Copy Path, acting on the front tab)

extension EditorWindowController {
    @objc func copyFileName(_ sender: Any?) {
        guard let path = coreDocument.path else { return }
        PathActions.copy((path as NSString).lastPathComponent)
    }

    @objc func copyRelativePath(_ sender: Any?) {
        guard let path = coreDocument.path else { return }
        PathActions.copy(PathActions.relativePath(path, projectRoot: projectRoot))
    }

    @objc func copyAbsolutePath(_ sender: Any?) {
        guard let path = coreDocument.path else { return }
        PathActions.copy(path)
    }

    @objc func copyForgeURL(_ sender: Any?) {
        guard let path = coreDocument.path,
            let url = PathActions.forgeURL(forPath: path, isDirectory: false)
        else {
            NSSound.beep()
            return
        }
        PathActions.copy(url)
    }
}
