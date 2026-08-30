import AppKit
import SwiftUI
import TextchumKit

/// One window: a tab bar over one or more panes.
///
/// A tab is a document open in this window. A pane is a view of one of
/// those documents, and several panes can show the same one — the same
/// buffer, two places to read it. Which tab is highlighted follows the
/// pane with the keyboard, so going to a pane is going to its tab, and
/// choosing a tab changes what that pane shows.
///
/// The tabs are drawn here rather than by AppKit. `NSWindow` tabs are
/// separate windows with one visible at a time, which cannot be what a
/// pane shows.
@MainActor
final class Workbench: NSWindowController, NSWindowDelegate {
    /// Every window, in the order they were made.
    private(set) static var all: [Workbench] = []

    /// The documents this window holds, left to right on the tab bar.
    private(set) var documents: [DocumentController] = []
    /// The panes, left to right (or top to bottom).
    private(set) var panes: [Pane] = []
    /// Which pane has the keyboard.
    private(set) var focusedPane = 0

    /// The navigator: one buffer list per window, and a folder tree that
    /// follows the focused document's project.
    let sidebarModel = SidebarModel()
    let sidebarContext = WindowSidebarContext()
    /// The sidebar · editor · preview split; the preview lives here so
    /// the whole window has one, beside whichever pane has the focus.
    private(set) var splitController: NSSplitViewController?

    private let tabModel = TabBarModel()
    private var paneSplit = NSSplitView()
    private var applyingSidebarWidth = false
    /// Set while this window closes after asking about unsaved files, so
    /// the close it starts again goes straight through.
    private var closingSettled = false

    /// One pane: a container in the split, the document it shows, and
    /// the view that document vended for it.
    final class Pane {
        let container = NSView()
        weak var document: DocumentController?
        var view: DocumentView?

        init() {
            container.translatesAutoresizingMaskIntoConstraints = true
            container.autoresizingMask = [.width, .height]
        }
    }

    // MARK: Building

