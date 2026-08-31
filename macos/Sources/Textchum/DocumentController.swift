import AppKit
import SwiftUI
import TextchumKit
import UniformTypeIdentifiers
import WebKit

/// Forwards script messages to a weak target, so the web view's user
/// content controller (which retains its handlers) cannot create a cycle.
private final class ScriptMessageProxy: NSObject, WKScriptMessageHandler {
    weak var target: DocumentController?

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
    /// The resolved save-preprocessor chain for (project root, language).
    var preprocessorCommands: (String?, String) -> [String] = { _, _ in [] }
    /// The effective hidden-name globs for a project root.
    var hiddenGlobs: (String) -> [String] = { _ in [".*"] }
    /// Expands the tree to a path and highlights it (Reveal in Tree).
    var revealInTree: (String) -> Void = { _ in }
    /// Whether the tree follows the focused file automatically.
    var followEnabled: () -> Bool = { true }
    let selectDocument: (ObjectIdentifier) -> Void
    /// Opens File Properties for a document in the navigator — the
    /// language badge is the obvious place to say "this is SQL".
    let showProperties: (ObjectIdentifier) -> Void
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

/// One view of a document: the text view, the scroll view around it,
/// the gutter beside it, and the container the three sit in.
///
/// A document can have several — the same buffer in two panes — and each
/// keeps its own place in the file. Colour and marks are rendering
/// attributes, which live on the layout manager rather than on the text,
/// so every view has to be painted.
@MainActor
final class DocumentView {
    let container = NSView()
    let scrollView: NSScrollView
    let textView: NSTextView
    let gutter: LineNumberGutterView
    /// The pinned context, laid over the top of the scroll view.
    let contextStrip = ContextStrip()

    init(scrollView: NSScrollView, textView: NSTextView, gutter: LineNumberGutterView) {
        self.scrollView = scrollView
        self.textView = textView
        self.gutter = gutter
        container.autoresizingMask = [.width, .height]
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        contextStrip.translatesAutoresizingMaskIntoConstraints = false
        contextStrip.isHidden = true
        container.addSubview(gutter)
        container.addSubview(scrollView)
        container.addSubview(contextStrip)
        NSLayoutConstraint.activate([
            gutter.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            gutter.topAnchor.constraint(equalTo: container.topAnchor),
            gutter.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: gutter.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: container.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            // Over the gutter too: the numbers under the pins belong
            // to hidden lines, and would sit beside the wrong text.
            contextStrip.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            contextStrip.trailingAnchor.constraint(equalTo: scrollView.trailingAnchor),
            contextStrip.topAnchor.constraint(equalTo: scrollView.topAnchor),
        ])
    }
}

/// One open document and everything the editor does with it.
///
/// It is not a window: a window (`Workbench`) holds tabs and panes, and
/// asks a document for a view when a pane is to show it. Commands reach
/// this through the responder chain, which the workbench points at the
/// document the focused pane is showing.
final class DocumentController: NSResponder {
    // Named to avoid NSWindowController's own `document` property.
    /// The document this is a view of. The core handle and the
    /// findings live there rather than here, so a second view of the
    /// same file shares them rather than keeping its own idea.
    /// `NSWindowController` has a `document` of its own, so this one
    /// says what it is.
    let openDocument: OpenDocument
    var coreDocument: CoreDocument { openDocument.core }

    /// The one floating list: references, the outline, the diagnostics.
    /// One at a time, so one panel.
    private let listPanel = ListPanel()
    /// This document's project root (nearest root marker), cached and
    /// refreshed when the path changes.
    private(set) var projectRoot: String?
    /// The navigator's folder-tree state, which belongs to the window.
    private var sidebarContext: WindowSidebarContext? { workbench?.sidebarContext }
    /// The (title, dirty, path) triple last published to the sidebar, to
    /// avoid rebuilding it on every keystroke.
    private var publishedState: (String, Bool, String?) = ("", false, nil)
    /// The window this document is shown in, and the views it has
    /// there — one per pane showing it.
    weak var workbench: Workbench?
    var window: NSWindow? { workbench?.window }
    private(set) var views: [DocumentView] = []
    /// The view the commands are about: the one with the keyboard, or
    /// the first one made.
    var focusedView: DocumentView? {
        views.first { $0.textView.window?.firstResponder === $0.textView } ?? views.first
    }
    private var textView: NSTextView? { focusedView?.textView }
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
    private var changeMarkTimer: Timer?
    /// Which request the gutter is waiting on: an answer from an
    /// older one is about text that has since been typed over.
    private var changeMarkGeneration: UInt64 = 0
    /// The latest language-server findings for this document, which
    /// belong to the document rather than to this view of it.
    private var diagnostics: [CoreDiagnostic] {
        get { openDocument.diagnostics }
        set { openDocument.diagnostics = newValue }
    }
    /// Every layout manager that has to be painted, one per view.
    private var paintTargets: [NSTextLayoutManager] {
        views.compactMap { $0.textView.textLayoutManager }
    }

    /// The views, for the smoke test to look at.
    var primaryView: NSTextView? { views.first?.textView }
    var secondaryView: NSTextView? { views.count > 1 ? views[1].textView : nil }
    var paintTargetCount: Int { paintTargets.count }

    /// The character a context-menu command is about, while one runs.
    /// A right-click does not move the caret, and the menu is about
    /// what was clicked.
    private var contextIndex: Int?
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
    /// The window's split view controller (sidebar · editor · preview),
    /// which belongs to the window rather than to this document.
    private var splitController: NSSplitViewController? { workbench?.splitController }
    /// The line-number gutter of the view with the keyboard.
    private var lineRuler: LineNumberGutterView? { focusedView?.gutter }
    /// The Markdown preview pane, present while the preview is shown.
    /// The Markdown preview beside the text, while this document shows
    /// one. It belongs to the document — the HTML is this file's — and
    /// the window puts it on screen for whichever document has the
    /// keyboard, so switching tabs does not leave one file's preview
    /// beside another file's text.
    private(set) var previewItem: NSSplitViewItem?
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
    /// The resolved save-preprocessor chain from the app's configuration.
    private let preprocessorCommands: (String?, String) -> [String]
    /// The effective hidden-name globs, from the app's configuration.
    private let hiddenGlobsProvider: (String) -> [String]
    /// Expands the shared tree to a path (checks the follow setting for
    /// automatic calls; the explicit action always reveals).
    private let revealPathInTree: (String) -> Void
    private let followEnabled: () -> Bool
    /// Where Save As should start for this (untitled) document: the
    /// folder of the file that was frontmost when it was created — the
    /// user was probably adding a file to that project.
    var suggestedSaveDirectory: URL?

