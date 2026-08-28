import AppKit
import TextchumKit

/// A floating list of source locations — the result of Find References.
/// ↑/↓ move, ⏎ (or double-click) jumps, ⎋ closes.
///
/// Code first, tests after, each under a heading with a count. Ask
/// where a function is used and the answer is usually dominated by its
/// test file; what calls this is the question, and what checks it is
/// the follow-up. A result that is all one or the other gets no
/// headings — a heading over every row it has tells the reader
/// nothing.
@MainActor
final class ReferencesPanel: NSObject {
    static let shared = ReferencesPanel()

    struct Location {
        let path: String
        /// Zero-based, LSP-style.
        let line: Int
        let character: Int
    }

    /// ⏎ jumps; ⎋ falls through to the panel, which closes.
    private final class KeyableTableView: NSTableView {
        var onReturn: (() -> Void)?
        override func keyDown(with event: NSEvent) {
            if event.charactersIgnoringModifiers == "\r" {
                onReturn?()
                return
            }
            super.keyDown(with: event)
        }
    }

    /// A row of the panel: a location to jump to, or a heading to read
    /// past.
    private enum Row {
        case heading(String)
        case location(display: String, location: Location)
    }

    private var panel: NSPanel?
    private let table = KeyableTableView()
    private var rows: [Row] = []
    private var onOpen: ((Location) -> Void)?

    func show(locations: [Location], over window: NSWindow?, onOpen: @escaping (Location) -> Void) {
        self.onOpen = onOpen
        // One read per file, to show the referenced line's text.
        var lineCache: [String: [Substring]] = [:]
        func lineText(_ location: Location) -> String {
            if lineCache[location.path] == nil {
                let contents = (try? String(contentsOfFile: location.path, encoding: .utf8)) ?? ""
                lineCache[location.path] = contents.split(
                    separator: "\n", omittingEmptySubsequences: false)
            }
            let lines = lineCache[location.path] ?? []
            guard lines.indices.contains(location.line) else { return "" }
            return lines[location.line].trimmingCharacters(in: .whitespaces)
        }
        func described(_ location: Location) -> Row {
            let name = (location.path as NSString).lastPathComponent
            return .location(
                display: "\(name):\(location.line + 1): \(lineText(location))",
                location: location)
        }
        // Stable: within each section the server's order stands.
        let code = locations.filter { !CoreReferences.isTest(path: $0.path) }
        let tests = locations.filter { CoreReferences.isTest(path: $0.path) }
        if code.isEmpty || tests.isEmpty {
            rows = locations.map(described)
        } else {
            rows =
                [.heading("Code (\(code.count))")] + code.map(described)
                + [.heading("Tests (\(tests.count))")] + tests.map(described)
        }

        let panel = self.panel ?? makePanel()
        self.panel = panel
        panel.title = "References (\(locations.count))"
        table.reloadData()
        if let first = rows.firstIndex(where: { if case .location = $0 { return true }
            return false })
        {
            table.selectRowIndexes([first], byExtendingSelection: false)
        }
        if let window {
            var frame = panel.frame
            frame.origin.x = window.frame.midX - frame.width / 2
            frame.origin.y = window.frame.maxY - frame.height - 120
            panel.setFrame(frame, display: false)
        } else {
            panel.center()
        }
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(table)
    }

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 320),
            styleMask: [.titled, .closable, .utilityWindow],
            backing: .buffered,
            defer: false
        )
        panel.isFloatingPanel = true

        table.addTableColumn(NSTableColumn(identifier: .init("location")))
        table.onReturn = { [weak self] in self?.openSelection() }
        table.headerView = nil
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.doubleAction = #selector(openSelection)
        table.rowHeight = 22
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.contentInsets = NSEdgeInsets(top: 6, left: 6, bottom: 6, right: 6)
        panel.contentView = scroll
        return panel
    }

    @objc private func openSelection() {
        let index = table.selectedRow >= 0 ? table.selectedRow : 0
        guard rows.indices.contains(index),
            case .location(_, let location) = rows[index]
        else { return }
        panel?.orderOut(nil)
        onOpen?(location)
    }
}

extension ReferencesPanel: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        rows.count
    }

    func tableView(
        _ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int
    ) -> NSView? {
        switch rows[row] {
        case .heading(let title):
            let identifier = NSUserInterfaceItemIdentifier("heading-cell")
            let cell =
                tableView.makeView(withIdentifier: identifier, owner: nil) as? NSTextField
                ?? {
                    let field = NSTextField(labelWithString: "")
                    field.identifier = identifier
                    field.font = .systemFont(ofSize: 11, weight: .semibold)
                    field.textColor = .secondaryLabelColor
                    return field
                }()
            cell.stringValue = title
            return cell
        case .location(let display, _):
            let identifier = NSUserInterfaceItemIdentifier("location-cell")
            let cell =
                tableView.makeView(withIdentifier: identifier, owner: nil) as? NSTextField
                ?? {
                    let field = NSTextField(labelWithString: "")
                    field.identifier = identifier
                    field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
                    field.lineBreakMode = .byTruncatingTail
                    return field
                }()
            cell.stringValue = display
            return cell
        }
    }

    /// Headings are read, not jumped to: a click on one selects
    /// nothing.
    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        if case .location = rows[row] { return true }
        return false
    }

    /// ↑/↓ step over a heading rather than stopping at it. Refusing the
    /// selection is not enough on its own — that leaves the arrow key
    /// doing nothing, and the rows past the heading unreachable.
    func tableView(
        _ tableView: NSTableView, selectionIndexesForProposedSelection proposed: IndexSet
    ) -> IndexSet {
        guard let index = proposed.first, rows.indices.contains(index) else { return proposed }
        if case .location = rows[index] { return proposed }
        let current = tableView.selectedRow
        let step = index >= current ? 1 : -1
        var at = index + step
        while rows.indices.contains(at) {
            if case .location = rows[at] { return IndexSet(integer: at) }
            at += step
        }
        // Nothing past the heading in that direction: stay put.
        return current >= 0 ? IndexSet(integer: current) : IndexSet()
    }
}
