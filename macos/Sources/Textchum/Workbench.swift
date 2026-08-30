import AppKit
import SwiftUI
import TextchumKit

/// One window: a tab bar over a row of columns.
///
/// A tab is a document open in this window. A column shows one of them
/// at a time and holds one or more views of it, stacked — the same
/// buffer, two places to look at it. The column owns the document; the
/// views are places to look at it, so when a column's tab changes every
/// view in it follows.
///
/// The tab bar highlights what the focused column shows, so going to a
/// pane is going to its tab, and choosing a tab changes what that
/// column shows.
///
/// The tabs are drawn here. `NSWindow` tabs are separate windows with
/// one visible at a time, which cannot be what a column shows.
@MainActor
final class Workbench: NSWindowController, NSWindowDelegate {
    /// Every window, in the order they were made.
    private(set) static var all: [Workbench] = []

    /// The documents this window holds, left to right on the tab bar.
    private(set) var documents: [DocumentController] = []
    /// The columns, left to right.
    private(set) var columns: [Column] = []
    /// Which column has the keyboard, and which of its views.
    private(set) var focusedColumn = 0
    private(set) var focusedView = 0

    /// The navigator: one buffer list per window, and a folder tree that
    /// follows the focused document's project.
    let sidebarModel = SidebarModel()
    let sidebarContext = WindowSidebarContext()
    /// The sidebar · editor · preview split; the preview lives here so
    /// the whole window has one, beside whichever pane has the focus.
    private(set) var splitController: NSSplitViewController?

    private let tabModel = TabBarModel()
    /// The row of columns.
    private var columnSplit = NSSplitView()
    private var applyingSidebarWidth = false
    /// Set while this window closes after asking about unsaved files, so
    /// the close it starts again goes straight through.
    private var closingSettled = false

    /// One column: the file it shows, and the views of that file
    /// stacked in it.
    @MainActor
    final class Column {
        /// The column's place in the row, and the split the views are
        /// stacked in.
        let split = NSSplitView()
        weak var document: DocumentController?
        var views: [DocumentView] = []

        init() {
            split.isVertical = false
            split.dividerStyle = .thin
            split.translatesAutoresizingMaskIntoConstraints = true
            split.autoresizingMask = [.width, .height]
        }