    init(
        document: CoreDocument,
        settings: EditorSettings? = nil,
        sidebar: SidebarConfiguration? = nil,
        lspApp: CoreApp? = nil,
        openLocation: ((String, Int, Int) -> Void)? = nil
    ) {
        self.openDocument = DocumentStore.shared.open(document, path: document.path)
        self.lspApp = lspApp
        self.openLocation = openLocation
        self.resolveProjectRoot =
            sidebar?.resolveProjectRoot ?? { CoreWorkspace.projectRoot(forPath: $0) }
        self.workspaceSettingsJSON = sidebar?.workspaceSettingsJSON ?? { "{}" }
        self.preprocessorCommands = sidebar?.preprocessorCommands ?? { _, _ in [] }
        self.hiddenGlobsProvider = sidebar?.hiddenGlobs ?? { _ in [".*"] }
        self.revealPathInTree = sidebar?.revealInTree ?? { _ in }
        self.followEnabled = sidebar?.followEnabled ?? { true }
        self.appliedSettings = settings
        super.init()

        completionPopup.onAccept = { [weak self] item in
            self?.accept(completion: item)
        }
        projectRoot = coreDocument.path.flatMap { self.resolveProjectRoot($0) }
        // Before any view is made: how this file is shown is what the
        // first column asks for.
        adoptProjectState()
        startWatchingFile()
        syncLSPOpenState()
        appearanceObservation = NSApp.observe(\.effectiveAppearance) { [weak self] _, _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.refreshDecorations() }
            }
        }
    }

    private var cachedFoldSpans: [(opening: NSRange, hidden: NSRange)] = []
    private var foldSpansAreStale = true

    /// The settings to dress a new view in, kept so that the second view
    /// of a document looks like the first.
    private var appliedSettings: EditorSettings?

    /// A view of this document for a pane to show.
    ///
    /// The text views share one content storage, which is what makes
    /// them views of one document rather than copies of it: an edit in
    /// either is the same edit, and there is one history and one save.
    func makeView() -> DocumentView {
        let scrollView: NSScrollView
        let textView: NSTextView
        if let first = views.first,
            let contentManager = first.textView.textLayoutManager?.textContentManager
        {
            let layoutManager = NSTextLayoutManager()
            contentManager.addTextLayoutManager(layoutManager)
            let container = NSTextContainer(
                size: NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude))
            container.widthTracksTextView = true
            layoutManager.textContainer = container
            textView = NSTextView(frame: .zero, textContainer: container)
            textView.isVerticallyResizable = true
            textView.isHorizontallyResizable = false
            textView.autoresizingMask = [.width]
            scrollView = NSScrollView()
            scrollView.hasVerticalScroller = true
            scrollView.documentView = textView
        } else {
            scrollView = NSTextView.scrollableTextView()
            textView = scrollView.documentView as! NSTextView
        }
        textView.font = appliedSettings?.font ?? .monospacedSystemFont(ofSize: 13, weight: .regular)
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

        let gutter = LineNumberGutterView(textView: textView)
        gutter.setVisible(appliedSettings?.lineNumbers ?? true)
        let view = DocumentView(scrollView: scrollView, textView: textView, gutter: gutter)

        // The gutter, the change bar and the preview all follow scrolling.
        let clipView = scrollView.contentView
        clipView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(editorDidScroll(_:)),
            name: NSView.boundsDidChangeNotification,
            object: clipView
        )
        view.contextStrip.onSelect = { [weak self, weak view] line in
            guard let self, let view else { return }
            let offset = view.gutter.lineStart(ofLine: line)
            view.textView.setSelectedRange(NSRange(location: offset, length: 0))
            view.textView.scrollRangeToVisible(NSRange(location: offset, length: 0))
            view.textView.window?.makeFirstResponder(view.textView)
        }
        // Mouse-move tracking feeds language-server hover.
        textView.addTrackingArea(
            NSTrackingArea(
                rect: .zero,
                options: [.mouseMoved, .activeInKeyWindow, .inVisibleRect],
                owner: self,
                userInfo: nil
            ))

        if let content = textView.textLayoutManager?.textContentManager
            as? NSTextContentStorage
        {
            content.delegate = self
        }
        let isFirst = views.isEmpty
        views.append(view)
        if isFirst {
            textView.string = coreDocument.text
            // A file opens already differing from its committed self as
            // often as not, so the marks are wanted on the first paint.
            DispatchQueue.main.async { [weak self] in
                MainActor.assumeIsolated { self?.refreshChangeMarks() }
            }
        }
        if let appliedSettings {
            apply(settings: appliedSettings)
        }
        // A new view starts unpainted: colour is a rendering attribute,
        // and it has a layer of those to itself.
        applyHighlights(force: true)
        renderMarks()
        refreshChangeMarks()
        updateChrome()
        // Markdown opens with the live preview beside it.
        if coreDocument.languageName == "markdown" {
            showPreview()
        }
        DispatchQueue.main.async { [weak self, weak view] in
            MainActor.assumeIsolated {
                guard let self, let view else { return }
                self.updateContextStrip(for: view)
            }
        }
        return view
    }

    /// Takes a view back — the pane showing it has gone, or is showing
    /// another document now.
    func drop(_ view: DocumentView) {
        NotificationCenter.default.removeObserver(
            self,
            name: NSView.boundsDidChangeNotification,
            object: view.scrollView.contentView)
        if let layoutManager = view.textView.textLayoutManager,
            let contentManager = layoutManager.textContentManager,
            views.count > 1
        {
            contentManager.removeTextLayoutManager(layoutManager)
        }
        views.removeAll { $0 === view }
    }

    // MARK: What the file remembers

    /// Takes on what the project record says about this file: how it is
    /// shown, what is folded, and the language it was told it is.
    ///
    /// Called once the file's project is known, which is the first time
    /// there is anywhere to look.
    func adoptProjectState() {
        guard let path = coreDocument.path, let root = projectRoot else { return }
        guard let state = ProjectState.state(forPath: path, projectRoot: root) else {
            // Nothing recorded for it. A language chosen before records
            // existed still lives in the configuration; it moves the
            // next time this file is written down.
            adoptLegacyLanguage(path: path)
            return
        }
        if let language = state.language {
            languageOverride = language
            _ = coreDocument.setLanguage(language)
        } else {
            adoptLegacyLanguage(path: path)
        }
        openDocument.folds = state.folds.map { (start: $0.start, end: $0.end) }
        openDocument.layout = DocumentLayout(
            views: max(1, state.views),
            dividers: state.dividers,
            places: state.places.map {
                DocumentLayout.Place(caret: $0.caret, scroll: $0.scroll)
            })
        adoptedProjectState = true
    }

    /// The language override as it was kept before records existed:
    /// `files.<path>.language` in the configuration.
    private func adoptLegacyLanguage(path: String) {
        let stored = (NSApp.delegate as? AppDelegate)?.fileOverride(path: path)
        guard let language = stored?.language else { return }
        languageOverride = language
        _ = coreDocument.setLanguage(language)
    }

    /// Writes down what this file remembers. Cheap enough to call
    /// whenever something it covers changes.
    func recordProjectState() {
        guard adoptedProjectState || !openDocument.folds.isEmpty || languageOverride != nil,
            let path = coreDocument.path, let root = projectRoot
        else { return }
        let layout = openDocument.layout
        let state = CoreProjectState.FileState(
            views: max(1, layout.views),
            dividers: layout.dividers,
            folds: openDocument.folds.map { (start: $0.start, end: $0.end) },
            language: languageOverride,
            places: layout.places.map {
                CoreProjectState.FileState.Place(caret: $0.caret, scroll: $0.scroll)
            })
        ProjectState.record(state, forPath: path, projectRoot: root)
    }

    /// The pane showing this document took the keyboard.
    func didTakeFocus() {
        if let path = coreDocument.path {
            followInTree(path)
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

    /// Recomputes the gutter's git marks once typing pauses.
    ///
    /// Debounced and off the main thread: the marks come from asking
    /// git for the committed file, which is a process to spawn, and
    /// then diffing it. Neither belongs in a keystroke.
    private func scheduleChangeMarks() {
        changeMarkTimer?.invalidate()
        changeMarkTimer = Timer.scheduledTimer(withTimeInterval: 0.4, repeats: false) {
            [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.refreshChangeMarks() }
            }
        }
    }

    /// Asks for the marks now. The document is read here, on the main
    /// thread, and only the two strings cross to the background queue.
    func refreshChangeMarks() {
        changeMarkTimer?.invalidate()
        guard let lineRuler else { return }
        guard let path = coreDocument.path else {
            lineRuler.setChangeMarks([])
            return
        }
        let text = coreDocument.text
        let generation = changeMarkGeneration &+ 1
        changeMarkGeneration = generation
        DispatchQueue.global(qos: .utility).async { [weak self] in
            let marks = CoreChanges.marks(forPath: path, text: text)
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    guard let self, self.changeMarkGeneration == generation else { return }
                    self.lineRuler?.setChangeMarks(marks)
                }
            }
        }
    }

    /// Called by the app when a server publishes findings for this path.
    func apply(diagnostics: [CoreDiagnostic]) {
        self.diagnostics = diagnostics
        renderMarks()
        updateChrome()
    }

    // MARK: Hover

    /// Tracking-area callback: after the mouse rests for a beat, ask the
    /// server what is under it.
    override func mouseMoved(with event: NSEvent) {
        hoverPopover?.close()
        hoverPopover = nil
        guard let textView else { return }
        let point = textView.convert(event.locationInWindow, from: nil)
        // A diagnostic is already in hand and needs no server: an
        // underline nobody can read is a notification with the message
        // taken out. It shows whether or not hover documentation is on,
        // since the mark is on screen either way.
        //
        // Nothing reported means nothing to look for: most documents,
        // most of the time, and this runs on every mouse move.
        if !diagnostics.isEmpty, let index = characterIndex(at: point),
            let diagnostic = diagnostic(atOffset: index)
        {
            hoverTimer?.invalidate()
            hoverTimer = Timer.scheduledTimer(withTimeInterval: 0.35, repeats: false) {
                [weak self] _ in
                DispatchQueue.main.async {
                    MainActor.assumeIsolated {
                        self?.showBalloon(Self.diagnosticText(diagnostic), at: point)
                    }
                }
            }
            return
        }
        guard appliedHoverDocs, lspApp != nil, lspOpenPath != nil else { return }
        hoverTimer?.invalidate()
        hoverTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: false) {
            [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.requestHover(at: point) }
            }
        }
    }

    /// The diagnostic covering a character offset, if any. The
    /// narrowest wins: nested findings are common, and the innermost is
    /// the one about the thing under the pointer.
    private func diagnostic(atOffset offset: Int) -> CoreDiagnostic? {
        guard let textView else { return nil }
        let text = textView.string as NSString
        return diagnostics
            .compactMap { diagnostic -> (CoreDiagnostic, NSRange)? in
                guard let range = nsRange(of: diagnostic, in: text) else { return nil }
                // A zero-length finding still marks a spot; give it one
                // character's worth of reach so it can be pointed at.
                let reach = range.length == 0
                    ? NSRange(location: range.location, length: 1) : range
                guard NSLocationInRange(offset, reach) else { return nil }
                return (diagnostic, range)
            }
            .min { $0.1.length < $1.1.length }
            .map(\.0)
    }

    /// The character under a point in the text view, or nil when the
    /// point is not over laid-out text.
    ///
    /// This asks AppKit rather than doing the arithmetic. The version
    /// that did it by hand added a line fragment's own character index
    /// to the fragment's document offset, and
    /// `NSTextLineFragment.characterIndex(for:)` answers `NSNotFound`
    /// for a point it does not cover — `Int.max`, which made the
    /// addition overflow and trap. Every mouse move over the editor ran
    /// it, so it crashed the app rather than misplacing a balloon.
    private func characterIndex(at point: NSPoint) -> Int? {
        guard let textView else { return nil }
        return Self.characterIndex(at: point, in: textView)
    }

    /// Static so it can be swept with points from anywhere, including
    /// the ones that caused the crash, without a window.
    static func characterIndex(at point: NSPoint, in textView: NSTextView) -> Int? {
        guard let layoutManager = textView.textLayoutManager else { return nil }
        let length = (textView.string as NSString).length
        guard length > 0 else { return nil }
        // Over laid-out text at all: below the last line, or beside it,
        // there is no fragment and no character to name.
        let origin = textView.textContainerOrigin
        let inText = NSPoint(x: point.x - origin.x, y: point.y - origin.y)
        guard layoutManager.textLayoutFragment(for: inText) != nil else { return nil }
        let index = textView.characterIndexForInsertion(at: point)
        guard index >= 0 else { return nil }
        return min(index, length - 1)
    }

    /// A diagnostic as a balloon reads it: what kind of finding, then
    /// what it says. The severity is in the gutter as a colour and has
    /// to be in the words too, or a warning reads like an error.
    static func diagnosticText(_ diagnostic: CoreDiagnostic) -> NSAttributedString {
        let text = NSMutableAttributedString(
            string: severityName(diagnostic.severity) + "\n",
            attributes: [
                .font: NSFont.systemFont(ofSize: 11, weight: .semibold),
                .foregroundColor: NSColor.secondaryLabelColor,
            ])
        text.append(
            NSAttributedString(
                string: diagnostic.message,
                attributes: [
                    .font: NSFont.systemFont(ofSize: 13),
                    .foregroundColor: NSColor.labelColor,
                ]))
        return text
    }

    /// Reads out the diagnostic on the caret's line — the same answer
    /// the pointer gives, for when your hands are on the keyboard.
    @objc func showDiagnosticAtCaret(_ sender: Any?) {
        guard let textView else { return }
        let text = textView.string as NSString
        let caret = anchorIndex
        let found = diagnostic(atOffset: caret) ?? diagnosticOnLine(of: caret, in: text)
        guard let found else {
            NSSound.beep()
            return
        }
        let screenRect = textView.firstRect(
            forCharacterRange: NSRange(location: caret, length: 0), actualRange: nil)
        guard let window = textView.window else { return }
        let point = textView.convert(window.convertFromScreen(screenRect).origin, from: nil)
        showBalloon(Self.diagnosticText(found), at: point)
    }

    /// The first diagnostic anywhere on the caret's line. The caret is
    /// rarely inside the marked stretch — it is usually at the end of
    /// the line being fixed — and answering "nothing here" then would
    /// be true and useless.
    private func diagnosticOnLine(of offset: Int, in text: NSString) -> CoreDiagnostic? {
        let line = text.lineRange(for: NSRange(location: offset, length: 0))
        return diagnostics
            .compactMap { diagnostic -> (CoreDiagnostic, NSRange)? in
                guard let range = nsRange(of: diagnostic, in: text) else { return nil }
                return NSLocationInRange(range.location, line) ? (diagnostic, range) : nil
            }
            .min { $0.1.location < $1.1.location }
            .map(\.0)
    }

    /// Shows hover documentation for the symbol under the caret. Works
    /// even when mouse hover is switched off — this is the deliberate ask.
    @objc func showHoverAtCaret(_ sender: Any?) {
        guard let textView, lspApp != nil, lspOpenPath != nil else {
            NSSound.beep()
            return
        }
        let caret = min(textView.selectedRange().location, (textView.string as NSString).length)
        let screenRect = textView.firstRect(forCharacterRange: NSRange(location: caret, length: 0), actualRange: nil)
        guard let window = textView.window else { return }
        let windowRect = window.convertFromScreen(screenRect)
        let point = textView.convert(windowRect.origin, from: nil)
        requestHover(at: point, deliberate: true)
    }

    /// Where a command that acts "under the caret" should look.
    ///
    /// A right-click does not move the caret, and the menu it opens is
    /// about the character under the pointer. While one of that menu's
    /// items runs, this is that character; the rest of the time it is
    /// the caret.
    var anchorIndex: Int {
        guard let textView else { return 0 }
        let length = (textView.string as NSString).length
        return min(contextIndex ?? textView.selectedRange().location, length)
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

    private func requestHover(at point: NSPoint, deliberate: Bool = false) {
        guard let lspApp, let path = lspOpenPath, let textView else { return }
        let text = textView.string as NSString
        let index = textView.characterIndexForInsertion(at: point)
        guard index >= 0, index <= text.length else { return }
        // A passive mouse rest only asks the server about symbols:
        // whitespace, punctuation, the void past a line's end, and
        // comments have no documentation, and an empty answer still
        // costs a round trip and a popover flicker.
        if !deliberate, !isHoverableSymbol(at: index, in: text) { return }
        let (line, character) = Self.lspPosition(ofIndex: index, in: text)
        lspApp.lspHover(path: path, line: line, character: character) { [weak self] json in
            self?.showHover(resultJSON: json, at: point)
        }
    }

    /// Whether `index` sits on an identifier character outside a comment
    /// — the only places a hover request can have an answer.
    private func isHoverableSymbol(at index: Int, in text: NSString) -> Bool {
        guard index < text.length, let scalar = UnicodeScalar(text.character(at: index)) else {
            return false
        }
        var identifier = CharacterSet.alphanumerics
        identifier.insert("_")
        guard identifier.contains(scalar) else { return false }
        // Asked by name: style ids are positions in an alphabetical
        // table and move whenever a capture is added.
        let spans = coreDocument.highlights(in: NSRange(location: index, length: 1))
        return !spans.contains { span in
            CoreTheme.commentStyleID.map { span.styleIndex == $0 } ?? false
                && NSLocationInRange(index, span.range)
        }
    }

    // MARK: Go to definition

    /// Jumps to the definition of the symbol under the caret: the
    /// language server's answer, or the ctags index for projects that
    /// opted into the fallback (also consulted when the server has no
    /// answer).
    ///
    /// With the caret already on the definition the key has nowhere to
    /// go, so it asks the question that is left — who uses this.
    @objc func jumpToDefinition(_ sender: Any?) {
        guard let textView else { return }
        guard let lspApp, let path = lspOpenPath else {
            if !ctagsJump() { NSSound.beep() }
            return
        }
        let text = textView.string as NSString
        let (line, character) = Self.lspPosition(ofIndex: anchorIndex, in: text)
        lspApp.lspDefinition(path: path, line: line, character: character) { [weak self] json in
            guard let self else { return }
            switch CoreDefinition.decide(
                result: json, path: path, line: line, character: character)
            {
            case .jump(let target):
                self.openLocation?(target.path, target.line, target.character)
            case .references:
                self.usesOfDefinition(path: path, line: line, character: character)
            case .choose(let targets):
                self.showReferences(
                    targets.map {
                        ReferenceLocation(path: $0.path, line: $0.line, character: $0.character)
                    },
                    title: "Definitions (\(targets.count))")
            case .nothing:
                if !self.ctagsJump() { NSSound.beep() }
            }
        }
    }

    /// The caret is on the definition, so the jump key asks who uses
    /// the symbol. One use is a jump — opening a panel to offer a
    /// single row asks for a keystroke that decides nothing. Several
    /// open the panel. None says so: a beep here reads as a failure,
    /// and the answer is that nothing refers to it.
    private func usesOfDefinition(path: String, line: Int, character: Int) {
        guard let lspApp else { return }
        lspApp.lspReferences(path: path, line: line, character: character) { [weak self] json in
            guard let self else { return }
            let uses = CoreDefinition.elsewhere(
                result: json, path: path, line: line, character: character)
            switch uses.count {
            case 0:
                self.presentInfo(
                    t("On the definition"),
                    details: t("Nothing else in the workspace refers to this symbol."))
            case 1:
                self.openLocation?(uses[0].path, uses[0].line, uses[0].character)
            default:
                self.showReferences(
                    uses.map {
                        ReferenceLocation(path: $0.path, line: $0.line, character: $0.character)
                    },
                    title: "Uses (\(uses.count))")
            }
        }
    }

    // MARK: References, rename, formatting

    /// Lists every reference to the symbol under the caret.
    @objc func findReferences(_ sender: Any?) {
        guard let lspApp, let path = lspOpenPath, let textView else { return }
        let text = textView.string as NSString
        let (line, character) = Self.lspPosition(ofIndex: anchorIndex, in: text)
        lspApp.lspReferences(path: path, line: line, character: character) { [weak self] json in
            guard let self else { return }
            let locations = Self.referenceLocations(fromResultJSON: json)
            guard !locations.isEmpty else {
                NSSound.beep()
                return
            }
            self.showReferences(locations)
        }
    }

    /// Every finding in the document, in the order they appear — which
    /// is the order they are fixed in, and the order the gutter shows
    /// them. Severity is in each row, so a file whose errors matter
    /// more than its warnings is still readable at a glance.
    @objc func showDiagnosticList(_ sender: Any?) {
        guard let textView else { return }
        guard !diagnostics.isEmpty else {
            presentInfo(t("Nothing reported"), details: t("No diagnostics for this document."))
            return
        }
        let text = textView.string as NSString
        let ordered = diagnostics.sorted {
            ($0.line, $0.character) < ($1.line, $1.character)
        }
        let rows: [ListPanel.Row] = ordered.map { diagnostic in
            let kind = Self.severityName(diagnostic.severity)
            let message = diagnostic.message
                .split(separator: "\n", maxSplits: 1)
                .first
                .map(String.init) ?? diagnostic.message
            return .item("\(diagnostic.line + 1)  \(kind)  \(message)")
        }
        listPanel.show(
            rows: rows, over: window,
            title: "Diagnostics (\(ordered.count))", placeholder: t("message…")
        ) { [weak self] index in
            guard let self, ordered.indices.contains(index) else { return }
            let diagnostic = ordered[index]
            guard let range = self.nsRange(of: diagnostic, in: text) else { return }
            (NSApp.delegate as? AppDelegate)?.recordJumpOrigin()
            self.selectionChangeIsFromEditing = false
            textView.setSelectedRange(NSRange(location: range.location, length: 0))
            textView.scrollRangeToVisible(range)
            self.window?.makeFirstResponder(textView)
        }
    }

    /// What kind of finding a severity is, in words.
    static func severityName(_ severity: Int) -> String {
        switch severity {
        case 1: return "Error"
        case 2: return "Warning"
        case 3: return "Information"
        case 4: return "Hint"
        default: return "Diagnostic"
        }
    }

    /// A plain statement, for when the answer is "there is nothing".
    private func presentInfo(_ message: String, details: String) {
        let alert = NSAlert()
        alert.alertStyle = .informational
        alert.messageText = message
        alert.informativeText = details
        alert.runModal()
    }

    /// The document outline: the file's symbols, nesting shown by
    /// indentation, filterable because a large file's outline is longer
    /// than a screen.
    private func showOutline(_ symbols: [OutlineSymbol]) {
        let rows: [ListPanel.Row] = symbols.map { symbol in
            .item(String(repeating: "  ", count: symbol.depth) + symbol.name)
        }
        listPanel.show(
            rows: rows, over: window, title: t("Document Outline"), placeholder: t("symbol…")
        ) { [weak self] index in
            guard let self, let path = self.coreDocument.path,
                symbols.indices.contains(index)
            else { return }
            let symbol = symbols[index]
            self.openLocation?(path, symbol.line, symbol.character)
        }
    }

    /// The references list: code first, tests after, each under a
    /// heading with a count. What calls this is the question; what
    /// checks it is the follow-up. All of one or the other gets no
    /// headings — a heading over every row there is says nothing.
    private func showReferences(_ locations: [ReferenceLocation], title: String? = nil) {
        var lineCache: [String: [Substring]] = [:]
        func lineText(_ location: ReferenceLocation) -> String {
            if lineCache[location.path] == nil {
                let contents = (try? String(contentsOfFile: location.path, encoding: .utf8)) ?? ""
                lineCache[location.path] = contents.split(
                    separator: "\n", omittingEmptySubsequences: false)
            }
            let lines = lineCache[location.path] ?? []
            guard lines.indices.contains(location.line) else { return "" }
            return lines[location.line].trimmingCharacters(in: .whitespaces)
        }
        func described(_ location: ReferenceLocation) -> String {
            let name = (location.path as NSString).lastPathComponent
            return "\(name):\(location.line + 1): \(lineText(location))"
        }

        let code = locations.filter { !CoreReferences.isTest(path: $0.path) }
        let tests = locations.filter { CoreReferences.isTest(path: $0.path) }
        var rows: [ListPanel.Row] = []
        var ordered: [ReferenceLocation] = []
        if code.isEmpty || tests.isEmpty {
            rows = locations.map { .item(described($0)) }
            ordered = locations
        } else {
            rows.append(.heading("Code (\(code.count))"))
            rows.append(contentsOf: code.map { .item(described($0)) })
            rows.append(.heading("Tests (\(tests.count))"))
            rows.append(contentsOf: tests.map { .item(described($0)) })
            ordered = code + tests
        }
        listPanel.show(
            rows: rows, over: window,
            title: title ?? "References (\(locations.count))",
            monospaced: true
        ) { [weak self] index in
            guard ordered.indices.contains(index) else { return }
            let location = ordered[index]
            self?.openLocation?(location.path, location.line, location.character)
        }
    }

    private static func referenceLocations(
        fromResultJSON json: String
    ) -> [ReferenceLocation] {
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
            return ReferenceLocation(path: path, line: line, character: character)
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
        alert.messageText = t("Rename Symbol")
        alert.informativeText =
            current.isEmpty ? "New name:" : "New name for “\(current)”:"
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.stringValue = current
        alert.accessoryView = field
        alert.addButton(withTitle: t("Rename"))
        alert.addButton(withTitle: t("Cancel"))
        alert.window.initialFirstResponder = field
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let newName = field.stringValue.trimmingCharacters(in: .whitespaces)
        guard !newName.isEmpty, newName != current else { return }
        let text = textView.string as NSString
        let (line, character) = Self.lspPosition(ofIndex: anchorIndex, in: text)
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

    /// Reformats the whole document: the language server's formatter,
    /// or the save-preprocessor chain when no server can help (untitled
    /// documents included — servers speak in files, chains do not care).
    @objc func formatDocument(_ sender: Any?) {
        guard let lspApp, let path = lspOpenPath, let textView else {
            formatViaPreprocessors()
            return
        }
        // Respect what the document already does: a tab-indented file
        // keeps tabs, everything else formats with spaces.
        let usesTabs =
            textView.string.contains("\n\t") || textView.string.hasPrefix("\t")
        lspApp.lspFormatting(path: path, tabSize: appliedTabWidth, insertSpaces: !usesTabs) {
            [weak self] json in
            guard let self else { return }
            let edits = LSPEdits.textEdits(fromResultJSON: json)
            guard !edits.isEmpty else {
                self.formatViaPreprocessors()
                return
            }
            self.apply(textEdits: edits)
        }
    }

    /// Format Document's fallback: the configured chain, or a beep when
    /// neither a server nor a chain can help.
    private func formatViaPreprocessors() {
        guard let language = coreDocument.languageName,
            !preprocessorCommands(projectRoot, language).isEmpty
        else {
            NSSound.beep()
            return
        }
        if case .failed(let failure) = preprocessBuffer() {
            presentError(
                "Preprocessor failed: \(failure.command)", details: failure.details)
        }
    }

    /// The document's headings as an outline — what a long post needs,
    /// and what no Markdown language server is required for. Returns
    /// whether there was anything to show.
    @discardableResult
    private func showMarkdownOutline() -> Bool {
        guard coreDocument.languageName == "markdown", let textView else { return false }
        let headings = CoreWorkspace.markdownHeadings(in: textView.string)
        guard !headings.isEmpty else { return false }
        let symbols = headings.map { heading in
            OutlineSymbol(
                name: heading.text,
                kind: "h\(heading.level)",
                line: heading.line,
                character: heading.character,
                // Nesting mirrors heading depth, so a post reads like
                // its own table of contents.
                depth: heading.level - 1
            )
        }
        self.showOutline(symbols)
        return true
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

    // MARK: Folding

    /// Folding hides the lines after the one that opens a block, and
    /// marks that line with an ellipsis.
    ///
    /// TextKit 2 lays out what the content storage offers, so a fold is
    /// a change to what the document says its paragraphs are rather
    /// than an attribute on the text. The lines inside a fold are handed
    /// over as a bare separator at a hundredth of a point — measured at
    /// 0.01pt each, against 16pt for a line of code.
    ///
    /// Withholding those paragraphs instead is the obvious move and does
    /// not work: `NSTextViewportLayoutController` cannot walk an
    /// enumeration with a gap in it, and the text area renders nothing
    /// at all. Measured before this was written.
    @objc func toggleFold(_ sender: Any?) {
        guard let textView else { return }
        let line = lineNumber(ofOffset: textView.selectedRange().location)
        if let at = openDocument.folds.firstIndex(where: { $0.start == line }) {
            openDocument.folds.remove(at: at)
            refreshFolds()
            return
        }
        guard let range = coreDocument.foldRanges.first(where: { $0.start == line }) else {
            NSSound.beep()
            return
        }
        openDocument.folds.append(range)
        refreshFolds()
    }

    /// Folds every block that is not inside one already folded.
    @objc func foldAll(_ sender: Any?) {
        var folds: [(start: Int, end: Int)] = []
        for range in coreDocument.foldRanges {
            // A block inside one already folded is hidden either way.
            if folds.contains(where: { range.start > $0.start && range.start <= $0.end }) {
                continue
            }
            folds.append(range)
        }
        if folds.isEmpty {
            NSSound.beep()
            return
        }
        openDocument.folds = folds
        refreshFolds()
    }

    @objc func unfoldAll(_ sender: Any?) {
        guard !openDocument.folds.isEmpty else {
            NSSound.beep()
            return
        }
        openDocument.folds = []
        refreshFolds()
    }

    var hasFolds: Bool { !openDocument.folds.isEmpty }

    /// The line a character offset is on, zero-based.
    private func lineNumber(ofOffset offset: Int) -> Int {
        let text = coreDocument.text as NSString
        var line = 0
        var index = 0
        while index < offset, index < text.length {
            index = NSMaxRange(text.lineRange(for: NSRange(location: index, length: 0)))
            if index <= offset { line += 1 }
        }
        return line
    }

    /// Rebuilds what the folds hide and lays the views out again.
    func refreshFolds() {
        foldSpansAreStale = true
        recordProjectState()
        for view in views {
            guard let content = view.textView.textLayoutManager?.textContentManager
                as? NSTextContentStorage
            else { continue }
            content.performEditingTransaction {
                let length = content.textStorage?.length ?? 0
                content.textStorage?.edited(
                    .editedAttributes, range: NSRange(location: 0, length: length),
                    changeInLength: 0)
            }
            view.gutter.invalidateLineStarts()
            view.gutter.needsDisplay = true
        }
        applyHighlights(force: true)
        renderMarks()
    }

    /// Where the folds are, in characters: the line that opens each one
    /// and the run it hides. Recomputed when the folds or the text
    /// change, and only while something is folded.
    private func foldSpans() -> [(opening: NSRange, hidden: NSRange)] {
        if !foldSpansAreStale { return cachedFoldSpans }
        foldSpansAreStale = false
        cachedFoldSpans = []
        guard !openDocument.folds.isEmpty, let textView else { return [] }
        let text = textView.string as NSString
        // One walk of the file, not one per paragraph.
        var starts: [Int] = [0]
        var index = 0
        while index < text.length {
            index = NSMaxRange(text.lineRange(for: NSRange(location: index, length: 0)))
            starts.append(index)
        }
        for fold in openDocument.folds {
            guard fold.start >= 0, fold.start + 1 < starts.count, fold.end < starts.count
            else { continue }
            let opening = text.lineRange(for: NSRange(location: starts[fold.start], length: 0))
            let from = starts[fold.start + 1]
            let to = min(starts[min(fold.end + 1, starts.count - 1)], text.length)
            guard to > from else { continue }
            cachedFoldSpans.append((opening, NSRange(location: from, length: to - from)))
        }
        return cachedFoldSpans
    }

    // MARK: Columns and views

    /// Edit ▸ New Column: another column beside this one, showing the
    /// same file to start with. It takes any tab afterwards.
    @objc func newColumn(_ sender: Any?) {
        workbench?.newColumn()
    }

    /// Edit ▸ Close Column.
    @objc func closeColumn(_ sender: Any?) {
        workbench?.closeColumn()
    }

    /// Edit ▸ Second View: this file again, under the view that has the
    /// keyboard. One buffer, two places to look at it — the top of a
    /// function while its end is being written.
    @objc func addView(_ sender: Any?) {
        workbench?.addViewToFocusedColumn()
    }

    /// Edit ▸ Close View.
    @objc func closeView(_ sender: Any?) {
        workbench?.closeFocusedView()
    }

    /// Edit ▸ Next Pane: the keyboard moves to the next view down this
    /// column, then to the next column along.
    @objc func focusOtherSide(_ sender: Any?) {
        workbench?.focusOtherPane()
    }

    /// File ▸ Close (⌘W): the tab, not the window. Closing the last
    /// tab closes the window with it.
    @objc func closeTab(_ sender: Any?) {
        guard let workbench else { return }
        workbench.closeTab(ObjectIdentifier(self))
    }

    /// Window ▸ Move Tab to New Window.
    @objc func moveTabToNewWindow(_ sender: Any?) {
        guard let workbench, workbench.documents.count > 1,
            let delegate = NSApp.delegate as? AppDelegate
        else { return }
        workbench.detach(self)
        let fresh = delegate.makeWorkbench()
        fresh.add(self)
        fresh.showWindow(nil)
        fresh.window?.makeKeyAndOrderFront(nil)
        NotificationCenter.default.post(name: .textchumDocumentsChanged, object: nil)
    }

    /// Window ▸ Next/Previous Tab, in the pane with the keyboard.
    @objc func selectNextTab(_ sender: Any?) {
        workbench?.cycleTab(forward: true)
    }

    @objc func selectPreviousTab(_ sender: Any?) {
        workbench?.cycleTab(forward: false)
    }

    /// Window ▸ This File in Every Column.
    @objc func showInEveryPane(_ sender: Any?) {
        workbench?.showEverywhere(ObjectIdentifier(self))
    }

    // MARK: Code actions

    /// What the server offers for the place the caret is: the quick fix
    /// for the diagnostic there, or the refactorings it has for the
    /// range.
    ///
    /// The findings under the caret go with the request, as the server
    /// itself published them — the core keeps those, since a
    /// reconstructed diagnostic is not one a server recognizes as its
    /// own. Without them a server answers with what it can do to that
    /// range in general, which is not the question a marked line asks.
    @objc func showCodeActions(_ sender: Any?) {
        guard let lspApp, let path = lspOpenPath, let textView else { return }
        let text = textView.string as NSString
        let (line, character) = Self.lspPosition(ofIndex: anchorIndex, in: text)
        lspApp.lspCodeAction(path: path, line: line, character: character) {
            [weak self] json in
            guard let self else { return }
            let actions = CoreCodeActions.actions(inResultJSON: json)
            guard !actions.isEmpty else {
                self.presentInfo(
                    t("Nothing on offer"),
                    details: t("The language server has no action for this place."))
                return
            }
            let rows: [ListPanel.Row] = actions.map { action in
                .item(action.preferred ? "\(action.title)  ·  suggested" : action.title)
            }
            self.listPanel.show(
                rows: rows, over: self.window, title: "Code Actions (\(actions.count))",
                placeholder: t("action…")
            ) { [weak self] index in
                guard let self, actions.indices.contains(index) else { return }
                self.run(CoreCodeActions.outcome(inResultJSON: json, at: index), path: path)
            }
        }
    }

    /// Carries out a chosen action: apply the edit, run the command, or
    /// ask the server to fill the edit in first — servers are allowed
    /// to answer cheaply and compute only the one that was chosen.
    private func run(_ outcome: CoreCodeActions.Outcome, path: String) {
        let delegate = NSApp.delegate as? AppDelegate
        switch outcome {
        case .edit(let edit):
            _ = delegate?.applyWorkspaceEdit(resultJSON: edit)
        case .command(let name, let argumentsJSON):
            lspApp?.lspExecuteCommand(path: path, command: name, argumentsJSON: argumentsJSON) {
                // A command's own answer is the server saying it is
                // done; the edits it made arrive as a workspace/applyEdit
                // request, which the pool handles.
                _ = $0
            }
        case .resolve(let actionJSON):
            lspApp?.lspResolveCodeAction(path: path, actionJSON: actionJSON) {
                [weak self] json in
                guard let self else { return }
                // The resolved action carries the edit; anything else
                // means the server had nothing after all.
                switch CoreCodeActions.outcome(inResultJSON: "[\(json)]", at: 0) {
                case .edit(let edit):
                    _ = (NSApp.delegate as? AppDelegate)?.applyWorkspaceEdit(resultJSON: edit)
                case .command(let name, let argumentsJSON):
                    self.lspApp?.lspExecuteCommand(
                        path: path, command: name, argumentsJSON: argumentsJSON) { _ in }
                default:
                    self.presentInfo(
                        t("Nothing came back"),
                        details: t("The language server had no edit for that action."))
                }
            }
        case .nothing:
            NSSound.beep()
        }
    }

    // MARK: Text transformations

    /// Sorts, cases, trims or converts the selection — or the whole
    /// document when nothing is selected, since that is what the
    /// operation is about when no part of it was singled out.
    ///
    /// A line-wise transformation is given whole lines: the selection
    /// grows to the boundaries around it first, because sorting half a
    /// line is not something anyone asked for.
    @objc func transformText(_ sender: NSMenuItem) {
        guard let textView, let kind = sender.representedObject as? String else { return }
        let text = textView.string as NSString
        var range = textView.selectedRange()
        if range.length == 0 {
            range = NSRange(location: 0, length: text.length)
        } else if CoreTransform.isLineWise(kind) {
            range = text.lineRange(for: range)
        }
        guard range.length > 0,
            let replacement = CoreTransform.apply(kind, to: text.substring(with: range)),
            replacement != text.substring(with: range)
        else { return }
        textView.insertText(replacement, replacementRange: range)
        // The transformed stretch stays selected, so a second one can
        // follow without selecting it again.
        textView.setSelectedRange(
            NSRange(location: range.location, length: (replacement as NSString).length))
    }

    /// View → Document Outline (⇧⌘O): the file's symbols from its
    /// server, fuzzy-filterable; selecting one jumps (via the jump
    /// stack, so Go Back returns here).
    @objc func showDocumentOutline(_ sender: Any?) {
        guard let lspApp, let path = lspOpenPath else {
            // Markdown has an outline without any server: its headings.
            if !showMarkdownOutline() { NSSound.beep() }
            return
        }
        lspApp.lspDocumentSymbols(path: path) { [weak self] json in
            guard let self else { return }
            let symbols = OutlineSymbol.parse(resultJSON: json)
            guard !symbols.isEmpty else {
                if !self.showMarkdownOutline() { NSSound.beep() }
                return
            }
            self.showOutline(symbols)
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
        var start = anchorIndex
        // A caret just past the last character of a word still means it.
        if !isWord(start), isWord(start - 1) { start -= 1 }
        guard isWord(start) else { return nil }
        var end = start + 1
        while isWord(start - 1) { start -= 1 }
        while isWord(end) { end += 1 }
        return text.substring(with: NSRange(location: start, length: end - start))
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

    /// Blame Line (⌃⌘B): what git knows about the line under the caret.
    ///
    /// The buffer's text goes to git along with the line number, so an
    /// unsaved edit above the caret cannot shift the answer onto the
    /// neighbouring line — which would arrive looking exactly as right
    /// as a correct one.
    @objc func blameLine(_ sender: Any?) {
        guard let textView else { return }
        guard let path = coreDocument.path else {
            presentError(
                "This document has no file yet.",
                details: t("Save it before asking git who wrote a line."))
            return
        }
        let text = textView.string as NSString
        let line = Self.lspPosition(ofIndex: anchorIndex, in: text).0 + 1
        do {
            let blame = try CoreBlame.line(line, ofPath: path, text: coreDocument.text)
            BlamePanel.shared.show(blame, file: path, over: window)
        } catch {
            presentError("git could not blame this line.", details: "\(error)")
        }
    }

    /// Go to Line (⌘L): a prompt taking a number, and taking the shapes
    /// a number arrives in — `412:8` from a compiler, a whole
    /// `src/main.rs:412:8` pasted out of a build log, `line 412` from a
    /// stack trace. Reading it is the core's job so both shells accept
    /// the same things.
    @objc func goToLine(_ sender: Any?) {
        guard let textView else { return }
        let alert = NSAlert()
        alert.messageText = t("Go to Line")
        alert.informativeText = "Line number, or line:column — of \(coreDocument.lineCount)."
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 220, height: 24))
        field.placeholderString = "412 or 412:8"
        alert.accessoryView = field
        alert.addButton(withTitle: t("Go"))
        alert.addButton(withTitle: t("Cancel"))
        alert.window.initialFirstResponder = field
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        guard let target = CoreDocument.parseGoTo(field.stringValue) else {
            // Nothing in what was typed names a line. Saying so beats
            // jumping somewhere arbitrary.
            NSSound.beep()
            return
        }
        // The jump is one Go Back should return from: reading was
        // interrupted here.
        (NSApp.delegate as? AppDelegate)?.recordJumpOrigin()
        let offset = coreDocument.offset(ofLine: target.line, column: target.column)
        let clamped = min(offset, (textView.string as NSString).length)
        selectionChangeIsFromEditing = false
        textView.setSelectedRange(NSRange(location: clamped, length: 0))
        // Centred rather than merely visible: a line scrolled to the
        // last row of the window is a line you have to scroll to read.
        centerSelection()
        window?.makeFirstResponder(textView)
    }

    /// Scrolls so the selection sits near the middle of the view, when
    /// there is enough document either side of it to do so.
    private func centerSelection() {
        guard let textView, let scrollView = textView.enclosingScrollView else { return }
        let rect = textView.firstRect(
            forCharacterRange: textView.selectedRange(), actualRange: nil)
        guard rect.height > 0, let window = textView.window else {
            textView.scrollRangeToVisible(textView.selectedRange())
            return
        }
        let inWindow = window.convertPoint(fromScreen: rect.origin)
        let inView = textView.convert(inWindow, from: nil)
        let height = scrollView.contentView.bounds.height
        let target = max(0, inView.y - height / 2)
        textView.scroll(NSPoint(x: 0, y: target))
        scrollView.reflectScrolledClipView(scrollView.contentView)
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

    /// Renders LSP hover markdown for the balloon: fenced code blocks in
    /// the monospaced font, everything else through Foundation's inline
    /// markdown parser (bold, italics, `code` spans, links as plain
    /// styled text). Block constructs beyond fences degrade gracefully
    /// to their literal text, which is how servers expect unsupporting
    /// clients to behave.
    static func hoverAttributedText(fromMarkdown content: String) -> NSAttributedString {
        let bodyFont = NSFont.systemFont(ofSize: 12)
        let codeFont = NSFont.monospacedSystemFont(ofSize: 11.5, weight: .regular)
        let result = NSMutableAttributedString()
        let append = { (chunk: String, isCode: Bool) in
            let trimmed = chunk.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { return }
            if result.length > 0 {
                result.append(NSAttributedString(
                    string: "\n\n", attributes: [.font: bodyFont]))
            }
            if isCode {
                result.append(NSAttributedString(
                    string: trimmed,
                    attributes: [.font: codeFont, .foregroundColor: NSColor.textColor]))
                return
            }
            var options = AttributedString.MarkdownParsingOptions()
            options.interpretedSyntax = .inlineOnlyPreservingWhitespace
            let styled: NSMutableAttributedString
            if let parsed = try? AttributedString(markdown: trimmed, options: options) {
                styled = NSMutableAttributedString(attributedString: NSAttributedString(parsed))
            } else {
                styled = NSMutableAttributedString(string: trimmed)
            }
            let full = NSRange(location: 0, length: styled.length)
            styled.addAttribute(.foregroundColor, value: NSColor.textColor, range: full)
            styled.enumerateAttribute(.inlinePresentationIntent, in: full) { value, range, _ in
                let intent = value as? InlinePresentationIntent ?? []
                if intent.contains(.code) {
                    styled.addAttribute(.font, value: codeFont, range: range)
                } else {
                    var traits: NSFontDescriptor.SymbolicTraits = []
                    if intent.contains(.stronglyEmphasized) { traits.insert(.bold) }
                    if intent.contains(.emphasized) { traits.insert(.italic) }
                    let descriptor = bodyFont.fontDescriptor.withSymbolicTraits(traits)
                    let font = NSFont(descriptor: descriptor, size: bodyFont.pointSize)
                    styled.addAttribute(.font, value: font ?? bodyFont, range: range)
                }
            }
            result.append(styled)
        }
        // Split on fence lines by hand; the odd chunks are code. The
        // language tag after the opening ``` is dropped.
        var isCode = false
        var chunk: [Substring] = []
        for line in content.split(separator: "\n", omittingEmptySubsequences: false) {
            if line.trimmingCharacters(in: .whitespaces).hasPrefix("```") {
                append(chunk.joined(separator: "\n"), isCode)
                chunk = []
                isCode.toggle()
            } else {
                chunk.append(line)
            }
        }
        append(chunk.joined(separator: "\n"), isCode)
        return result
    }

    private func showHover(resultJSON: String, at point: NSPoint) {
        guard let content = Self.hoverText(fromResultJSON: resultJSON) else { return }
        showBalloon(Self.hoverAttributedText(fromMarkdown: content), at: point)
    }

    /// The balloon hover documentation and diagnostics both appear in.
    private func showBalloon(_ attributed: NSAttributedString, at point: NSPoint) {
        guard let textView else { return }
        hoverPopover?.close()

        // Measured by hand, framed by hand: Auto Layout inside an
        // NSPopover collapsed the wrapping label to a sliver and showed
        // an empty balloon — the popover sized itself while the label
        // laid out at zero. Explicit frames cannot disagree with the
        // popover about geometry.
        guard attributed.length > 0 else { return }
        let label = NSTextField(wrappingLabelWithString: "")
        label.attributedStringValue = attributed
        // Ask the field itself how it wraps — an NSString measurement
        // can disagree with the control by a word, clipping the tail.
        let fitted = label.sizeThatFits(NSSize(width: 480, height: 800))
        let textSize = NSSize(
            width: ceil(fitted.width) + 4, height: ceil(fitted.height) + 2)
        label.frame = NSRect(x: 12, y: 10, width: textSize.width, height: textSize.height)
        let container = NSView(
            frame: NSRect(
                x: 0, y: 0,
                width: textSize.width + 24, height: textSize.height + 20))
        container.addSubview(label)
        let controller = NSViewController()
        controller.view = container

        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = controller
        popover.contentSize = container.frame.size
        popover.show(
            relativeTo: NSRect(origin: point, size: NSSize(width: 1, height: 1)),
            of: textView,
            preferredEdge: .maxY
        )
        hoverPopover = popover
    }

    /// Debug hook: scrolls to a fraction of the document, so the
    /// viewport-scoped colouring can be checked far from the start.
    func debugScroll(toFraction fraction: Double) {
        guard let textView, let scrollView = textView.enclosingScrollView else { return }
        let total = scrollView.documentView?.frame.height ?? 0
        let visible = scrollView.contentView.bounds.height
        let target = max(0, (total - visible) * fraction)
        scrollView.contentView.scroll(to: NSPoint(x: 0, y: target))
        scrollView.reflectScrolledClipView(scrollView.contentView)
    }

    /// Debug hook: renders a hover balloon with known content at a fixed
    /// spot, so the popover's layout is screenshot-verifiable without
    /// synthesizing mouse events.
    func debugShowHover() {
        showHover(
            resultJSON: #"{"contents": {"kind": "markdown", "value": "```go\nfunc Frobnicate(x int) int\n```\n\nTurns **x** into a properly `frobnicated` value, *carefully*."}}"#,
            at: NSPoint(x: 200, y: 100))
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
        .front-matter { display: grid; grid-template-columns: auto 1fr;
               gap: .15em 1em; margin: 0 0 1.4em;
               padding: .8em 1em; border-radius: 6px;
               background: rgba(128,128,128,.10);
               border-left: 3px solid rgba(128,128,128,.45);
               font-size: .9em; }
        .front-matter dt { grid-column: 1; margin: 0; font-weight: 600;
               opacity: .75; }
        .front-matter dd { grid-column: 2; margin: 0;
               font-family: ui-monospace, monospace; }
        .shortcode { display: inline-block; padding: .05em .5em;
               border-radius: 999px; font-size: .85em;
               font-family: ui-monospace, monospace;
               background: rgba(128,128,128,.18);
               border: 1px solid rgba(128,128,128,.35); }
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
        guard previewItem == nil, coreDocument.languageName == "markdown" else { return }

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
        previewItem = item
        previewWebView = webView
        workbench?.refreshPreview()
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
        guard previewItem != nil else { return }
        previewItem = nil
        previewWebView = nil
        workbench?.refreshPreview()
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

    /// Recomputes the pinned context for one view from its scroll
    /// position: at most five rows, rebuilt only when the lines change.
    func updateContextStrip(for view: DocumentView) {
        let strip = view.contextStrip
        guard appliedSettings?.contextLines ?? true, coreDocument.languageName != nil else {
            strip.show(lines: [], text: { _ in NSAttributedString() }, rowHeight: 0)
            return
        }
        let clip = view.scrollView.contentView.bounds
        let offset = view.textView.characterIndexForInsertion(
            at: NSPoint(x: 5, y: clip.minY + 1))
        let topLine = view.gutter.lineIndex(forOffset: offset)
        let lines = coreDocument.contextLines(
            topLine: topLine, maxRows: ContextStrip.maxRows)
        let font = appliedSettings?.font ?? .monospacedSystemFont(ofSize: 13, weight: .regular)
        let rowHeight = (font.ascender - font.descender + font.leading).rounded(.up) + 2
        strip.show(
            lines: lines,
            text: { [weak self] line in
                self?.attributedLine(line, in: view) ?? NSAttributedString()
            },
            rowHeight: rowHeight)
    }

    /// One line the way the editor shows it, colours included, without
    /// its line break.
    private func attributedLine(_ line: Int, in view: DocumentView) -> NSAttributedString {
        guard let storage = view.textView.textStorage else { return NSAttributedString() }
        let text = storage.string as NSString
        let start = view.gutter.lineStart(ofLine: line)
        guard start < text.length else { return NSAttributedString() }
        var range = text.lineRange(for: NSRange(location: start, length: 0))
        while range.length > 0,
            [10, 13].contains(text.character(at: NSMaxRange(range) - 1))
        {
            range.length -= 1
        }
        return storage.attributedSubstring(from: range)
    }

    @objc private func editorDidScroll(_ notification: Notification) {
        if let clipView = notification.object as? NSClipView,
            let view = views.first(where: { $0.scrollView.contentView === clipView })
        {
            updateContextStrip(for: view)
        }
        lineRuler?.needsDisplay = true
        completionPopup.dismiss()
        // Colouring follows the viewport, so scrolling into fresh text
        // has to paint it. Coalesced: one pass per runloop turn, not
        // one per scroll tick.
        scheduleHighlightRefresh()
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
        applyHighlights(force: true)
        renderMarks()
        scheduleSpellCheck()
    }

    // MARK: Spell check (prose only)

    /// Ranges the last spell pass flagged, painted with the overlays.
    private var spellingRanges: [NSRange] = []
    private var spellTimer: Timer?

    /// The configured spelling dictionaries ("auto", "en_US", … — empty
    /// means off), set from `apply(settings:)`. Several can apply at
    /// once, which is what a bilingual document needs.
    private var appliedSpellLanguages: [String] = []

    /// Words to accept whatever the dictionaries say, folded for
    /// comparison. A personal list is an allowlist someone typed, not a
    /// dictionary with rules about capitalization, so case is ignored.
    private var acceptedWords: Set<String> = []

    /// Words waved through for this session only — the middle ground
    /// between "this is a word" and "fix it".
    private var ignoredWords: Set<String> = []

    /// Applies the settings' choice and re-checks (or clears the marks).
    func applySpellSettings(languages: [String], words: [String]) {
        let folded = Set(words.map { $0.lowercased() })
        guard languages != appliedSpellLanguages || folded != acceptedWords else { return }
        appliedSpellLanguages = languages
        acceptedWords = folded
        if languages.isEmpty {
            spellTimer?.invalidate()
            if !spellingRanges.isEmpty {
                spellingRanges = []
                renderMarks()
            }
        } else {
            scheduleSpellCheck()
        }
    }

    /// Accepts a word for the rest of the session, without saving it.
    func ignoreWordForSession(_ word: String) {
        ignoredWords.insert(word)
        runSpellPass()
    }

    /// Debounced pass so typing does not spell-check every keystroke.
    private func scheduleSpellCheck() {
        guard !appliedSpellLanguages.isEmpty else { return }
        spellTimer?.invalidate()
        spellTimer = Timer.scheduledTimer(withTimeInterval: 0.7, repeats: false) {
            [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.runSpellPass() }
            }
        }
    }

    /// Where prose lives in this document: everywhere for languages
    /// that are prose (markdown, git commits, plain text), only inside
    /// comments for code — spell checking identifiers helps no one.
    private func proseRanges(in text: NSString) -> [NSRange] {
        let language = coreDocument.languageName
        if language == nil || language == "markdown" || language == "gitcommit" {
            let whole = NSRange(location: 0, length: text.length)
            guard language == "markdown" else { return [whole] }
            // Hugo posts carry structured data and template calls in
            // among the prose; a slug is not a misspelling.
            let skip = CoreWorkspace.hugoNonProseRanges(in: text as String)
            return skip.isEmpty ? [whole] : Self.ranges(of: whole, excluding: skip)
        }
        guard text.length <= Self.highlightSizeCap else { return [] }
        // Asked by name: style ids are positions in an alphabetical
        // table and move whenever a capture is added.
        return coreDocument.highlights(in: NSRange(location: 0, length: text.length))
            .filter { span in
                CoreTheme.commentStyleID.map { span.styleIndex == $0 } ?? false
            }
            .map(\.range)
    }

    /// `whole` with every excluded range cut out of it, in order.
    static func ranges(of whole: NSRange, excluding excluded: [NSRange]) -> [NSRange] {
        var kept: [NSRange] = []
        var cursor = whole.location
        for range in excluded.sorted(by: { $0.location < $1.location }) {
            if range.location > cursor {
                kept.append(NSRange(location: cursor, length: range.location - cursor))
            }
            cursor = max(cursor, NSMaxRange(range))
        }
        if cursor < NSMaxRange(whole) {
            kept.append(NSRange(location: cursor, length: NSMaxRange(whole) - cursor))
        }
        return kept
    }

    private func runSpellPass() {
        guard let textView, !appliedSpellLanguages.isEmpty else { return }
        let text = textView.string as NSString
        let prose = proseRanges(in: text).filter { $0.length > 0 }

        // Each dictionary gets its own pass, and only a word that every
        // one of them rejects is a misspelling: a Spanish word in an
        // otherwise English document is not a mistake when the user
        // asked for both dictionaries.
        var found: Set<NSRange>?
        for language in appliedSpellLanguages {
            let flagged = misspellings(in: text, prose: prose, language: language)
            found = found.map { $0.intersection(flagged) } ?? flagged
            // Nothing left to narrow, and each pass costs a full walk.
            if found?.isEmpty == true { break }
        }

        let accepted = acceptedWords
        let ignored = ignoredWords
        let ranges = (found ?? [])
            .filter { range in
                let word = text.substring(with: range)
                return !accepted.contains(word.lowercased()) && !ignored.contains(word)
            }
            .sorted { $0.location < $1.location }
        if ranges != spellingRanges {
            spellingRanges = ranges
            renderMarks()
        }
    }

    /// Every range one dictionary flags across the prose of a document.
    private func misspellings(
        in text: NSString,
        prose: [NSRange],
        language: String
    ) -> Set<NSRange> {
        let checker = NSSpellChecker.shared
        if language == "auto" {
            checker.automaticallyIdentifiesLanguages = true
        } else {
            checker.automaticallyIdentifiesLanguages = false
            checker.setLanguage(language)
        }
        var found: Set<NSRange> = []
        for range in prose {
            let segment = text.substring(with: range)
            var offset = 0
            while offset < range.length {
                let miss = checker.checkSpelling(
                    of: segment,
                    startingAt: offset,
                    language: language == "auto" ? nil : language,
                    wrap: false,
                    inSpellDocumentWithTag: 0,
                    wordCount: nil
                )
                guard miss.location != NSNotFound, miss.length > 0 else { break }
                found.insert(
                    NSRange(location: range.location + miss.location, length: miss.length))
                offset = NSMaxRange(miss)
            }
        }
        return found
    }

    /// The misspelled range containing `location`, if any — what a
    /// context menu needs to know before it can offer replacements.
    func misspelledRange(at location: Int) -> NSRange? {
        spellingRanges.first { NSLocationInRange(location, $0) }
    }

    /// What a context-menu command is about: the character clicked and
    /// the command to run there. Carried on the item, so the action
    /// does not have to ask where the pointer was.
    final class ContextCommand: NSObject {
        let index: Int
        let selector: Selector

        init(index: Int, selector: Selector) {
            self.index = index
            self.selector = selector
        }
    }

    /// What a spelling menu item is about, carried on the item itself so
    /// the action does not have to ask where the pointer was.
    final class SpellingFix: NSObject {
        let range: NSRange
        let word: String
        let replacement: String?

        init(range: NSRange, word: String, replacement: String?) {
            self.range = range
            self.word = word
            self.replacement = replacement
        }
    }

    @objc func replaceMisspelling(_ sender: NSMenuItem) {
        guard let fix = sender.representedObject as? SpellingFix,
            let replacement = fix.replacement,
            let textView
        else { return }
        let text = textView.string as NSString
        // The document may have moved on between the right-click and the
        // choice; replacing whatever now sits at those offsets would
        // corrupt it.
        guard NSMaxRange(fix.range) <= text.length,
            text.substring(with: fix.range) == fix.word
        else { return }
        guard textView.shouldChangeText(in: fix.range, replacementString: replacement) else {
            return
        }
        textView.textStorage?.replaceCharacters(in: fix.range, with: replacement)
        textView.didChangeText()
    }

    @objc func addMisspellingToDictionary(_ sender: NSMenuItem) {
        guard let fix = sender.representedObject as? SpellingFix else { return }
        (NSApp.delegate as? AppDelegate)?.addSpellWord(fix.word)
    }

    @objc func ignoreMisspelling(_ sender: NSMenuItem) {
        guard let fix = sender.representedObject as? SpellingFix else { return }
        ignoreWordForSession(fix.word)
    }

    /// Where the selected word appears in the visible text.
    private var occurrenceRanges: [NSRange] = []

    /// Recomputes the occurrence marks for the current selection.
    ///
    /// Only a selection that is exactly one word marks anything; the
    /// core decides that, and answers with nothing otherwise. The
    /// search covers the painted stretch, so a long document costs
    /// what a short one does.
    private func refreshOccurrences() {
        let previous = occurrenceRanges
        occurrenceRanges = []
        defer {
            if previous != occurrenceRanges { renderMarks() }
        }
        guard appliedMarkOccurrences, let textView else { return }
        let selection = textView.selectedRange()
        guard selection.length > 0 else { return }
        let text = textView.string as NSString
        guard let painted = highlightRange()?.painted,
            selection.location >= painted.location,
            NSMaxRange(selection) <= NSMaxRange(painted),
            NSMaxRange(painted) <= text.length
        else { return }
        let visible = text.substring(with: painted)
        let relative = (selection.location - painted.location)..<(
            NSMaxRange(selection) - painted.location)
        occurrenceRanges = CoreOccurrences.marks(
            in: visible, selection: relative, base: painted.location,
            caseSensitive: appliedOccurrencesCaseSensitive,
            wholeWord: appliedOccurrencesWholeWord
        )
        .map { NSRange(location: $0.start, length: $0.end - $0.start) }
        // The selection itself is already marked, by being selected.
        .filter { $0 != selection }
    }

    /// Clears the occurrence marks. Escape says "I am done looking".
    func clearOccurrences() {
        guard !occurrenceRanges.isEmpty else { return }
        occurrenceRanges = []
        renderMarks()
    }

    /// Marks each finding with a tinted background: red for errors,
    /// orange for warnings, blue otherwise — each misspelling from the
    /// prose spell pass in purple, and the other places the selected
    /// word appears in grey, so the three never read as one. They share
    /// one pass because they share one attribute: the background tint
    /// is what TextKit 2 renders from this layer, and a second pass
    /// would clear the first one's marks.
    private func renderMarks() {
        guard let textView, let layoutManager = textView.textLayoutManager,
            let contentManager = layoutManager.textContentManager
        else { return }
        let documentRange = layoutManager.documentRange
        for target in paintTargets {
            target.removeRenderingAttribute(.underlineStyle, for: documentRange)
            target.removeRenderingAttribute(.underlineColor, for: documentRange)
            target.removeRenderingAttribute(.backgroundColor, for: documentRange)
        }

        let text = textView.string as NSString
        for occurrence in occurrenceRanges {
            guard NSMaxRange(occurrence) <= text.length,
                let start = contentManager.location(
                    documentRange.location, offsetBy: occurrence.location),
                let end = contentManager.location(start, offsetBy: occurrence.length),
                let textRange = NSTextRange(location: start, end: end)
            else { continue }
            for target in paintTargets {
                target.addRenderingAttribute(
                    .backgroundColor,
                    value: NSColor.systemGray.withAlphaComponent(0.30), for: textRange)
            }
        }
        for spelling in spellingRanges {
            guard NSMaxRange(spelling) <= text.length,
                let start = contentManager.location(
                    documentRange.location, offsetBy: spelling.location),
                let end = contentManager.location(start, offsetBy: spelling.length),
                let textRange = NSTextRange(location: start, end: end)
            else { continue }
            for target in paintTargets {
                target.addRenderingAttribute(
                    .backgroundColor,
                    value: NSColor.systemPurple.withAlphaComponent(0.18), for: textRange)
            }
        }
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
            for target in paintTargets {
                target.addRenderingAttribute(
                    .underlineStyle, value: NSUnderlineStyle.thick.rawValue, for: textRange)
                target.addRenderingAttribute(.underlineColor, value: color, for: textRange)
                target.addRenderingAttribute(
                    .backgroundColor, value: color.withAlphaComponent(0.15), for: textRange)
            }
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

    /// Set while a viewport recolour is already queued.
    private var highlightRefreshPending = false

    /// Repaints the visible stretch once the current runloop turn ends.
    private func scheduleHighlightRefresh() {
        guard !highlightRefreshPending else { return }
        highlightRefreshPending = true
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.highlightRefreshPending = false
                guard self.applyHighlights(force: false) else { return }
                // Scrolling brings fresh text into the painted stretch,
                // where the selected word may also appear.
                self.refreshOccurrences()
                self.renderMarks()
            }
        }
    }

    /// The stretch of document worth colouring: what the reader can
    /// see, plus a margin so a flick of the scroll wheel lands on
    /// coloured text rather than black. Nil means "colour everything",
    /// which is what a document smaller than one margin deserves.
    private func highlightRange() -> (viewport: NSRange, painted: NSRange)? {
        let length = coreDocument.lengthInUTF16
        guard length > Self.viewportMargin * 2 else {
            guard length > 0 else { return nil }
            let whole = NSRange(location: 0, length: length)
            return (whole, whole)
        }
        guard let textView, let scrollView = textView.enclosingScrollView,
            let layoutManager = textView.textLayoutManager,
            let contentManager = layoutManager.textContentManager
        else {
            let head = NSRange(location: 0, length: min(length, Self.viewportMargin * 2))
            return (head, head)
        }

        // The viewport controller knows exactly what is laid out; fall
        // back to the caret when it has not settled yet (first paint).
        var start = 0
        var end = 0
        if let viewport = layoutManager.textViewportLayoutController.viewportRange {
            start = contentManager.offset(
                from: contentManager.documentRange.location, to: viewport.location)
            end = contentManager.offset(
                from: contentManager.documentRange.location, to: viewport.endLocation)
        } else {
            let caret = min(textView.selectedRange().location, length)
            start = caret
            end = caret
        }
        _ = scrollView
        let from = max(0, start - Self.viewportMargin)
        let to = min(length, end + Self.viewportMargin)
        guard to > from else { return nil }
        return (
            NSRange(location: start, length: max(0, end - start)),
            NSRange(location: from, length: to - from)
        )
    }

    /// What the last pass painted, and how long the document was then.
    private var paintedRange: NSRange?
    private var paintedLength: Int?

    /// Whether a scroll has to repaint at all.
    ///
    /// The margin around the viewport exists so that scrolling can move
    /// most of a screen before anything needs recolouring. Repainting
    /// on every scroll turn instead does the whole job again for a few
    /// pixels of movement — several milliseconds of rendering
    /// attributes and font runs per runloop turn, which is what the
    /// stutter was.
    ///
    /// A document whose length changed is repainted regardless: the
    /// offsets a remembered range is expressed in have moved.
    static func shouldRepaint(
        viewport: NSRange, painted: NSRange?, documentLength: Int, paintedLength: Int?
    ) -> Bool {
        guard let painted, paintedLength == documentLength else { return true }
        return viewport.location < painted.location
            || NSMaxRange(viewport) > NSMaxRange(painted)
    }

    /// Paints the core's styled spans over the visible stretch: colour
    /// as TextKit 2 rendering attributes (which never invalidate
    /// layout), and — only for themes that ask for them — bold and
    /// italic as storage attributes, since a font is layout and cannot
    /// ride the rendering layer.
    ///
    /// Scoping to the viewport is what lets a large file be coloured at
    /// all: colouring whole documents meant a hard cap past which text
    /// arrived with no colour whatsoever.
    @discardableResult
    private func applyHighlights(force: Bool) -> Bool {
        guard let textView,
            let layoutManager = textView.textLayoutManager,
            let contentManager = layoutManager.textContentManager
        else { return false }
        guard let (viewport, painted) = highlightRange(), painted.length > 0 else {
            paintedRange = nil
            paintedLength = nil
            return false
        }
        let length = coreDocument.lengthInUTF16
        guard force
            || Self.shouldRepaint(
                viewport: viewport, painted: paintedRange,
                documentLength: length, paintedLength: paintedLength)
        else { return false }
        // A scroll only exposes text at one end. Repainting the whole
        // stretch for it costs a colour per span over sixteen thousand
        // units — about 3.6 ms, which arrives as a catch in an inertial
        // scroll. Painting the exposed part instead makes a wheel notch
        // cost a wheel notch.
        //
        // A forced pass — an edit, a theme change — repaints the lot,
        // because what changed is not confined to an edge.
        let documentRange = layoutManager.documentRange
        let previous = paintedRange
        let incremental =
            !force && previous != nil && paintedLength == length
            && NSIntersectionRange(previous!, painted).length > 0
        paintedRange = painted
        paintedLength = length

        var toPaint: [NSRange] = [painted]
        if incremental, let previous {
            toPaint = []
            if painted.location < previous.location {
                toPaint.append(
                    NSRange(
                        location: painted.location,
                        length: previous.location - painted.location))
            }
            if NSMaxRange(painted) > NSMaxRange(previous) {
                toPaint.append(
                    NSRange(
                        location: NSMaxRange(previous),
                        length: NSMaxRange(painted) - NSMaxRange(previous)))
            }
            if toPaint.isEmpty { return true }
        } else {
            for target in paintTargets {
                target.removeRenderingAttribute(.foregroundColor, for: documentRange)
            }
        }

        let spans = toPaint.flatMap { coreDocument.highlights(in: $0) }
        guard !spans.isEmpty else { return true }

        let darkAppearance =
            (window?.effectiveAppearance ?? NSApp.effectiveAppearance)
                .bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        let wantsTraits = HighlightPalette.hasTypographicStyles
        var wantedFonts: [NSRange: NSFont] = [:]
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
            for target in paintTargets {
                target.setRenderingAttributes([.foregroundColor: color], for: range)
            }

            guard wantsTraits, let storage = textView.textStorage else { continue }
            let traits = HighlightPalette.traits(forStyle: span.styleIndex)
            guard traits.bold || traits.italic else { continue }
            let clamped = NSIntersectionRange(
                span.range, NSRange(location: 0, length: storage.length))
            guard clamped.length > 0 else { continue }
            wantedFonts[clamped] = Self.font(
                appliedFont, bold: traits.bold, italic: traits.italic)
        }
        if wantsTraits, let storage = textView.textStorage {
            for stretch in toPaint {
                _ = Self.applyTraitFonts(
                    wantedFonts, over: stretch, in: storage, plain: appliedFont)
            }
        }
        return true
    }

    /// Writes the theme's bold and italic into the text storage, and
    /// writes **only what differs**.
    ///
    /// Colour rides TextKit 2's rendering attributes, which never
    /// invalidate layout. A font cannot: it is a storage attribute, and
    /// setting one invalidates layout over its range whether or not the
    /// value changed. Starting each pass by resetting the whole painted
    /// stretch to the plain font therefore invalidated some sixteen
    /// thousand units of layout on every keystroke, which is enough to
    /// move the visible area out from under the caret.
    ///
    /// Reading attributes costs nothing, so this reads first: runs that
    /// already carry the font they should are left alone, and text that
    /// merely shifted keeps the font that shifted with it.
    /// Returns how many units it wrote, which is what the smoke test
    /// asserts: a second pass over unchanged text must write nothing.
    @discardableResult
    static func applyTraitFonts(
        _ wanted: [NSRange: NSFont], over painted: NSRange, in storage: NSTextStorage,
        plain: NSFont
    ) -> Int {
        let bounds = NSIntersectionRange(painted, NSRange(location: 0, length: storage.length))
        guard bounds.length > 0 else { return 0 }
        var written = 0

        // Which font each position should end up with: plain unless a
        // span said otherwise.
        var desired = [NSFont?](repeating: nil, count: bounds.length)
        for (range, font) in wanted {
            let inside = NSIntersectionRange(range, bounds)
            guard inside.length > 0 else { continue }
            let start = inside.location - bounds.location
            for offset in start..<(start + inside.length) {
                desired[offset] = font
            }
        }

        // Walk what is there and write only where the two disagree,
        // coalescing neighbours so one changed word costs one write.
        var pending: (range: NSRange, font: NSFont)?
        func flush() {
            if let open = pending {
                storage.addAttribute(.font, value: open.font, range: open.range)
                written += open.range.length
            }
            pending = nil
        }
        storage.enumerateAttribute(.font, in: bounds, options: []) { value, range, _ in
            let current = value as? NSFont
            var index = range.location
            while index < NSMaxRange(range) {
                let want = desired[index - bounds.location] ?? plain
                if current == want {
                    flush()
                    index += 1
                    continue
                }
                if var open = pending, open.font == want, NSMaxRange(open.range) == index {
                    open.range.length += 1
                    pending = open
                } else {
                    flush()
                    pending = (NSRange(location: index, length: 1), want)
                }
                index += 1
            }
        }
        flush()
        return written
    }

    /// The editor font with traits applied. Monospaced families keep
    /// their advance width across weights, so this does not reflow the
    /// document — it just makes a comment look like a comment.
    private static func font(_ base: NSFont, bold: Bool, italic: Bool) -> NSFont {
        var traits: NSFontDescriptor.SymbolicTraits = []
        if bold { traits.insert(.bold) }
        if italic { traits.insert(.italic) }
        let descriptor = base.fontDescriptor.withSymbolicTraits(traits)
        return NSFont(descriptor: descriptor, size: base.pointSize) ?? base
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
                t("The file was modified by another program, and you have unsaved changes. Reloading will discard your changes (one Undo brings them back).")
            alert.addButton(withTitle: t("Keep My Changes"))
            alert.addButton(withTitle: t("Reload From Disk"))
            if alert.runModal() == .alertSecondButtonReturn {
                reloadFromDisk()
            }
        } else {
            // Clean documents follow the disk silently.
            reloadFromDisk()
        }
    }

    /// File → Revert to Saved: throw away the buffer and take the disk's
    /// word for it — the manual escape hatch for the rare external change
    /// the watcher misses (delete-and-replace flows like git checkout).
    @objc func revertToSaved(_ sender: Any?) {
        guard coreDocument.path != nil else { return }
        if coreDocument.isDirty {
            let alert = NSAlert()
            alert.alertStyle = .warning
            alert.messageText = t("Revert to the saved version?")
            alert.informativeText =
                t("Your unsaved changes will be replaced by the file on disk (one Undo brings them back).")
            alert.addButton(withTitle: t("Revert"))
            alert.addButton(withTitle: t("Cancel"))
            guard alert.runModal() == .alertFirstButtonReturn else { return }
        }
        reloadFromDisk()
    }

    private func reloadFromDisk() {
        let selection = textView?.selectedRange()
        do {
            guard let edit = try coreDocument.reload() else {
                // The buffer already matched the disk, but a commit or
                // a branch switch moves what git compares against.
                refreshChangeMarks()
                return
            }
            replay([edit])
            refreshChangeMarks()
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

    /// The text view, for app-level interactions (⌘-click navigation).
    var editorTextView: NSTextView? { textView }

    /// How much beyond the viewport gets coloured, in UTF-16 units:
    /// enough that a scroll flick lands on coloured text, small enough
    /// that a megabyte file costs the same as a small one.
    private static let viewportMargin = 8_000

    /// The editor font as configured, so highlight traits derive from
    /// it rather than from whatever a span inherited.
    private var appliedFont: NSFont = .monospacedSystemFont(ofSize: 13, weight: .regular)

    /// The configured tab width, remembered for formatting requests.
    private var appliedTabWidth = 4

    /// Whether mouse-rest hover documentation is on. The deliberate
    /// show-at-caret command ignores this.
    private var appliedHoverDocs = true
    /// Whether a file stays open when the window showing it closes.
    private var appliedKeepBuffers = false
    /// The language this file was told it is, when its name does not
    /// say. Recorded with the project rather than with the settings.
    private(set) var languageOverride: String?
    /// Whether the project record has been read for this file, so that
    /// writing one back does not invent a record for a file nobody has
    /// said anything about.
    private var adoptedProjectState = false

    /// Whether selecting a word marks its other occurrences, and how
    /// those are matched.
    private var appliedMarkOccurrences = true
    private var appliedOccurrencesCaseSensitive = true
    private var appliedOccurrencesWholeWord = true

    /// Applies configuration-derived settings to the view: the font, and
    /// tab stops sized to the configured width in that font.
    func apply(settings: EditorSettings) {
        appliedSettings = settings
        defer {
            for view in views {
                view.contextStrip.invalidateText()
                updateContextStrip(for: view)
            }
        }
        appliedFont = settings.font
        appliedTabWidth = settings.tabWidth
        appliedHoverDocs = settings.hoverDocs
        appliedKeepBuffers = settings.keepBuffers
        appliedMarkOccurrences = settings.markOccurrences
        appliedOccurrencesCaseSensitive = settings.occurrencesCaseSensitive
        appliedOccurrencesWholeWord = settings.occurrencesWholeWord
        if !settings.markOccurrences, !occurrenceRanges.isEmpty {
            occurrenceRanges = []
            renderMarks()
        }
        applySpellSettings(languages: settings.spellLanguages, words: settings.spellWords)
        applyAutosave(seconds: settings.autosaveSeconds)
        if !settings.hoverDocs {
            hoverTimer?.invalidate()
            hoverPopover?.close()
            hoverPopover = nil
        }
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
            // That reset just erased the bold and italic the highlight
            // pass had applied — colour lives in the rendering layer
            // and survives, fonts do not. Repaint them.
            refreshDecorations()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("DocumentController is created in code")
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
    /// What this document is called on the tab and, while it has the
    /// keyboard, in the title bar.
    var chromeTitle: String {
        guard let path = coreDocument.path else { return t("Untitled") }
        return displayTitle ?? URL(fileURLWithPath: path).lastPathComponent
    }

    /// The document's facts, for the window's subtitle.
    /// What the status bar says about this document right now.
    var statusInfo: StatusBar.Info {
        var info = StatusBar.Info()
        if let view = focusedView {
            let caret = view.textView.selectedRange().location
            let line = view.gutter.lineIndex(forOffset: caret)
            info.line = line + 1
            info.column = caret - view.gutter.lineStart(ofLine: line) + 1
        }
        info.tabWidth = appliedTabWidth
        let stored = coreDocument.path.map {
            (NSApp.delegate as? AppDelegate)?.fileOverride(path: $0)
                ?? CoreConfig.FileOverride()
        }
        // The override answers when it says; the file itself otherwise,
        // the way formatting already reads it.
        info.usesTabs =
            stored?.spaces.map { !$0 }
            ?? {
                let text = textView?.string ?? ""
                return text.contains("\n\t") || text.hasPrefix("\t")
            }()
        info.language = coreDocument.languageName
        info.encoding = coreDocument.encodingName
        return info
    }

    var chromeSubtitle: String {
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
        return subtitle
    }

    func updateChrome() {
        workbench?.refreshChrome(for: self)
        publishSidebarState()
    }

    /// Publishes title/dirty/path changes to the sidebar — but only actual
    /// changes, so per-keystroke chrome updates stay cheap.
    private func publishSidebarState() {
        let state = (chromeTitle, coreDocument.isDirty, coreDocument.path)
        guard state != publishedState else { return }
        if state.2 != publishedState.2 {
            projectRoot = state.2.flatMap(resolveProjectRoot)
            // Off the current turn: chrome is updated from places AppKit
            // reaches while it is laying out, and the navigator reads
            // this as it draws. SwiftUI ends the process over a value
            // set while its view graph is updating.
            let root = projectRoot
            let context = sidebarContext
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    if context?.projectRoot != root { context?.projectRoot = root }
                }
            }
        }
        publishedState = state
        NotificationCenter.default.post(name: .textchumDocumentsChanged, object: self)
    }

    /// Recomputes the project root under the current workspace settings
    /// (called when those settings change).
    func refreshProjectRoot() {
        projectRoot = coreDocument.path.flatMap(resolveProjectRoot)
        if sidebarContext?.projectRoot != projectRoot {
            sidebarContext?.projectRoot = projectRoot
        }
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
    private func replay(_ edits: [CoreDocument.AppliedEdit], movingCaret: Bool = true) {
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
        // case the document shrank, and reveal it. Mirroring a snippet's
        // linked stops passes false: the caret belongs where the user is
        // typing, not where the copy went.
        if movingCaret {
            selectionChangeIsFromEditing = true
            caret = min(caret, (textView.string as NSString).length)
            textView.setSelectedRange(NSRange(location: caret, length: 0))
            textView.scrollRangeToVisible(NSRange(location: caret, length: 0))
        }

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
    ///
    /// A snippet is expanded by the core first, and the plain text that
    /// comes back is what the view inserts; the core is then told where
    /// it landed, which starts the tabstop session Tab walks.
    private func accept(completion item: CompletionPopup.Item) {
        guard let textView else { return }
        let replacementRange =
            currentWordPrefix()?.range
            ?? NSRange(location: textView.selectedRange().location, length: 0)
        guard item.isSnippet else {
            coreDocument.cancelSnippet()
            textView.insertText(item.insertText, replacementRange: replacementRange)
            return
        }
        let expanded = coreDocument.expandSnippet(
            item.insertText, at: replacementRange.location)
        coreDocument.cancelSnippet()
        textView.insertText(expanded, replacementRange: replacementRange)
        guard let selection = coreDocument.beginSnippet(at: replacementRange.location),
            NSMaxRange(selection) <= (textView.string as NSString).length
        else { return }
        selectionChangeIsFromEditing = true
        textView.setSelectedRange(selection)
    }

    // MARK: Snippet tabstops

    /// What a key means while a snippet is being filled in. Nil leaves
    /// the key to the text view, which is most of them: a snippet takes
    /// three keys and gives the rest back.
    enum SnippetKey {
        case nextStop
        case previousStop
        case cancel
    }

    /// The snippet meaning of a command selector, if it has one.
    static func snippetKey(for selector: Selector) -> SnippetKey? {
        switch selector {
        case #selector(NSResponder.insertTab(_:)): return .nextStop
        case #selector(NSResponder.insertBacktab(_:)): return .previousStop
        case #selector(NSResponder.cancelOperation(_:)): return .cancel
        default: return nil
        }
    }

    /// Moves to the next tabstop, or back to the previous one, and
    /// selects its placeholder. Returns false when no snippet is being
    /// filled in, so Tab stays Tab.
    @discardableResult
    private func moveThroughSnippet(forward: Bool) -> Bool {
        guard let textView, coreDocument.isSnippetActive,
            let selection = coreDocument.advanceSnippet(forward: forward),
            NSMaxRange(selection) <= (textView.string as NSString).length
        else { return false }
        selectionChangeIsFromEditing = true
        textView.setSelectedRange(selection)
        textView.scrollRangeToVisible(selection)
        return true
    }

    /// Copies the tabstop just typed in to the other places carrying the
    /// same number. The caret is put back where it was, shifted by
    /// whatever the mirroring inserted ahead of it, so typing a linked
    /// placeholder feels like typing anything else.
    private func mirrorSnippetStops() {
        guard coreDocument.isSnippetActive, let textView else { return }
        let edits = coreDocument.syncSnippet()
        guard !edits.isEmpty else { return }
        var caret = textView.selectedRange()
        for edit in edits where NSMaxRange(edit.range) <= caret.location {
            caret.location += (edit.replacement as NSString).length - edit.range.length
        }
        replay(edits, movingCaret: false)
        caret.location = min(caret.location, (textView.string as NSString).length)
        caret.length = min(caret.length, (textView.string as NSString).length - caret.location)
        selectionChangeIsFromEditing = true
        textView.setSelectedRange(caret)
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

    // MARK: Save preprocessors

    /// What running the configured chain over the buffer produced.
    private enum PreprocessOutcome {
        case clean
        case failed(Preprocessors.Failure)
    }

    /// Runs the configured save-preprocessor chain (ruff, black, gofmt,
    /// prettier…) over the buffer and applies the result through the
    /// normal text-view path — the core stays synchronized and undo
    /// works. No chain configured counts as clean.
    private func preprocessBuffer() -> PreprocessOutcome {
        guard let textView, let language = coreDocument.languageName else { return .clean }
        let commands = preprocessorCommands(projectRoot, language)
        guard !commands.isEmpty else { return .clean }
        // Untitled documents still offer a name for {filename}: the
        // language's extension is what stdin-filepath tools care about.
        let documentPath =
            coreDocument.path
            ?? CoreLanguages.all
            .first { $0.name == language && !$0.fileExtension.isEmpty }
            .map { "Untitled.\($0.fileExtension)" }
        switch Preprocessors.run(
            commands: commands, on: textView.string, in: projectRoot,
            documentPath: documentPath)
        {
        case .success(let output):
            applyWholeDocument(output)
            return .clean
        case .failure(let failure):
            return .failed(failure)
        }
    }

    /// Replaces the buffer with `new` as one minimal edit: the common
    /// prefix and suffix stay untouched, so the caret and scroll
    /// position survive a formatter that only changed a few lines.
    private func applyWholeDocument(_ new: String) {
        guard let textView else { return }
        let old = textView.string
        guard new != old else { return }
        let oldChars = Array(old.utf16)
        let newChars = Array(new.utf16)
        var prefix = 0
        while prefix < min(oldChars.count, newChars.count),
            oldChars[prefix] == newChars[prefix]
        {
            prefix += 1
        }
        var suffix = 0
        while suffix < min(oldChars.count, newChars.count) - prefix,
            oldChars[oldChars.count - 1 - suffix] == newChars[newChars.count - 1 - suffix]
        {
            suffix += 1
        }
        // Never split a surrogate pair at either boundary.
        if prefix > 0, prefix < oldChars.count, UTF16.isTrailSurrogate(oldChars[prefix]) {
            prefix -= 1
        }
        if suffix > 0, suffix < newChars.count,
            UTF16.isLeadSurrogate(newChars[newChars.count - suffix])
        {
            suffix -= 1
        }
        let range = NSRange(location: prefix, length: oldChars.count - prefix - suffix)
        let replacement = String(
            decoding: newChars[prefix..<(newChars.count - suffix)], as: UTF16.self)
        textView.insertText(replacement, replacementRange: range)
    }

    /// Edit → Run Save Preprocessors: format the buffer through the
    /// configured chain without saving.
    @objc func runPreprocessors(_ sender: Any?) {
        guard let language = coreDocument.languageName,
            !preprocessorCommands(projectRoot, language).isEmpty
        else {
            NSSound.beep()
            return
        }
        if case .failed(let failure) = preprocessBuffer() {
            presentError(
                "Preprocessor failed: \(failure.command)", details: failure.details)
        }
    }

    /// The pre-save half of the flow: runs the chain, and on failure
    /// lets the user choose between saving the unprocessed buffer and
    /// not saving at all. Returns whether the save should proceed.
    private func preprocessBeforeSave() -> Bool {
        switch preprocessBuffer() {
        case .clean:
            return true
        case .failed(let failure):
            let alert = NSAlert()
            alert.alertStyle = .warning
            alert.messageText = "Save preprocessor failed: \(failure.command)"
            alert.informativeText = failure.details
            alert.addButton(withTitle: t("Save Without Preprocessing"))
            alert.addButton(withTitle: t("Cancel"))
            return alert.runModal() == .alertFirstButtonReturn
        }
    }

    // MARK: Saving

    /// Saves, asking for a location if the document has none. Returns
    /// whether the document ended up saved.
    @discardableResult
    func saveInteractively() -> Bool {
        guard coreDocument.path != nil else { return saveAsInteractively() }
        guard preprocessBeforeSave() else { return false }
        do {
            try coreDocument.save()
            noteOwnSave()
            updateChrome()
            refreshChangeMarks()
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
        if coreDocument.path == nil, let suggestedSaveDirectory {
            panel.directoryURL = suggestedSaveDirectory
        }
        // The bare filename, not the window title — a disambiguated
        // title carries path components no filename should. An untitled
        // document that already speaks a language suggests its
        // extension.
        let untitledName =
            CoreLanguages.all
            .first { $0.name == coreDocument.languageName && !$0.fileExtension.isEmpty }
            .map { "Untitled.\($0.fileExtension)" } ?? "Untitled.txt"
        panel.nameFieldStringValue =
            coreDocument.path.map { ($0 as NSString).lastPathComponent } ?? untitledName
        guard panel.runModal() == .OK, let url = panel.url else { return false }
        guard preprocessBeforeSave() else { return false }
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

    // MARK: Autosave

    private var autosaveTimer: Timer?
    private var autosaveSeconds: UInt32 = 0

    func applyAutosave(seconds: UInt32) {
        autosaveSeconds = seconds
        if seconds == 0 {
            autosaveTimer?.invalidate()
            autosaveTimer = nil
        }
    }

    /// Restarts the autosave clock. Counting from the last keystroke
    /// rather than on a fixed interval means the save happens once the
    /// typing stops, not in the middle of a sentence.
    private func scheduleAutosave() {
        guard autosaveSeconds > 0, coreDocument.path != nil else { return }
        autosaveTimer?.invalidate()
        autosaveTimer = Timer.scheduledTimer(
            withTimeInterval: TimeInterval(autosaveSeconds),
            repeats: false
        ) { [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.autosave() }
            }
        }
    }

    /// Saves without the interactive parts: no save panel (there is a
    /// path, or this never runs) and no preprocessor chain. A formatter
    /// reflowing the line being typed is not a favour; explicit saves
    /// remain the place for that.
    private func autosave() {
        autosaveTimer = nil
        guard coreDocument.path != nil, coreDocument.isDirty else { return }
        do {
            try coreDocument.save()
            noteOwnSave()
            updateChrome()
        } catch {
            // Not quiet: this is the case where the work is not where
            // the user believes it is.
            presentError("Autosave could not write the document.", details: "\(error)")
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

extension DocumentController: NSTextContentStorageDelegate {
    /// What a paragraph is, as far as layout is concerned.
    ///
    /// The line that opens a fold gains an ellipsis; the lines it hides
    /// are handed over as a bare separator at a hundredth of a point,
    /// which is what makes them take no room. Everything else is left
    /// alone, and nil means "as it is in the document".
    func textContentStorage(
        _ textContentStorage: NSTextContentStorage,
        textParagraphWith range: NSRange
    ) -> NSTextParagraph? {
        let spans = foldSpans()
        guard !spans.isEmpty, let storage = textContentStorage.textStorage else { return nil }
        if let fold = spans.first(where: { $0.opening.location == range.location }) {
            _ = fold
            let shown = NSMutableAttributedString(
                attributedString: storage.attributedSubstring(from: range))
            guard shown.length > 0 else { return nil }
            let attributes = storage.attributes(at: range.location, effectiveRange: nil)
            // Before the separator, not after it: after it is a new line.
            shown.replaceCharacters(
                in: NSRange(location: shown.length - 1, length: 1),
                with: NSAttributedString(string: " ⋯\n", attributes: attributes))
            return NSTextParagraph(attributedString: shown)
        }
        guard spans.contains(where: { NSLocationInRange(range.location, $0.hidden) }) else {
            return nil
        }
        let style = NSMutableParagraphStyle()
        style.maximumLineHeight = 0.01
        style.minimumLineHeight = 0.01
        style.lineSpacing = 0
        style.paragraphSpacing = 0
        style.paragraphSpacingBefore = 0
        // A paragraph with no separator at all crashes the layout:
        // NSTextParagraph reads the last character to find one.
        return NSTextParagraph(
            attributedString: NSAttributedString(
                string: "\n",
                attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 0.01, weight: .regular),
                    .paragraphStyle: style,
                ]))
    }
}

extension DocumentController: WKNavigationDelegate {
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        // The template page is ready; push the first render.
        updatePreview()
    }

    /// The preview shows this document and never anything else.
    ///
    /// A link clicked in it used to navigate the pane, which has no
    /// back button, no history and no address bar: the document was
    /// gone until it was edited or the pane was closed and reopened.
    /// A link goes to the browser, which is where a link a reader wants
    /// to follow belongs.
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        guard navigationAction.navigationType == .linkActivated else {
            // The template and the rendered document arrive as content,
            // not as a click.
            decisionHandler(.allow)
            return
        }
        guard let url = navigationAction.request.url else {
            decisionHandler(.cancel)
            return
        }
        // A link into the document itself is a place in this page; the
        // page scrolls to it and stays. The core decides, so both
        // previews treat the same links the same way.
        if CorePreview.isPlaceInPage(here: webView.url?.absoluteString ?? "", target: url.absoluteString) {
            decisionHandler(.allow)
            return
        }
        decisionHandler(.cancel)
        NSWorkspace.shared.open(url)
    }
}

// MARK: - Window lifecycle

extension DocumentController {
    /// The tab is closing: nothing is left running behind it.
    func willClose() {
        recordProjectState()
        completionPopup.dismiss()
        completionTimer?.invalidate()
        lspChangeTimer?.invalidate()
        if let path = lspOpenPath {
            lspApp?.lspDidClose(path: path)
            lspOpenPath = nil
        }
        for view in views { drop(view) }
    }

    /// View → Reveal in Tree: expand the navigator to this document.
    /// File → Get Info (⌘I): what this document is, when its name does
    /// not say. Also reached by clicking the language in the title bar.
    @objc func showFileProperties(_ sender: Any?) {
        guard let path = coreDocument.path else {
            // An untitled document has no path to remember a choice
            // against; New with Format is where its language is set.
            NSSound.beep()
            return
        }
        let delegate = NSApp.delegate as? AppDelegate
        let stored = delegate?.fileOverride(path: path) ?? CoreConfig.FileOverride()
        let facts =
            "\(coreDocument.encodingName) · \(coreDocument.lengthInBytes) bytes\n\(path)"
        FilePropertiesPanel.shared.show(
            over: window,
            title: (path as NSString).lastPathComponent,
            facts: facts,
            detected: CoreLanguages.detected(forPath: path),
            properties: .init(
                language: stored.language,
                tabWidth: stored.tabWidth,
                spaces: stored.spaces
            )
        ) { [weak self] properties in
            MainActor.assumeIsolated {
                guard let self else { return }
                delegate?.setFileOverride(
                    path: path,
                    .init(
                        language: properties.language,
                        tabWidth: properties.tabWidth,
                        spaces: properties.spaces
                    )
                )
                self.applyFileProperties(properties)
            }
        }
    }

    /// Applies a properties change to this document now, so picking a
    /// language recolours the text while the panel is still open.
    func applyFileProperties(_ properties: FilePropertiesPanel.Properties) {
        let language = properties.language
            ?? coreDocument.path.flatMap { CoreLanguages.detected(forPath: $0) }
        _ = coreDocument.setLanguage(language)
        // What a file is, when its name does not say, is data about the
        // file: it goes with the project record.
        languageOverride = properties.language
        adoptedProjectState = true
        recordProjectState()
        if let width = properties.tabWidth, let textView {
            appliedTabWidth = Int(width)
            let style = NSMutableParagraphStyle()
            let font = textView.font ?? .monospacedSystemFont(ofSize: 13, weight: .regular)
            let spaceWidth = (" " as NSString).size(withAttributes: [.font: font]).width
            style.tabStops = []
            style.defaultTabInterval = spaceWidth * CGFloat(width)
            textView.defaultParagraphStyle = style
            textView.textStorage?.addAttribute(
                .paragraphStyle,
                value: style,
                range: NSRange(location: 0, length: textView.textStorage?.length ?? 0)
            )
        }
        refreshDecorations()
        updateChrome()
        // A language change redraws what the pins say, and whether
        // there are any: plain text pins nothing.
        for view in views {
            view.contextStrip.invalidateText()
            updateContextStrip(for: view)
        }
    }

    @objc func revealInTree(_ sender: Any?) {
        guard let path = coreDocument.path else {
            NSSound.beep()
            return
        }
        splitController?.splitViewItems.first?.isCollapsed = false
        revealPathInTree(path)
    }

    /// The follow-the-file half: same reveal, but never uncollapses a
    /// sidebar the user closed.
    func followInTree(_ path: String) {
        guard followEnabled(),
            splitController?.splitViewItems.first?.isCollapsed == false
        else { return }
        revealPathInTree(path)
    }

    /// Standard dirty-document close flow: Save / Cancel / Don't Save.
    /// Whether this document is willing to close, asking about changes
    /// that were never saved.
    func mayClose() -> Bool {
        // Files set to outlive their windows go aside as they are, and
        // are settled when the editor itself closes.
        if appliedKeepBuffers { return true }
        guard coreDocument.isDirty else { return true }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = t("Do you want to save the changes made to {}?", chromeTitle)
        alert.informativeText = t("Your changes will be lost if you don’t save them.")
        alert.addButton(withTitle: t("Save"))
        alert.addButton(withTitle: t("Cancel"))
        alert.addButton(withTitle: t("Don’t Save"))
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

extension DocumentController: NSTextViewDelegate {
    /// The editor's own context menu.
    ///
    /// AppKit's is about text in general — Speech, Substitutions,
    /// Services, autofill — and holds none of the commands that act on
    /// the place that was clicked, which is what a right-click in code
    /// is for. So the menu is built here instead of passed through.
    ///
    /// The commands act on `charIndex`, not on the caret: clicking does
    /// not move the caret, and a menu that answered about somewhere
    /// else would be answering the wrong question. Items that need a
    /// language server are left out when none is running rather than
    /// shown greyed, since a disabled row explains nothing.
    ///
    /// Spelling comes from the prose-scoped pass rather than AppKit's
    /// checker, which is off here. The word's range is captured into
    /// the items instead of looked up when one is chosen: a menu can
    /// stay open while the document moves underneath.
    func textView(
        _ textView: NSTextView,
        menu: NSMenu,
        for event: NSEvent,
        at charIndex: Int
    ) -> NSMenu? {
        let contextMenu = NSMenu()
        if let range = misspelledRange(at: charIndex) {
            addSpellingItems(to: contextMenu, range: range, textView: textView)
            contextMenu.addItem(.separator())
        }
        for (title, selector) in [
            ("Cut", #selector(NSText.cut(_:))),
            ("Copy", #selector(NSText.copy(_:))),
            ("Paste", #selector(NSText.paste(_:))),
        ] {
            // No target: the responder chain reaches the text view,
            // which also decides whether the item is enabled.
            contextMenu.addItem(NSMenuItem(title: title, action: selector, keyEquivalent: ""))
        }

        var commands: [(String, Selector)] = []
        let hasServer = lspApp != nil && lspOpenPath != nil
        if lspOpenPath != nil {
            // Without a server the ctags index may still answer.
            commands.append(("Jump to Definition", #selector(jumpToDefinition(_:))))
        }
        if hasServer {
            commands.append(("Find References", #selector(findReferences(_:))))
            commands.append(("Code Actions…", #selector(showCodeActions(_:))))
            commands.append(("Rename Symbol…", #selector(renameSymbol(_:))))
        }
        if !diagnostics.isEmpty {
            commands.append(("Show Diagnostic for Line", #selector(showDiagnosticAtCaret(_:))))
            commands.append(("Diagnostics…", #selector(showDiagnosticList(_:))))
        }
        if lspOpenPath != nil {
            commands.append(("Blame Line…", #selector(blameLine(_:))))
        }
        // Formatting falls back to the save-preprocessor chain, so it
        // is offered with or without a server.
        commands.append(("Format Document", #selector(formatDocument(_:))))
        commands.append(("File Properties…", #selector(showFileProperties(_:))))

        contextMenu.addItem(.separator())
        for (title, selector) in commands {
            let item = NSMenuItem(
                title: title, action: #selector(runContextCommand(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = ContextCommand(index: charIndex, selector: selector)
            contextMenu.addItem(item)
        }
        return contextMenu
    }

    /// Runs a context-menu command about the character that was
    /// clicked, rather than about the caret.
    @objc private func runContextCommand(_ sender: NSMenuItem) {
        guard let command = sender.representedObject as? ContextCommand else { return }
        contextIndex = command.index
        defer { contextIndex = nil }
        _ = perform(command.selector, with: sender)
    }

    /// The spelling section: suggestions for `range`, and the two ways
    /// to say the word is fine.
    private func addSpellingItems(to spelling: NSMenu, range: NSRange, textView: NSTextView) {
        let word = (textView.string as NSString).substring(with: range)
        let checker = NSSpellChecker.shared
        let guesses =
            checker.guesses(
                forWordRange: range,
                in: textView.string,
                language: appliedSpellLanguages.first == "auto"
                    ? nil : appliedSpellLanguages.first,
                inSpellDocumentWithTag: 0
            ) ?? []
        if guesses.isEmpty {
            // An empty section is a gap the reader has to interpret; a
            // disabled label says why it is empty.
            let none = NSMenuItem(title: t("No Suggestions"), action: nil, keyEquivalent: "")
            none.isEnabled = false
            spelling.addItem(none)
        }
        for guess in guesses.prefix(8) {
            let item = NSMenuItem(
                title: guess,
                action: #selector(replaceMisspelling(_:)),
                keyEquivalent: ""
            )
            item.target = self
            item.representedObject = SpellingFix(range: range, word: word, replacement: guess)
            spelling.addItem(item)
        }
        spelling.addItem(.separator())
        let add = NSMenuItem(
            title: t("Add to Dictionary"),
            action: #selector(addMisspellingToDictionary(_:)),
            keyEquivalent: ""
        )
        add.target = self
        add.representedObject = SpellingFix(range: range, word: word, replacement: nil)
        spelling.addItem(add)
        let ignore = NSMenuItem(
            title: t("Ignore While This Runs"),
            action: #selector(ignoreMisspelling(_:)),
            keyEquivalent: ""
        )
        ignore.target = self
        ignore.representedObject = SpellingFix(range: range, word: word, replacement: nil)
        spelling.addItem(ignore)
    }

    func textView(
        _ textView: NSTextView,
        shouldChangeTextIn affectedCharRange: NSRange,
        replacementString: String?
    ) -> Bool {
        // A nil replacement is an attribute-only change; no text moves.
        guard let replacementString else { return true }
        if wrapSelection(in: textView, range: affectedCharRange, typed: replacementString) {
            // The wrap did the edit itself, with the selection kept on
            // what was wrapped so the next delimiter nests inside it.
            return false
        }
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

    /// Wraps the selection in a pair when an opening delimiter is typed
    /// over it: `hello` and `[` give `[hello]`, and the selection stays
    /// on `hello`, so `[({` in a row gives `[({hello})]`.
    ///
    /// Answers whether it handled the edit. The text view is told to
    /// refuse the original change, since this replaced it.
    private func wrapSelection(in textView: NSTextView, range: NSRange, typed: String) -> Bool {
        guard range.length > 0, let closing = CorePairs.closing(of: typed) else {
            return false
        }
        let text = textView.string as NSString
        guard NSMaxRange(range) <= text.length else { return false }
        let selected = text.substring(with: range)
        let wrapped = typed + selected + closing
        // Through the same door as any other edit: the core first, the
        // view second, one undo step for the pair.
        coreDocument.beginEditGroup()
        do {
            try coreDocument.replace(utf16Range: range, with: wrapped)
        } catch {
            coreDocument.endEditGroup()
            NSSound.beep()
            NSLog("wrap rejected by core: \(error)")
            return false
        }
        coreDocument.endEditGroup()
        selectionChangeIsFromEditing = true
        textView.textStorage?.replaceCharacters(in: range, with: wrapped)
        let inner = NSRange(
            location: range.location + (typed as NSString).length, length: range.length)
        textView.setSelectedRange(inner)
        lastTypedText = typed
        textDidChangeFromWrap()
        return true
    }

    /// The preview, for the smoke test to look at.
    var previewWebViewForTest: WKWebView? { previewWebView }

    /// The text view was filled directly; tell the core what it says.
    func noteTextReplaced() {
        guard let textView else { return }
        try? coreDocument.replace(
            utf16Range: NSRange(location: 0, length: coreDocument.lengthInUTF16),
            with: textView.string)
        textView.setSelectedRange(
            NSRange(location: 0, length: (textView.string as NSString).length))
    }

    /// The bookkeeping an edit through the delegate would have done.
    private func textDidChangeFromWrap() {
        NotificationCenter.default.post(name: NSText.didChangeNotification, object: textView)
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
        // AppKit prefers this method over the single-range one whenever
        // a delegate has both, so ordinary typing arrives here with one
        // range. Hand that case over so it goes through one door.
        if affectedRanges.count == 1 {
            return self.textView(
                textView,
                shouldChangeTextIn: affectedRanges[0].rangeValue,
                replacementString: replacementStrings[0])
        }
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
        // The lines moved; where the folds sit in characters has to be
        // worked out again before the next layout pass.
        foldSpansAreStale = true
        mirrorSnippetStops()
        updateChrome()
        refreshDecorations()
        scheduleLSPChange()
        scheduleChangeMarks()
        schedulePreviewUpdate()
        lineRuler?.invalidateLineStarts()
        completionAfterTyping()
        scheduleAutosave()
        for view in views {
            view.contextStrip.invalidateText()
            updateContextStrip(for: view)
        }
        assertInSync()
    }

    /// Keyboard routing while the completion popup is visible: arrows
    /// navigate it, return/tab accept, escape dismisses — everything else
    /// keeps flowing to the editor. With the popup away, return picks up
    /// the auto-indent path.
    func textView(_ textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard completionPopup.isVisible else {
            if coreDocument.isSnippetActive {
                switch Self.snippetKey(for: commandSelector) {
                case .nextStop:
                    if moveThroughSnippet(forward: true) { return true }
                case .previousStop:
                    if moveThroughSnippet(forward: false) { return true }
                case .cancel:
                    // Escape gives the keys back where the caret is,
                    // rather than jumping it to the end of the snippet.
                    coreDocument.cancelSnippet()
                    return true
                case nil:
                    break
                }
            }
            if commandSelector == #selector(NSResponder.insertNewline(_:)) {
                return insertNewlineAutoIndenting(in: textView)
            }
            if commandSelector == #selector(NSResponder.deleteBackward(_:)) {
                return deleteBackwardByIndent(in: textView)
            }
            if commandSelector == #selector(NSResponder.insertTab(_:)) {
                return indentToBlockAbove(in: textView)
            }
            if commandSelector == #selector(NSResponder.cancelOperation(_:)),
                !occurrenceRanges.isEmpty
            {
                // Escape puts the marks away, and only the marks: the
                // selection stays, so the word is still there to act on.
                clearOccurrences()
                return true
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

    /// Backspace in a line's leading spaces deletes back to the
    /// previous tab stop rather than one space at a time. Anywhere else
    /// in the line it is an ordinary backspace, which is what keeps the
    /// behaviour from ever surprising: it is the position that decides.
    ///
    /// Returns false to let AppKit do its usual thing — a selection, a
    /// caret at the very start of a line, a line indented with tabs.
    private func deleteBackwardByIndent(in textView: NSTextView) -> Bool {
        let selection = textView.selectedRange()
        guard selection.length == 0, selection.location > 0 else { return false }
        let text = textView.string as NSString
        let lineStart = text.lineRange(for: NSRange(location: selection.location, length: 0))
            .location
        let before = text.substring(
            with: NSRange(location: lineStart, length: selection.location - lineStart))
        let width = CoreDocument.backspaceWidth(before: before, tabWidth: appliedTabWidth)
        guard width > 1 else { return false }
        let target = NSRange(location: selection.location - width, length: width)
        // Through insertText, so the edit takes the same synchronized
        // path (delegate → core → history) as anything typed.
        textView.insertText("", replacementRange: target)
        return true
    }

    /// Tab in a line's leading whitespace lines the line up with the
    /// block above it, and one level deeper when it is already level.
    /// Everywhere else in the line Tab is Tab.
    private func indentToBlockAbove(in textView: NSTextView) -> Bool {
        let selection = textView.selectedRange()
        guard selection.length == 0 else { return false }
        let text = textView.string as NSString
        let line = text.lineRange(for: NSRange(location: selection.location, length: 0))
        let currentIndent = Self.leadingWhitespace(
            of: text.substring(with: line))
        // Only in the indentation: past it, Tab inserts as it always
        // has.
        guard selection.location <= line.location + (currentIndent as NSString).length else {
            return false
        }
        var previous: String?
        var at = line.location
        while at > 0 {
            let above = text.lineRange(for: NSRange(location: at - 1, length: 0))
            let content = text.substring(with: above)
            if !content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                previous = content
                break
            }
            at = above.location
        }
        // What the document already does, the way auto-indent decides
        // it: a tab-indented file keeps tabs.
        let usesTabs =
            currentIndent.contains("\t") || (previous ?? "").hasPrefix("\t")
            || textView.string.contains("\n\t")
        let wanted = CoreDocument.alignedIndent(
            previous: previous, currentIndent: currentIndent,
            tabWidth: appliedTabWidth, useTabs: usesTabs)
        guard wanted != currentIndent else { return false }
        let replacing = NSRange(
            location: line.location, length: (currentIndent as NSString).length)
        textView.insertText(wanted, replacementRange: replacing)
        // The caret follows the indentation it just asked for.
        let caret = line.location + (wanted as NSString).length
        textView.setSelectedRange(NSRange(location: caret, length: 0))
        return true
    }

    /// The whitespace a line starts with.
    static func leadingWhitespace(of line: String) -> String {
        String(line.prefix { $0 == " " || $0 == "\t" })
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
        // Clicking in a pane is how you say which one you mean: the
        // caret moved there, so that pane has the keyboard now.
        if let textView = notification.object as? NSTextView {
            workbench?.noteFocus(on: textView)
        }
        workbench?.refreshStatus()
        // A caret move that is not part of an edit (click, arrow keys) ends
        // the current typing run for undo purposes.
        if selectionChangeIsFromEditing {
            selectionChangeIsFromEditing = false
        } else {
            coreDocument.breakUndoCoalescing()
            completionPopup.dismiss()
            // Clicking away from a snippet is done with it; leaving Tab
            // captured after that would be a mode with no way out.
            if let textView = notification.object as? NSTextView {
                coreDocument.snippetCaretMoved(to: textView.selectedRange().location)
            }
        }
        // A new selection asks a new question, and an edit answers the
        // old one differently; both go through here.
        refreshOccurrences()
    }
}

// MARK: - Menu validation

extension DocumentController: NSMenuItemValidation {
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        switch menuItem.action {
        case #selector(performUndo(_:)):
            return coreDocument.canUndo
        case #selector(performRedo(_:)):
            return coreDocument.canRedo
        case #selector(jumpToDefinition(_:)):
            return lspOpenPath != nil || ctagsFallbackEnabled
        case #selector(findReferences(_:)), #selector(renameSymbol(_:)),
            #selector(formatDocument(_:)), #selector(showDocumentOutline(_:)),
            #selector(showCodeActions(_:)):
            return lspOpenPath != nil
        case #selector(closeColumn(_:)), #selector(showInEveryPane(_:)):
            return workbench?.canCloseColumn == true
        case #selector(closeView(_:)):
            return workbench?.canCloseView == true
        case #selector(focusOtherSide(_:)):
            return workbench?.hasSeveralPanes == true
        case #selector(goToBlockStart(_:)), #selector(goToBlockEnd(_:)):
            return coreDocument.languageName != nil
        case #selector(togglePreview(_:)):
            menuItem.state = previewItem != nil ? .on : .off
            return coreDocument.languageName == "markdown"
        case #selector(copyFileName(_:)), #selector(copyRelativePath(_:)),
            #selector(copyAbsolutePath(_:)), #selector(revertToSaved(_:)):
            return coreDocument.path != nil
        case #selector(copyForgeURL(_:)):
            return coreDocument.path.map(PathActions.isInGitRepository) ?? false
        default:
            return true
        }
    }
}

// MARK: - Copy path actions (File → Copy Path, acting on the front tab)

extension DocumentController {
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