    init(sidebar: SidebarConfiguration?) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 920, height: 480),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        // Textchum draws its own tabs, so AppKit's are off: a window
        // tab is a window, and a window cannot sit in a pane.
        window.tabbingMode = .disallowed
        super.init(window: window)
        window.delegate = self
        window.center()

        paneSplit = NSSplitView()
        paneSplit.isVertical = true
        paneSplit.dividerStyle = .thin
        paneSplit.translatesAutoresizingMaskIntoConstraints = false

        let tabHost = NSHostingView(rootView: TabBarView(model: tabModel))
        tabHost.translatesAutoresizingMaskIntoConstraints = false
        tabModel.onSelect = { [weak self] id in self?.showInFocusedPane(id) }
        tabModel.onClose = { [weak self] id in self?.closeTab(id) }
        tabModel.onSelectEverywhere = { [weak self] id in self?.showEverywhere(id) }

        let editorSide = NSView()
        editorSide.addSubview(tabHost)
        editorSide.addSubview(paneSplit)
        NSLayoutConstraint.activate([
            tabHost.leadingAnchor.constraint(equalTo: editorSide.leadingAnchor),
            tabHost.trailingAnchor.constraint(equalTo: editorSide.trailingAnchor),
            tabHost.topAnchor.constraint(equalTo: editorSide.topAnchor),
            tabHost.heightAnchor.constraint(equalToConstant: 30),
            paneSplit.leadingAnchor.constraint(equalTo: editorSide.leadingAnchor),
            paneSplit.trailingAnchor.constraint(equalTo: editorSide.trailingAnchor),
            paneSplit.topAnchor.constraint(equalTo: tabHost.bottomAnchor),
            paneSplit.bottomAnchor.constraint(equalTo: editorSide.bottomAnchor),
        ])

        let editorController = NSViewController()
        editorController.view = editorSide

        if let sidebar {
            let splitController = NSSplitViewController()
            let sidebarView = SidebarView(
                model: sidebarModel,
                currentDocumentID: ObjectIdentifier(self),
                context: sidebarContext,
                treeState: sidebar.treeState,
                onSelectDocument: sidebar.selectDocument,
                onShowProperties: sidebar.showProperties,
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
                },
                hiddenGlobs: sidebar.hiddenGlobs,
                onRevealInTree: sidebar.revealInTree
            )
            let sidebarHost = NSHostingController(rootView: sidebarView)
            // Without this, the list inherits a phantom titlebar inset
            // and its first row starts scrolled out of view.
            sidebarHost.safeAreaRegions = []
            let sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebarHost)
            sidebarItem.minimumThickness = 180
            sidebarItem.maximumThickness = 400
            // Full-height layout slides the list under the title bar and
            // hides the first section header; keep the sidebar below it.
            sidebarItem.allowsFullHeightLayout = false
            splitController.addSplitViewItem(sidebarItem)
            splitController.addSplitViewItem(NSSplitViewItem(viewController: editorController))
            // The same autosave name in every window is deliberate: they
            // share one stored position, so a width set in one is the
            // width the next window opens with.
            splitController.splitView.autosaveName = "TextchumEditorSidebar"
            splitController.splitView.identifier =
                NSUserInterfaceItemIdentifier("TextchumEditorSidebar")
            observeSidebarWidth(of: splitController)
            window.contentViewController = splitController
            self.splitController = splitController
        } else {
            window.contentView = editorSide
        }

        // Screenshot hook: a fixed content size makes documentation
        // captures reproducible (TEXTCHUM_DEBUG_WINDOW=1200x760).
        var contentSize = NSSize(width: 920, height: 480)
        if let spec = ProcessInfo.processInfo.environment["TEXTCHUM_DEBUG_WINDOW"] {
            let parts = spec.split(separator: "x").compactMap { Double($0) }
            if parts.count == 2 {
                contentSize = NSSize(width: parts[0], height: parts[1])
            }
        }
        window.setContentSize(contentSize)
        window.center()

        // One pane to start with; splitting adds the second.
        let pane = Pane()
        panes = [pane]
        paneSplit.addArrangedSubview(pane.container)
        Self.all.append(self)
    }

    required init?(coder: NSCoder) {
        fatalError("Workbench is created in code")
    }

    /// Menu commands are the focused document's. The window controller
    /// is in the responder chain; the document is not, so the chain is
    /// told where to find it. `nextResponder` would be the other way to
    /// do it, and AppKit owns the window's.
    override func supplementalTarget(forAction action: Selector, sender: Any?) -> Any? {
        if let document = focusedDocument, document.responds(to: action) {
            return document
        }
        return super.supplementalTarget(forAction: action, sender: sender)
    }

    // MARK: Tabs

    /// Adds a document to this window and shows it in the focused pane.
    func add(_ document: DocumentController, at index: Int? = nil) {
        document.workbench = self
        if let index, index <= documents.count {
            documents.insert(document, at: index)
        } else {
            documents.append(document)
        }
        show(document, in: focusedPane)
        refreshTabs()
    }

    /// Takes a document out of this window without closing it — it is
    /// moving to another window.
    func detach(_ document: DocumentController) {
        documents.removeAll { $0 === document }
        for (index, pane) in panes.enumerated() where pane.document === document {
            release(pane)
            // A pane whose document left shows the next one along, and
            // an empty window keeps its pane empty rather than closing:
            // the caller is mid-move.
            if let next = documents.first {
                show(next, in: index)
            }
        }
        if document.workbench === self { document.workbench = nil }
        refreshTabs()
    }

    /// Closes a tab: the document goes if it agrees to, and every pane
    /// showing it moves to another.
    @discardableResult
    func closeTab(_ id: ObjectIdentifier) -> Bool {
        guard let document = documents.first(where: { ObjectIdentifier($0) == id }) else {
            return false
        }
        guard document.mayClose() else { return false }
        documents.removeAll { $0 === document }
        for (index, pane) in panes.enumerated() where pane.document === document {
            release(pane)
            if let next = documents.first {
                show(next, in: index)
            }
        }
        document.willClose()
        document.workbench = nil
        onDocumentClosed?(document)
        refreshTabs()
        // A window with nothing left in it has nothing to show.
        if documents.isEmpty {
            closingSettled = true
            window?.close()
        }
        return true
    }

    /// Called when a tab closes, so the application can forget the
    /// document and remember it for Reopen Closed Tab.
    var onDocumentClosed: ((DocumentController) -> Void)?

    /// The document a pane shows.
    func document(inPane index: Int) -> DocumentController? {
        panes.indices.contains(index) ? panes[index].document : nil
    }

    /// The document with the keyboard — what the menu commands and the
    /// window's chrome are about.
    var focusedDocument: DocumentController? { document(inPane: focusedPane) }

    /// Shows a document in a pane, making it a view of its own.
    func show(_ document: DocumentController, in paneIndex: Int) {
        guard panes.indices.contains(paneIndex) else { return }
        let pane = panes[paneIndex]
        if pane.document === document { return }
        release(pane)
        let view = document.makeView()
        pane.document = document
        pane.view = view
        view.container.frame = pane.container.bounds
        view.container.autoresizingMask = [.width, .height]
        view.container.translatesAutoresizingMaskIntoConstraints = true
        pane.container.addSubview(view.container)
        refreshTabs()
        refreshChrome(for: document)
    }

    /// Takes whatever a pane is showing out of it.
    private func release(_ pane: Pane) {
        if let view = pane.view {
            view.container.removeFromSuperview()
            pane.document?.drop(view)
        }
        pane.view = nil
        pane.document = nil
    }

    /// Tab ▸ chosen: the focused pane shows it.
    func showInFocusedPane(_ id: ObjectIdentifier) {
        guard let document = documents.first(where: { ObjectIdentifier($0) == id }) else {
            return
        }
        show(document, in: focusedPane)
        focus(pane: focusedPane)
    }

    /// One file on every side at once — reading two places in it without
    /// choosing the tab twice.
    func showEverywhere(_ id: ObjectIdentifier) {
        guard let document = documents.first(where: { ObjectIdentifier($0) == id }) else {
            return
        }
        for index in panes.indices {
            show(document, in: index)
        }
        focus(pane: focusedPane)
    }

    /// The next (or previous) tab, in the pane with the keyboard.
    func cycleTab(forward: Bool) {
        guard documents.count > 1, let current = focusedDocument,
            let at = documents.firstIndex(where: { $0 === current })
        else { return }
        let next = (at + (forward ? 1 : documents.count - 1)) % documents.count
        show(documents[next], in: focusedPane)
        focus(pane: focusedPane)
    }

    /// Tab i, counting from one — ⌘1…⌘9.
    func selectTab(number: Int) {
        guard number >= 1, number <= documents.count else { return }
        show(documents[number - 1], in: focusedPane)
        focus(pane: focusedPane)
    }

    // MARK: Panes

    var isSplit: Bool { panes.count > 1 }

    /// Puts a second pane beside the first, showing the same document —
    /// one buffer, two places to read it. It can be pointed at another
    /// tab afterwards.
    func split() {
        guard panes.count < 2, let showing = focusedDocument else { return }
        let pane = Pane()
        panes.append(pane)
        paneSplit.addArrangedSubview(pane.container)
        // Neither side collapses, and the divider is placed after a
        // layout pass — before one the split has no width to halve.
        paneSplit.setHoldingPriority(NSLayoutConstraint.Priority(250), forSubviewAt: 0)
        paneSplit.setHoldingPriority(NSLayoutConstraint.Priority(250), forSubviewAt: 1)
        paneSplit.layoutSubtreeIfNeeded()
        paneSplit.setPosition(paneSplit.bounds.width / 2, ofDividerAt: 0)
        show(showing, in: panes.count - 1)
        focus(pane: panes.count - 1)
    }

    /// Takes the second pane away; the first keeps the whole area.
    func closeSplit() {
        guard panes.count > 1 else { return }
        let pane = panes.removeLast()
        release(pane)
        pane.container.removeFromSuperview()
        // NSSplitView.addArrangedSubview turns off the autoresizing
        // mask; the pane that stays needs it back or it comes out of
        // the split laid out by constraints it no longer has.
        if let first = panes.first {
            first.container.translatesAutoresizingMaskIntoConstraints = true
            first.container.autoresizingMask = [.width, .height]
            first.container.frame = paneSplit.bounds
        }
        paneSplit.adjustSubviews()
        focus(pane: 0)
        refreshTabs()
    }

    /// The keyboard crosses the divider.
    func focusOtherPane() {
        guard panes.count > 1 else { return }
        focus(pane: (focusedPane + 1) % panes.count)
    }

    /// Gives a pane the keyboard, and the window's chrome with it.
    func focus(pane index: Int) {
        guard panes.indices.contains(index) else { return }
        focusedPane = index
        if let textView = panes[index].view?.textView {
            window?.makeFirstResponder(textView)
            textView.scrollRangeToVisible(textView.selectedRange())
        }
        if let document = panes[index].document {
            refreshChrome(for: document)
            document.didTakeFocus()
        }
        refreshTabs()
    }

    /// The pane a text view belongs to has the keyboard now — clicking
    /// in a pane is how you say which one you mean.
    func noteFocus(on textView: NSTextView) {
        guard
            let index = panes.firstIndex(where: { $0.view?.textView === textView }),
            index != focusedPane
        else { return }
        focusedPane = index
        if let document = panes[index].document {
            refreshChrome(for: document)
            document.didTakeFocus()
        }
        refreshTabs()
    }

    // MARK: Chrome

    /// The window wears the focused document's facts, and the tab bar
    /// its name and dirty mark.
    func refreshChrome(for document: DocumentController) {
        refreshTabs()
        guard focusedDocument === document, let window else { return }
        if let path = document.coreDocument.path {
            window.representedURL = URL(fileURLWithPath: path)
        } else {
            window.representedURL = nil
        }
        window.title = document.chromeTitle
        window.subtitle = document.chromeSubtitle
        window.isDocumentEdited = document.coreDocument.isDirty
    }

    /// Rebuilds the tab bar from the documents and the focused pane.
    func refreshTabs() {
        tabModel.tabs = documents.map { document in
            TabBarModel.Tab(
                id: ObjectIdentifier(document),
                title: document.chromeTitle,
                isDirty: document.coreDocument.isDirty,
                shownElsewhere: panes.contains {
                    $0.document === document && $0 !== panes[safe: focusedPane]
                }
            )
        }
        tabModel.selected = focusedDocument.map(ObjectIdentifier.init)
    }

    // MARK: Window

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        if closingSettled {
            closingSettled = false
            return true
        }
        // Every tab is asked, one at a time: a window closing takes its
        // files with it.
        for document in documents where !document.mayClose() {
            return false
        }
        return true
    }

    func windowWillClose(_ notification: Notification) {
        for document in documents {
            document.willClose()
            document.workbench = nil
            onDocumentClosed?(document)
        }
        documents = []
        for pane in panes { release(pane) }
        Self.all.removeAll { $0 === self }
    }

    func windowDidBecomeKey(_ notification: Notification) {
        NotificationCenter.default.post(name: .textchumDocumentsChanged, object: self)
        if let path = focusedDocument?.coreDocument.path {
            focusedDocument?.followInTree(path)
        }
    }

    // MARK: One sidebar width for the whole application

    /// Watches this window's divider and keeps every other window's in
    /// step. The autosave name makes the width outlive a launch; this
    /// makes it the same width in windows that are already open, which
    /// is what "the navigator is this wide" ought to mean.
    private func observeSidebarWidth(of controller: NSSplitViewController) {
        NotificationCenter.default.addObserver(
            forName: NSSplitView.didResizeSubviewsNotification,
            object: controller.splitView,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, !self.applyingSidebarWidth else { return }
                let width = self.currentSidebarWidth
                // A collapsed sidebar is a different setting — hiding
                // the navigator in one window should not hide it
                // everywhere.
                guard width > 1 else { return }
                NotificationCenter.default.post(
                    name: .textchumSidebarWidthChanged,
                    object: self,
                    userInfo: ["width": width]
                )
            }
        }
        NotificationCenter.default.addObserver(
            forName: .textchumSidebarWidthChanged,
            object: nil,
            queue: .main
        ) { [weak self] note in
            MainActor.assumeIsolated {
                guard let self,
                    note.object as? Workbench !== self,
                    let width = note.userInfo?["width"] as? CGFloat
                else { return }
                self.applySidebarWidth(width)
            }
        }
    }

    private var currentSidebarWidth: CGFloat {
        splitController?.splitViewItems.first?.viewController.view.frame.width ?? 0
    }

    private func applySidebarWidth(_ width: CGFloat) {
        guard let splitView = splitController?.splitView,
            Self.shouldAdoptSidebarWidth(
                width,
                current: currentSidebarWidth,
                collapsed: splitController?.splitViewItems.first?.isCollapsed ?? true
            )
        else { return }
        applyingSidebarWidth = true
        splitView.setPosition(width, ofDividerAt: 0)
        applyingSidebarWidth = false
    }

    /// Whether a width broadcast by another window is worth adopting.
    ///
    /// Separated out because the two ways this goes wrong are not
    /// visible in a screenshot: adopting a width already held starts
    /// two windows answering each other, and adopting anything while
    /// collapsed silently reopens a navigator the user closed. Hiding
    /// the sidebar in one window is a different decision from choosing
    /// how wide it is, and only the second one travels.
    static func shouldAdoptSidebarWidth(
        _ width: CGFloat,
        current: CGFloat,
        collapsed: Bool
    ) -> Bool {
        guard !collapsed else { return false }
        guard width > 1 else { return false }
        // Sub-point differences are the same width arriving back; acting
        // on them costs a layout pass and risks a loop.
        return abs(current - width) > 0.5
    }
}