        /// Where each view sits in the column, as fractions of its
        /// height — what #102 remembers per file.
        var dividerFractions: [Double] {
            guard views.count > 1, split.bounds.height > 1 else { return [] }
            return views.dropLast().map { view in
                Double(view.container.frame.maxY / split.bounds.height)
            }
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

        columnSplit = NSSplitView()
        columnSplit.isVertical = true
        columnSplit.dividerStyle = .thin
        columnSplit.translatesAutoresizingMaskIntoConstraints = false

        let tabHost = NSHostingView(rootView: TabBarView(model: tabModel))
        tabHost.translatesAutoresizingMaskIntoConstraints = false
        tabModel.onSelect = { [weak self] id in self?.showInFocusedPane(id) }
        tabModel.onClose = { [weak self] id in self?.closeTab(id) }
        tabModel.onSelectEverywhere = { [weak self] id in self?.showEverywhere(id) }

        let editorSide = NSView()
        editorSide.addSubview(tabHost)
        editorSide.addSubview(columnSplit)
        NSLayoutConstraint.activate([
            tabHost.leadingAnchor.constraint(equalTo: editorSide.leadingAnchor),
            tabHost.trailingAnchor.constraint(equalTo: editorSide.trailingAnchor),
            tabHost.topAnchor.constraint(equalTo: editorSide.topAnchor),
            tabHost.heightAnchor.constraint(equalToConstant: 30),
            columnSplit.leadingAnchor.constraint(equalTo: editorSide.leadingAnchor),
            columnSplit.trailingAnchor.constraint(equalTo: editorSide.trailingAnchor),
            columnSplit.topAnchor.constraint(equalTo: tabHost.bottomAnchor),
            columnSplit.bottomAnchor.constraint(equalTo: editorSide.bottomAnchor),
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

        // One column to start with; New Column adds the next.
        let column = Column()
        columns = [column]
        columnSplit.addArrangedSubview(column.split)
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

    /// Adds a document to this window and shows it in the focused
    /// column.
    func add(_ document: DocumentController, at index: Int? = nil) {
        document.workbench = self
        if let index, index <= documents.count {
            documents.insert(document, at: index)
        } else {
            documents.append(document)
        }
        show(document, inColumn: focusedColumn)
        refreshTabs()
    }

    /// Takes a document out of this window without closing it — it is
    /// moving to another window.
    func detach(_ document: DocumentController) {
        documents.removeAll { $0 === document }
        for (index, column) in columns.enumerated() where column.document === document {
            // A column whose file left shows the next one along, and an
            // empty window keeps its column empty: the caller is
            // mid-move.
            if let next = documents.first {
                show(next, inColumn: index)
            } else {
                release(column)
            }
        }
        if document.workbench === self { document.workbench = nil }
        refreshTabs()
    }

    /// Closes a tab: the document goes if it agrees to, and every
    /// column showing it moves to another.
    @discardableResult
    func closeTab(_ id: ObjectIdentifier) -> Bool {
        guard let document = documents.first(where: { ObjectIdentifier($0) == id }) else {
            return false
        }
        guard document.mayClose() else { return false }
        documents.removeAll { $0 === document }
        for (index, column) in columns.enumerated() where column.document === document {
            if let next = documents.first {
                show(next, inColumn: index)
            } else {
                release(column)
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

    /// The document a column shows.
    func document(inColumn index: Int) -> DocumentController? {
        columns.indices.contains(index) ? columns[index].document : nil
    }

    /// The document with the keyboard — what the menu commands and the
    /// window's chrome are about.
    var focusedDocument: DocumentController? { document(inColumn: focusedColumn) }

    /// Shows a document in a column, in as many views as the column had.
    ///
    /// The column owns the file; the views are places to look at it. A
    /// column reading one file in two views goes on reading two views
    /// of whatever it is switched to.
    func show(_ document: DocumentController, inColumn index: Int) {
        guard columns.indices.contains(index) else { return }
        let column = columns[index]
        if column.document === document { return }
        // What the outgoing file looked like here is what it looks like
        // when it comes back.
        record(column)
        release(column)
        column.document = document
        let layout = document.openDocument.layout
        for _ in 0..<max(1, layout.views) {
            addView(to: column)
        }
        placeDividers(of: column, at: layout.dividers)
        for (view, place) in zip(column.views, layout.places) {
            restore(view: view, to: place)
        }
        refreshTabs()
        refreshChrome(for: document)
    }

    /// Writes down how a column is showing its file: the file is what
    /// remembers, so any column it lands in afterwards looks the same.
    private func record(_ column: Column) {
        guard let document = column.document, !column.views.isEmpty else { return }
        document.openDocument.layout = DocumentLayout(
            views: column.views.count,
            dividers: column.dividerFractions,
            places: column.views.map { view in
                DocumentLayout.Place(
                    caret: view.textView.selectedRange().location,
                    scroll: Double(view.scrollView.contentView.bounds.origin.y))
            })
        // The file is what remembers, and the project is where that is
        // written down.
        document.recordProjectState()
    }

    /// Records every column, for the session and for a file that is
    /// about to be closed.
    func recordLayouts() {
        for column in columns { record(column) }
    }

    /// Puts a view back where it was looking.
    private func restore(view: DocumentView, to place: DocumentLayout.Place) {
        let length = (view.textView.string as NSString).length
        view.textView.setSelectedRange(
            NSRange(location: min(place.caret, length), length: 0))
        if place.scroll > 0 {
            view.scrollView.contentView.scroll(to: NSPoint(x: 0, y: place.scroll))
            view.scrollView.reflectScrolledClipView(view.scrollView.contentView)
        }
    }

    /// One more view of what the column shows, stacked under the rest.
    @discardableResult
    private func addView(to column: Column) -> DocumentView? {
        guard let document = column.document else { return nil }
        let view = document.makeView()
        column.views.append(view)
        column.split.addArrangedSubview(view.container)
        for index in column.views.indices {
            column.split.setHoldingPriority(
                NSLayoutConstraint.Priority(250), forSubviewAt: index)
        }
        return view
    }

    /// Everything a column is showing, given up.
    private func release(_ column: Column) {
        for view in column.views {
            view.container.removeFromSuperview()
            column.document?.drop(view)
        }
        column.views = []
        column.document = nil
    }

    /// Shares a column's height between its views.
    private func placeDividers(of column: Column, at fractions: [Double] = []) {
        guard column.views.count > 1 else { return }
        column.split.layoutSubtreeIfNeeded()
        let height = column.split.bounds.height
        guard height > 1 else { return }
        for divider in 0..<(column.views.count - 1) {
            let fraction =
                divider < fractions.count
                ? fractions[divider]
                : Double(divider + 1) / Double(column.views.count)
            column.split.setPosition(height * CGFloat(fraction), ofDividerAt: divider)
        }
    }

    /// Tab ▸ chosen: the focused column shows it.
    func showInFocusedPane(_ id: ObjectIdentifier) {
        guard let document = documents.first(where: { ObjectIdentifier($0) == id }) else {
            return
        }
        show(document, inColumn: focusedColumn)
        focus(column: focusedColumn, view: 0)
    }

    /// One file in every column at once.
    func showEverywhere(_ id: ObjectIdentifier) {
        guard let document = documents.first(where: { ObjectIdentifier($0) == id }) else {
            return
        }
        for index in columns.indices {
            show(document, inColumn: index)
        }
        focus(column: focusedColumn, view: focusedView)
    }

    /// The next (or previous) tab, in the column with the keyboard.
    func cycleTab(forward: Bool) {
        guard documents.count > 1, let current = focusedDocument,
            let at = documents.firstIndex(where: { $0 === current })
        else { return }
        let next = (at + (forward ? 1 : documents.count - 1)) % documents.count
        show(documents[next], inColumn: focusedColumn)
        focus(column: focusedColumn, view: 0)
    }

    /// Tab i, counting from one — ⌘1…⌘9.
    func selectTab(number: Int) {
        guard number >= 1, number <= documents.count else { return }
        show(documents[number - 1], inColumn: focusedColumn)
        focus(column: focusedColumn, view: 0)
    }

    // MARK: Columns and views

    var isSplit: Bool { columns.count > 1 }
    /// Whether there is more than one place for the keyboard to be.
    var hasSeveralPanes: Bool {
        columns.count > 1 || columns.contains { $0.views.count > 1 }
    }
    var canCloseColumn: Bool { columns.count > 1 }
    var canCloseView: Bool {
        columns.indices.contains(focusedColumn) && columns[focusedColumn].views.count > 1
    }

    /// A column beside this one, showing the same file to start with.
    /// It takes any tab afterwards.
    func newColumn() {
        guard let showing = focusedDocument else { return }
        let column = Column()
        columns.append(column)
        columnSplit.addArrangedSubview(column.split)
        // No column collapses to nothing, and the dividers are placed
        // after a layout pass — before one the row has no width to
        // share out.
        for index in columns.indices {
            columnSplit.setHoldingPriority(
                NSLayoutConstraint.Priority(250), forSubviewAt: index)
        }
        column.document = showing
        addView(to: column)
        spreadColumns()
        focus(column: columns.count - 1, view: 0)
        refreshTabs()
    }

    /// Takes the focused column away; the rest share its width.
    func closeColumn() {
        guard columns.count > 1 else { return }
        let column = columns.remove(at: min(focusedColumn, columns.count - 1))
        release(column)
        column.split.removeFromSuperview()
        // NSSplitView.addArrangedSubview turns the autoresizing mask
        // off; a column that comes back out of the row needs it on, or
        // it is laid out by constraints it no longer has.
        for other in columns {
            other.split.translatesAutoresizingMaskIntoConstraints = true
            other.split.autoresizingMask = [.width, .height]
        }
        columnSplit.adjustSubviews()
        spreadColumns()
        focus(column: min(focusedColumn, columns.count - 1), view: 0)
        refreshTabs()
    }

    /// Another view of this column's file, under the one that has the
    /// keyboard: the top of a function while its end is being written.
    func addViewToFocusedColumn() {
        guard columns.indices.contains(focusedColumn) else { return }
        let column = columns[focusedColumn]
        guard column.document != nil else { return }
        addView(to: column)
        placeDividers(of: column)
        record(column)
        focus(column: focusedColumn, view: column.views.count - 1)
    }

    /// Takes the focused view out of its column, leaving the others.
    func closeFocusedView() {
        guard columns.indices.contains(focusedColumn) else { return }
        let column = columns[focusedColumn]
        guard column.views.count > 1, column.views.indices.contains(focusedView) else {
            return
        }
        let view = column.views.remove(at: focusedView)
        view.container.removeFromSuperview()
        column.document?.drop(view)
        for other in column.views {
            other.container.translatesAutoresizingMaskIntoConstraints = true
            other.container.autoresizingMask = [.width, .height]
        }
        column.split.adjustSubviews()
        placeDividers(of: column)
        record(column)
        focus(column: focusedColumn, view: min(focusedView, column.views.count - 1))
    }

    /// Puts a column back the way a session recorded it: the views it
    /// held, and where the dividers between them sat.
    func restore(column index: Int, views: Int, dividers: [Double]) {
        guard columns.indices.contains(index) else { return }
        let column = columns[index]
        while column.views.count < max(1, views) {
            guard addView(to: column) != nil else { break }
        }
        placeDividers(of: column, at: dividers)
    }

    /// Shares the row's width between the columns.
    private func spreadColumns() {
        guard columns.count > 1 else { return }
        columnSplit.layoutSubtreeIfNeeded()
        let width = columnSplit.bounds.width
        guard width > 1 else { return }
        for divider in 0..<(columns.count - 1) {
            columnSplit.setPosition(
                width * CGFloat(divider + 1) / CGFloat(columns.count), ofDividerAt: divider)
        }
    }

    /// The keyboard moves to the next place there is one: the next view
    /// down this column, then the next column along.
    func focusOtherPane() {
        let places = panePlaces()
        guard places.count > 1 else { return }
        let at =
            places.firstIndex { $0 == (focusedColumn, focusedView) }
            ?? 0
        let next = places[(at + 1) % places.count]
        focus(column: next.0, view: next.1)
    }

    /// Every (column, view) pair, in reading order.
    private func panePlaces() -> [(Int, Int)] {
        columns.enumerated().flatMap { column, holder in
            holder.views.indices.map { (column, $0) }
        }
    }

    /// Gives a view the keyboard, and the window's chrome with it.
    func focus(column: Int, view: Int = 0) {
        guard columns.indices.contains(column) else { return }
        focusedColumn = column
        let holder = columns[column]
        focusedView = holder.views.indices.contains(view) ? view : 0
        if let textView = holder.views[safe: focusedView]?.textView {
            window?.makeFirstResponder(textView)
            textView.scrollRangeToVisible(textView.selectedRange())
        }
        if let document = holder.document {
            refreshChrome(for: document)
            document.didTakeFocus()
        }
        refreshTabs()
    }

    /// The view a text view belongs to has the keyboard now — clicking
    /// in a pane is how you say which one you mean.
    func noteFocus(on textView: NSTextView) {
        for (column, holder) in columns.enumerated() {
            guard let view = holder.views.firstIndex(where: { $0.textView === textView })
            else { continue }
            guard column != focusedColumn || view != focusedView else { return }
            focusedColumn = column
            focusedView = view
            if let document = holder.document {
                refreshChrome(for: document)
                document.didTakeFocus()
            }
            refreshTabs()
            return
        }
    }

    // MARK: Chrome

    /// Puts the focused document's Markdown preview beside the text,
    /// and takes away whichever one was there.
    ///
    /// The preview belongs to the document whose HTML it shows; the
    /// window is where it goes. Without this, switching tabs left one
    /// file's preview open beside another file's text.
    func refreshPreview() {
        guard let splitController else { return }
        let wanted = focusedDocument?.previewItem
        // The sidebar and the editor come first; anything after them is
        // a preview that a document put there.
        for item in splitController.splitViewItems.dropFirst(2) where item !== wanted {
            splitController.removeSplitViewItem(item)
        }
        guard let wanted else { return }
        if !splitController.splitViewItems.contains(wanted) {
            // The editor must never be squeezed out: it keeps its space
            // (higher holding priority, real minimum); the preview yields.
            if splitController.splitViewItems.count > 1 {
                let editorItem = splitController.splitViewItems[1]
                editorItem.minimumThickness = 340
                editorItem.holdingPriority = NSLayoutConstraint.Priority(260)
            }
            splitController.addSplitViewItem(wanted)
        }
    }

    /// The window wears the focused document's facts, and the tab bar
    /// its name and dirty mark.
    func refreshChrome(for document: DocumentController) {
        refreshTabs()
        refreshPreview()
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

    /// Rebuilds the tab bar from the documents and the focused column.
    func refreshTabs() {
        tabModel.tabs = documents.map { document in
            TabBarModel.Tab(
                id: ObjectIdentifier(document),
                title: document.chromeTitle,
                isDirty: document.coreDocument.isDirty,
                // Shown in a column other than the focused one, so the
                // bar can say a file is on screen twice.
                shownElsewhere: columns.enumerated().contains {
                    $0.element.document === document && $0.offset != focusedColumn
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
        for column in columns { release(column) }
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
