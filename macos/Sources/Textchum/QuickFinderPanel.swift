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

        let stack = NSStackView(views: [scopeField, queryField, scroll])
        stack.orientation = .vertical
        stack.spacing = 6
        stack.edgeInsets = NSEdgeInsets(top: 10, left: 10, bottom: 10, right: 10)
        stack.distribution = .fill
        scroll.setContentHuggingPriority(.defaultLow, for: .vertical)
        panel.contentView = stack
        return panel
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
                    let hits = (try? CoreSearch.grep(root: scope, pattern: query, limit: 200))
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
    func debugSet(scope: String, query: String) {
        scopeField.stringValue = scope
        queryField.stringValue = query
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