extension Array {
    /// The element at `index`, or nil — pane lookups happen while panes
    /// are being added and taken away.
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}

// MARK: - The tab bar

/// What the tab bar shows, and what it reports back.
@MainActor
final class TabBarModel: ObservableObject {
    struct Tab: Identifiable, Equatable {
        let id: ObjectIdentifier
        let title: String
        let isDirty: Bool
        /// Shown in a pane other than the focused one, so the bar can
        /// say a file is on screen twice.
        let shownElsewhere: Bool
    }

    @Published var tabs: [Tab] = []
    @Published var selected: ObjectIdentifier?

    var onSelect: ((ObjectIdentifier) -> Void)?
    var onClose: ((ObjectIdentifier) -> Void)?
    /// ⌥-click: the same file on every side at once.
    var onSelectEverywhere: ((ObjectIdentifier) -> Void)?
}

struct TabBarView: View {
    @ObservedObject var model: TabBarModel

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 0) {
                ForEach(model.tabs) { tab in
                    TabChip(
                        tab: tab,
                        isSelected: tab.id == model.selected,
                        onSelect: {
                            if NSEvent.modifierFlags.contains(.option) {
                                model.onSelectEverywhere?(tab.id)
                            } else {
                                model.onSelect?(tab.id)
                            }
                        },
                        onClose: { model.onClose?(tab.id) }
                    )
                    if tab.id != model.tabs.last?.id {
                        Divider().frame(height: 16)
                    }
                }
                Spacer(minLength: 0)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // The bar is window furniture; the tab in the pane is the
        // editor's own surface, so it wears the editor's background and
        // reads as the sheet the text sits on.
        .background(Color(nsColor: .windowBackgroundColor))
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(Color(nsColor: .separatorColor))
                .frame(height: 1)
        }
    }
}

private struct TabChip: View {
    let tab: TabBarModel.Tab
    let isSelected: Bool
    let onSelect: () -> Void
    let onClose: () -> Void
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 5) {
            if tab.isDirty {
                Circle().frame(width: 6, height: 6).foregroundStyle(.secondary)
            }
            Text(tab.title)
                .lineLimit(1)
                .font(.system(size: 12))
            // A file open in the other pane too, said once rather than
            // by drawing the tab twice.
            if tab.shownElsewhere {
                Image(systemName: "rectangle.split.2x1")
                    .font(.system(size: 9))
                    .foregroundStyle(.secondary)
            }
            Button(action: onClose) {
                Image(systemName: "xmark")
                    .font(.system(size: 8, weight: .bold))
            }
            .buttonStyle(.plain)
            .opacity(hovering || isSelected ? 1 : 0)
            .accessibilityLabel("Close \(tab.title)")
        }
        .padding(.horizontal, 10)
        .frame(height: 30)
        .background(
            isSelected ? Color(nsColor: .textBackgroundColor) : Color.clear
        )
        .foregroundStyle(isSelected ? Color.primary : Color.secondary)
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
        .help(tab.title)
    }
}
