import AppKit
import TextchumKit

/// The shared quick-search panel behind ⌘T (fuzzy file open) and ⇧⌘F
/// (find in project).
///
/// The design rule both modes share: **the scope is a visible, editable
/// path.** The top field shows exactly where the search looks —
/// defaulting to the current document's project — and widening the search
/// is literally editing that path. Below it, the query field and results;
/// ↑/↓ move the selection, ⏎ opens, ⎋ closes.
@MainActor
final class QuickFinderPanel: NSObject {
    enum Mode {
        case files
        case grep
        /// The files the branch touches, as an openable list: the pull
        /// request's files, read from git alone.
        case changed

        var title: String {
            switch self {
            case .files: t("Open Quickly")
            case .grep: t("Find in Project")
            case .changed: t("Changed in Branch")
            }
        }

        var placeholder: String {
            switch self {
            case .files, .changed: t("fuzzy file name…")
            case .grep: t("regular expression…")
            }
        }
    }

    /// The merge-base priorities the changed list resolves with; handed
    /// in by the caller, which knows the project.
    var mergeBaseBranches: [String] = []

    private var panel: NSPanel?
    /// Spelled out in the status strip, because a finder whose ⏎ does
    /// not open is only friendly if it says so.
    private static let keyHint = "↑↓ select · ⏎ search · ⌘⏎ open · ⎋ close"

    private var mode: Mode = .files
    /// The scope's file list, walked once when the panel opens (or the
    /// scope changes) and matched in memory per keystroke — re-walking
    /// a real repository on every character is what made results
    /// arrive late, or never.
    private var fileIndex: [String] = []
    /// The last scope the finder actually used, so reopening it
    /// without a project in front does not fall back to the home
    /// directory and index the world.
    private(set) static var lastScope: String?
    private var keyMonitor: Any?
    /// The scope `fileIndex` belongs to, and whether its walk is done.
    private var indexedScope: String?
    private var isIndexing = false
    private let scopeField = NSTextField()
    private let queryField = NSTextField()
    /// Stacked refinements (grep mode): one row per filter, below the
    /// query, narrowing by line content or file name.
    private let filtersStack = NSStackView()
    private let addFilterButton = NSButton(
        title: t("＋ Add Filter"), target: nil, action: nil)
    private let table = NSTableView()
    /// Says what the last search did, so an empty list is never mute.
    private let statusLabel = NSTextField(labelWithString: "")
    private var rows: [(display: String, path: String, line: Int)] = []
    private var searchGeneration = 0
    private var debounce: Timer?
    /// Opens a result: absolute path, one-based line (0 = just open).
    var onOpen: ((String, Int) -> Void)?

    /// Presents the panel for `mode`, scoped to `scope`.
    func show(mode: Mode, scope: String, over window: NSWindow?) {
        self.mode = mode
        let panel = self.panel ?? makePanel()
        self.panel = panel
        panel.title = mode.title
        scopeField.stringValue = scope
        Self.lastScope = scope
        queryField.placeholderString = mode.placeholder
        queryField.stringValue = ""
        rows = []
        table.reloadData()
        clearFilters()

        if let window {
            var frame = panel.frame
            frame.origin.x = window.frame.midX - frame.width / 2
            frame.origin.y = window.frame.maxY - frame.height - 120
            panel.setFrame(frame, display: false)
        } else {
            panel.center()
        }
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(queryField)
        // Files mode indexes the scope once; grep asks the core per
        // query (it streams from disk by design).
        if mode != .grep {
            refreshFileIndex(force: true)
        } else {
            runSearch()
        }
    }

