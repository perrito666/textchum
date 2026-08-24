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

        var title: String {
            switch self {
            case .files: "Open Quickly"
            case .grep: "Find in Project"
            }
        }

        var placeholder: String {
            switch self {
            case .files: "fuzzy file name…"
            case .grep: "regular expression…"
            }
        }
    }

    private var panel: NSPanel?
    private var mode: Mode = .files
    private let scopeField = NSTextField()
    private let queryField = NSTextField()
    /// Stacked refinements (grep mode): one row per filter, below the
    /// query, narrowing by line content or file name.
    private let filtersStack = NSStackView()
    private let addFilterButton = NSButton(
        title: "＋ Add Filter", target: nil, action: nil)
    private let table = NSTableView()
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
        queryField.placeholderString = mode.placeholder
        queryField.stringValue = ""
        rows = []
        table.reloadData()
        clearFilters()
        // Filters only make sense over content hits.
        filtersStack.isHidden = mode == .files
        addFilterButton.isHidden = mode == .files

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
        runSearch()
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

        let stack = NSStackView(views: [scopeField, queryField, filtersStack, addRow, scroll])
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

    private func scheduleSearch() {
        debounce?.invalidate()
        debounce = Timer.scheduledTimer(withTimeInterval: 0.15, repeats: false) {
            [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.runSearch() }
            }
        }
    }

    private func runSearch() {
        let scope = (scopeField.stringValue as NSString).expandingTildeInPath
        let query = queryField.stringValue
        let mode = self.mode
        let filters = currentFilters()
        searchGeneration += 1
        let generation = searchGeneration

        // Pure core functions, run off the main thread; stale results
        // (an older generation) are dropped on arrival.
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let results: [(String, String, Int)]
            switch mode {
            case .files:
                results = CoreSearch.fuzzyFiles(root: scope, query: query, limit: 100)
                    .map { ($0, "\(scope)/\($0)", 0) }
            case .grep:
                if query.isEmpty {
                    results = []
                } else {
                    let hits = try? CoreSearch.grep(
                        root: scope, pattern: query, limit: 200, filters: filters)
                    results = (hits ?? []).map {
                        ("\($0.path):\($0.line): \($0.text)", "\(scope)/\($0.path)", $0.line)
                    }
                }
            }
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    guard let self, self.searchGeneration == generation else { return }
                    self.rows = results
                    self.table.reloadData()
                    if !results.isEmpty {
                        self.table.selectRowIndexes([0], byExtendingSelection: false)
                    }
                }
            }
        }
    }

    /// Debug hook: force scope and query and search immediately.
    /// `filters` use the compact spec `line+foo`, `line-foo`, `file+foo`,
    /// `file-foo`.
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
        scheduleSearch()
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
            openSelection()
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
