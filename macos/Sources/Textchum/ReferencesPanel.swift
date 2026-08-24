import AppKit

/// A floating list of source locations — the result of Find References.
/// ↑/↓ move, ⏎ (or double-click) jumps, ⎋ closes.
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

    private var panel: NSPanel?
    private let table = KeyableTableView()
    private var rows: [(display: String, location: Location)] = []
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
        rows = locations.map { location in
            let name = (location.path as NSString).lastPathComponent
            return ("\(name):\(location.line + 1): \(lineText(location))", location)
        }

        let panel = self.panel ?? makePanel()
        self.panel = panel
        panel.title = "References (\(rows.count))"
        table.reloadData()
        if !rows.isEmpty {
            table.selectRowIndexes([0], byExtendingSelection: false)
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
        guard rows.indices.contains(index) else { return }
        panel?.orderOut(nil)
        onOpen?(rows[index].location)
    }
}

extension ReferencesPanel: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        rows.count
    }

    func tableView(
        _ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int
    ) -> NSView? {
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
        cell.stringValue = rows[row].display
        return cell
    }
}