    /// Walks the scope off the main thread, then runs the pending query
    /// against the fresh list. Cheap to call: it no-ops when the scope
    /// is already indexed and `force` is false.
    private func refreshFileIndex(force: Bool) {
        let scope = (scopeField.stringValue as NSString).expandingTildeInPath
        guard force || indexedScope != scope else { return }
        indexedScope = scope
        fileIndex = []
        isIndexing = true
        statusLabel.stringValue =
            "Indexing \((scope as NSString).lastPathComponent)…   ·   \(Self.keyHint)"
        searchGeneration += 1
        let generation = searchGeneration
        let mode = self.mode
        let branches = mergeBaseBranches
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let paths: [String]
            if mode == .changed {
                paths = CoreChanges.branchFiles(near: scope, branches: branches)?
                    .files.map(\.path) ?? []
            } else {
                paths = CoreSearch.listFiles(root: scope)
            }
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    guard let self, self.searchGeneration == generation else { return }
                    self.fileIndex = paths
                    self.isIndexing = false
                    self.runSearch()
                }
            }
        }
    }

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 420),
            styleMask: [.titled, .closable, .utilityWindow],
            backing: .buffered,
            defer: false
        )
        panel.isFloatingPanel = true
        panel.becomesKeyOnlyIfNeeded = false

        // ⏎ searches, ⌘⏎ opens: the finder should never open something
        // on the strength of a keystroke meant to refine the query.
        // The text field swallows plain keys, so ⌘⏎ is caught here.
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) {
            [weak self] event in
            guard let self, self.panel?.isKeyWindow == true else { return event }
            let isReturn = event.keyCode == 36 || event.keyCode == 76
            guard isReturn, event.modifierFlags.contains(.command) else { return event }
            self.openSelection()
            return nil
        }

        scopeField.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        scopeField.placeholderString = "scope path"
        scopeField.delegate = self
        queryField.font = .systemFont(ofSize: 15)
        queryField.delegate = self

        table.addTableColumn(NSTableColumn(identifier: .init("result")))
        table.headerView = nil
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.doubleAction = #selector(openSelection)
        table.rowHeight = 22
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true

        filtersStack.orientation = .vertical
        filtersStack.spacing = 4
        addFilterButton.target = self
        addFilterButton.action = #selector(addFilterPressed)
        addFilterButton.bezelStyle = .inline
        addFilterButton.font = .systemFont(ofSize: 11)
        let addRow = NSStackView(views: [addFilterButton, NSView()])
        addRow.orientation = .horizontal

        statusLabel.font = .systemFont(ofSize: 11)
        statusLabel.textColor = .secondaryLabelColor
        statusLabel.lineBreakMode = .byTruncatingTail

        let stack = NSStackView(views: [
            scopeField, queryField, filtersStack, addRow, scroll, statusLabel,
        ])
        stack.orientation = .vertical
        stack.spacing = 6
        stack.edgeInsets = NSEdgeInsets(top: 10, left: 10, bottom: 10, right: 10)
        stack.distribution = .fill
        scroll.setContentHuggingPriority(.defaultLow, for: .vertical)
        panel.contentView = stack
        return panel
    }

    // MARK: Filters

    /// One filter row: kind popup + pattern field + remove button.
    private func makeFilterRow() -> NSStackView {
        let kind = NSPopUpButton()
        kind.addItems(withTitles: [
            "line contains", "line excludes", "file contains", "file excludes",
        ])
        kind.font = .systemFont(ofSize: 11)
        kind.target = self
        kind.action = #selector(filterChanged)
        let pattern = NSTextField()
        pattern.placeholderString = "filter text…"
        pattern.font = .systemFont(ofSize: 12)
        pattern.delegate = self
        let remove = NSButton(
            image: NSImage(systemSymbolName: "minus.circle", accessibilityDescription: "Remove")
                ?? NSImage(),
            target: self,
            action: #selector(removeFilterPressed(_:))
        )
        remove.isBordered = false
        let row = NSStackView(views: [kind, pattern, remove])
        row.orientation = .horizontal
        row.spacing = 6
        return row
    }

    @objc private func addFilterPressed() {
        filtersStack.addArrangedSubview(makeFilterRow())
        if let pattern = (filtersStack.arrangedSubviews.last as? NSStackView)?
            .arrangedSubviews[1] as? NSTextField
        {
            panel?.makeFirstResponder(pattern)
        }
    }

    @objc private func removeFilterPressed(_ sender: NSButton) {
        if let row = sender.superview as? NSStackView {
            filtersStack.removeArrangedSubview(row)
            row.removeFromSuperview()
            scheduleSearch()
        }
    }

    @objc private func filterChanged() {
        scheduleSearch()
    }

    private func clearFilters() {
        for view in filtersStack.arrangedSubviews {
            filtersStack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
    }

    /// The current filter rows as core filters (empty patterns skipped).
    private func currentFilters() -> [CoreSearch.Filter] {
        filtersStack.arrangedSubviews.compactMap { view in
            guard let row = view as? NSStackView,
                let kind = row.arrangedSubviews.first as? NSPopUpButton,
                let pattern = row.arrangedSubviews.dropFirst().first as? NSTextField,
                !pattern.stringValue.isEmpty
            else { return nil }
            switch kind.indexOfSelectedItem {
            case 0: return .init(kind: .line, include: true, pattern: pattern.stringValue)
            case 1: return .init(kind: .line, include: false, pattern: pattern.stringValue)
            case 2: return .init(kind: .file, include: true, pattern: pattern.stringValue)
            default: return .init(kind: .file, include: false, pattern: pattern.stringValue)
            }
        }
    }

    // MARK: Searching

    /// Debounced scope re-walk: typing a path should not walk after
    /// every character.
    private func scheduleIndexRefresh() {
        debounce?.invalidate()
        let timer = Timer(timeInterval: 0.3, repeats: false) { [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    guard let self else { return }
                    if self.mode != .grep {
                        self.refreshFileIndex(force: true)
                    } else {
                        self.runSearch()
                    }
                }
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        debounce = timer
    }

    private func scheduleSearch() {
        debounce?.invalidate()
        // .common mode: a timer scheduled while the field editor is
        // tracking would otherwise wait for typing to stop entirely.
        let timer = Timer(timeInterval: 0.05, repeats: false) { [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.runSearch() }
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        debounce = timer
    }

    private func runSearch() {
        let scope = (scopeField.stringValue as NSString).expandingTildeInPath
        let query = queryField.stringValue
        let mode = self.mode
        let filters = currentFilters()
        let index = fileIndex
        // The walk owns the status line until it finishes; matching an
        // empty index would blank the panel and look like "no results".
        if mode != .grep, isIndexing { return }
        searchGeneration += 1
        let generation = searchGeneration

        // Pure core functions, run off the main thread; stale results
        // (an older generation) are dropped on arrival.
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            var results: [(String, String, Int)] = []
            var status = ""
            switch mode {
            case .files, .changed:
                // Over-fetch so the stacked filters have something to
                // prune; every filter kind applies to the path here.
                let names = CoreSearch.matchFiles(paths: index, query: query, limit: 400)
                let filtered = names.filter { name in
                    filters.allSatisfy { filter in
                        name.lowercased().contains(filter.pattern.lowercased())
                            == filter.include
                    }
                }
                results = filtered.prefix(100).map { ($0, "\(scope)/\($0)", 0) }
                if filtered.isEmpty {
                    status =
                        !names.isEmpty
                        ? "\(names.count) files matched, all filtered out."
                        : index.isEmpty
                            ? (FileManager.default.fileExists(atPath: scope)
                                ? "No files under this scope."
                                : "That scope does not exist.")
                            : "No files match \u{201c}\(query)\u{201d} in \(index.count) files."
                } else if filtered.count < names.count {
                    status =
                        "\(filtered.count) of \(names.count) matches survive the filters."
                } else if index.count >= 100_000 {
                    status =
                        "\(filtered.count) of \(index.count)+ files — narrow the scope."
                } else {
                    status = "\(filtered.count) of \(index.count) files."
                }
            case .grep:
                if query.isEmpty {
                    status = "Type to search."
                } else {
                    do {
                        // Smart case, as ripgrep does it: a lowercase
                        // query matches any case, a query with an
                        // uppercase letter is taken literally.
                        let smartCase = query == query.lowercased()
                        let found = try CoreSearch.grep(
                            root: scope, pattern: query, caseInsensitive: smartCase,
                            limit: 200, filters: filters)
                        results = found.hits.map {
                            ("\($0.path):\($0.line): \($0.text)", "\(scope)/\($0.path)", $0.line)
                        }
                        status = Self.status(for: found, scope: scope)
                    } catch let error as CoreIOError {
                        // A bad pattern used to read as "no results".
                        // The regex crate's message spans lines with a
                        // caret diagram; one line fits the status strip.
                        status = error.message
                            .components(separatedBy: .newlines)
                            .map { $0.trimmingCharacters(in: .whitespaces) }
                            .filter { !$0.isEmpty && $0 != "^" }
                            .joined(separator: " ")
                    } catch {
                        status = "\(error)"
                    }
                }
            }
            let finalStatus = status
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    guard let self, self.searchGeneration == generation else { return }
                    self.rows = results
                    self.statusLabel.stringValue =
                        finalStatus.isEmpty
                        ? Self.keyHint : "\(finalStatus)   ·   \(Self.keyHint)"
                    self.table.reloadData()
                    if !results.isEmpty {
                        self.table.selectRowIndexes([0], byExtendingSelection: false)
                    }
                }
            }
        }
    }

    /// One line explaining what the search did — the difference between
    /// "your query matched nothing" and "nothing was read at all".
    /// Pure (results in, string out), so it runs on the search queue —
    /// `nonisolated` opts it out of the class's main-actor isolation.
    private nonisolated static func status(
        for results: CoreSearch.Results, scope: String
    ) -> String {
        let stats = results.stats
        if !results.hits.isEmpty {
            let files = Set(results.hits.map(\.path)).count
            return "\(results.hits.count) matches in \(files) "
                + "file\(files == 1 ? "" : "s") · \(stats.filesSearched) searched"
        }
        if !FileManager.default.fileExists(atPath: scope) {
            return "That scope does not exist."
        }
        if stats.filesSearched == 0 {
            return stats.unreadable > 0
                ? "Nothing readable in this scope (\(stats.unreadable) entries denied)."
                : "No files to search here — everything is ignored or the scope is empty."
        }
        return "No matches in \(stats.filesSearched) files searched."
    }

    /// Debug hook: force scope and query and search immediately.
    /// `filters` use the compact spec `line+foo`, `line-foo`, `file+foo`,
    /// `file-foo`.
    /// Debug hook: types `query` the way a person does — through the
    /// field editor, so the delegate chain (and its debounce timer)
    /// runs exactly as it would live.
    func debugType(scope: String, query: String) {
        scopeField.stringValue = scope
        refreshFileIndex(force: true)
        queryField.stringValue = ""
        panel?.makeFirstResponder(queryField)
        guard let editor = queryField.currentEditor() else { return }
        for character in query {
            editor.insertText(String(character))
        }
    }

    func debugSet(scope: String, query: String, filters: [String] = []) {
        scopeField.stringValue = scope
        queryField.stringValue = query
        clearFilters()
        for spec in filters {
            guard spec.count > 5 else { continue }
            let kind = String(spec.prefix(4))
            let include = spec.dropFirst(4).first == "+"
            let pattern = String(spec.dropFirst(5))
            let row = makeFilterRow()
            (row.arrangedSubviews[0] as? NSPopUpButton)?.selectItem(
                at: (kind == "line" ? 0 : 2) + (include ? 0 : 1))
            (row.arrangedSubviews[1] as? NSTextField)?.stringValue = pattern
            filtersStack.addArrangedSubview(row)
        }
        runSearch()
    }

    @objc private func openSelection() {
        let index = table.selectedRow >= 0 ? table.selectedRow : 0
        guard rows.indices.contains(index) else { return }
        let row = rows[index]
        panel?.orderOut(nil)
        onOpen?(row.path, row.line)
    }

    private func moveSelection(by delta: Int) {
        guard !rows.isEmpty else { return }
        let next = min(max(table.selectedRow + delta, 0), rows.count - 1)
        table.selectRowIndexes([next], byExtendingSelection: false)
        table.scrollRowToVisible(next)
    }
}

