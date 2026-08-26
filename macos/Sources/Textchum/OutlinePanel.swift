import AppKit

/// The document outline (⇧⌘O): the file's symbols from its language
/// server, fuzzy-filterable, nesting shown by indentation. ↑/↓ move,
/// ⏎ jumps (through the jump stack), ⎋ closes.
@MainActor
final class OutlinePanel: NSObject {
    static let shared = OutlinePanel()

    struct Symbol {
        let name: String
        let kind: String
        /// Zero-based, LSP-style.
        let line: Int
        let character: Int
        let depth: Int
    }

    private var panel: NSPanel?
    private let queryField = NSTextField()
    private let table = NSTableView()
    private var all: [Symbol] = []
    private var rows: [Symbol] = []
    private var onSelect: ((Symbol) -> Void)?

    func show(
        symbols: [Symbol], over window: NSWindow?,
        title: String = "Document Outline", placeholder: String = "symbol…",
        onSelect: @escaping (Symbol) -> Void
    ) {
        self.all = symbols
        self.onSelect = onSelect
        let panel = self.panel ?? makePanel()
        self.panel = panel
        panel.title = title
        queryField.placeholderString = placeholder
        queryField.stringValue = ""
        applyFilter()

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
    }

    /// Parses a documentSymbol result: `DocumentSymbol[]` (hierarchical,
    /// flattened depth-first) or `SymbolInformation[]` (already flat).
    static func symbols(fromResultJSON json: String) -> [Symbol] {
        guard let data = json.data(using: .utf8),
            let array = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]]
        else { return [] }
        var flattened: [Symbol] = []
        func position(of raw: [String: Any], key: String) -> (Int, Int)? {
            guard let range = raw[key] as? [String: Any],
                let start = range["start"] as? [String: Any],
                let line = start["line"] as? Int,
                let character = start["character"] as? Int
            else { return nil }
            return (line, character)
        }
        func walk(_ nodes: [[String: Any]], depth: Int) {
            for node in nodes {
                guard let name = node["name"] as? String else { continue }
                let kind = kindLabel(node["kind"] as? Int ?? 0)
                if let position = position(of: node, key: "selectionRange")
                    ?? position(of: node, key: "range")
                {
                    // DocumentSymbol: ranges live on the node itself.
                    flattened.append(
                        Symbol(
                            name: name, kind: kind,
                            line: position.0, character: position.1, depth: depth))
                    if let children = node["children"] as? [[String: Any]] {
                        walk(children, depth: depth + 1)
                    }
                } else if let location = node["location"] as? [String: Any],
                    let position = position(of: location, key: "range")
                {
                    // SymbolInformation: flat, with a Location.
                    flattened.append(
                        Symbol(
                            name: name, kind: kind,
                            line: position.0, character: position.1, depth: 0))
                }
            }
        }
        walk(array, depth: 0)
        return flattened
    }

    private static func kindLabel(_ kind: Int) -> String {
        switch kind {
        case 1: "file"
        case 2: "module"
        case 3: "namespace"
        case 4: "package"
        case 5: "class"
        case 6: "method"
        case 7: "property"
        case 8: "field"
        case 9: "constructor"
        case 10: "enum"
        case 11: "interface"
        case 12: "function"
        case 13: "variable"
        case 14: "constant"
        case 22: "enum member"
        case 23: "struct"
        case 25: "operator"
        case 26: "type parameter"
        default: ""
        }
    }

    // MARK: Filtering

    private func applyFilter() {
        let query = queryField.stringValue
        if query.isEmpty {
            // Unfiltered, the outline keeps document order and nesting.
            rows = all
        } else {
            rows =
                all
                .compactMap { symbol in
                    Fuzzy.score(symbol.name, query: query).map { (symbol, $0) }
                }
                .sorted { $0.1 > $1.1 }
                .map(\.0)
        }
        table.reloadData()
        if !rows.isEmpty {
            table.selectRowIndexes([0], byExtendingSelection: false)
            table.scrollRowToVisible(0)
        }
    }

    // MARK: Panel

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 480, height: 380),
            styleMask: [.titled, .closable, .utilityWindow],
            backing: .buffered,
            defer: false
        )
        panel.title = "Document Outline"
        panel.isFloatingPanel = true
        panel.becomesKeyOnlyIfNeeded = false

        queryField.font = .systemFont(ofSize: 15)
        queryField.placeholderString = "symbol…"
        queryField.delegate = self

        table.addTableColumn(NSTableColumn(identifier: .init("symbol")))
        table.headerView = nil
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.doubleAction = #selector(jumpToSelection)
        table.rowHeight = 22
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true

        let stack = NSStackView(views: [queryField, scroll])
        stack.orientation = .vertical
        stack.spacing = 6
        stack.edgeInsets = NSEdgeInsets(top: 10, left: 10, bottom: 10, right: 10)
        scroll.setContentHuggingPriority(.defaultLow, for: .vertical)
        panel.contentView = stack
        return panel
    }

    @objc private func jumpToSelection() {
        let index = table.selectedRow >= 0 ? table.selectedRow : 0
        guard rows.indices.contains(index) else { return }
        let symbol = rows[index]
        panel?.orderOut(nil)
        onSelect?(symbol)
    }

    private func moveSelection(by delta: Int) {
        guard !rows.isEmpty else { return }
        let next = min(max(table.selectedRow + delta, 0), rows.count - 1)
        table.selectRowIndexes([next], byExtendingSelection: false)
        table.scrollRowToVisible(next)
    }
}

extension OutlinePanel: NSTextFieldDelegate {
    func controlTextDidChange(_ notification: Notification) {
        applyFilter()
    }

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
            jumpToSelection()
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            panel?.orderOut(nil)
            return true
        default:
            return false
        }
    }
}

extension OutlinePanel: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        rows.count
    }

    func tableView(
        _ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int
    ) -> NSView? {
        let identifier = NSUserInterfaceItemIdentifier("outline-cell")
        let cell: NSStackView
        let name: NSTextField
        let kind: NSTextField
        if let reused = tableView.makeView(withIdentifier: identifier, owner: nil)
            as? NSStackView,
            reused.arrangedSubviews.count == 2,
            let reusedName = reused.arrangedSubviews[0] as? NSTextField,
            let reusedKind = reused.arrangedSubviews[1] as? NSTextField
        {
            cell = reused
            name = reusedName
            kind = reusedKind
        } else {
            name = NSTextField(labelWithString: "")
            name.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            name.lineBreakMode = .byTruncatingTail
            kind = NSTextField(labelWithString: "")
            kind.font = .systemFont(ofSize: 11)
            kind.textColor = .secondaryLabelColor
            kind.alignment = .right
            cell = NSStackView(views: [name, kind])
            cell.orientation = .horizontal
            cell.identifier = identifier
        }
        let symbol = rows[row]
        let indent = queryField.stringValue.isEmpty
            ? String(repeating: "    ", count: symbol.depth) : ""
        name.stringValue = indent + symbol.name
        kind.stringValue = symbol.kind
        return cell
    }
}