extension QuickFinderPanel: NSTextFieldDelegate {
    func controlTextDidChange(_ notification: Notification) {
        // A new scope needs a new walk; a new query only re-matches.
        if notification.object as AnyObject? === scopeField {
            scheduleIndexRefresh()
        } else {
            scheduleSearch()
        }
    }

    /// Keyboard control from the text fields: arrows move the result
    /// selection, return opens, escape closes.
    func control(
        _ control: NSControl, textView: NSTextView, doCommandBy selector: Selector
    ) -> Bool {
        switch selector {
        case #selector(NSResponder.moveDown(_:)):
            moveSelection(by: 1)
            return true
        case #selector(NSResponder.moveUp(_:)):
            moveSelection(by: -1)
            return true
        case #selector(NSResponder.insertNewline(_:)):
            // Search now (flushing the debounce). ⌘⏎ is what opens —
            // see the hint in the status line.
            debounce?.invalidate()
            if mode != .grep, indexedScope
                != (scopeField.stringValue as NSString).expandingTildeInPath
            {
                refreshFileIndex(force: true)
            } else {
                runSearch()
            }
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            panel?.orderOut(nil)
            return true
        default:
            return false
        }
    }
}

extension QuickFinderPanel: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        rows.count
    }

    func tableView(
        _ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int
    ) -> NSView? {
        let identifier = NSUserInterfaceItemIdentifier("cell")
        let cell =
            tableView.makeView(withIdentifier: identifier, owner: nil) as? NSTextField
            ?? {
                let field = NSTextField(labelWithString: "")
                field.identifier = identifier
                field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
                field.lineBreakMode = .byTruncatingMiddle
                return field
            }()
        cell.stringValue = rows[row].display
        return cell
    }
}
