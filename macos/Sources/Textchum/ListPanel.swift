import AppKit

/// The floating list every jump-to-somewhere command shows: references,
/// the document outline, the diagnostics. Arrow through it, press ⏎ to
/// go, ⎋ to close.
///
/// One panel rather than one per command. Find References and Document
/// Outline were written separately and had already drifted — the
/// outline filtered as you type and references did not, which nothing
/// had decided. A third list would have made that three.
///
/// Callers describe rows and get back the index of the item they
/// supplied. Headings are the panel's business: a caller never learns
/// one was in the list.
@MainActor
final class ListPanel: NSObject {
    /// A row: something to choose, or a heading to read past.
    enum Row {
        case heading(String)
        case item(String)
    }

    /// ⏎ chooses; ⎋ falls through to the panel, which closes.
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

    /// The field forwards the arrows and ⏎ to the list, so the list can
    /// be walked without leaving what you are typing.
    private final class QueryField: NSTextField {
        var onKey: ((NSEvent) -> Bool)?
        override func keyDown(with event: NSEvent) {
            if onKey?(event) == true { return }
            super.keyDown(with: event)
        }
    }

    private var panel: NSPanel?
    private let queryField = QueryField()
    private let table = KeyableTableView()
    private let scroll = NSScrollView()
    /// Every row given, headings included.
    private var allRows: [Row] = []
    /// What is on screen: the row, and the item index when it is one.
    private var shown: [(row: Row, item: Int?)] = []
    private var monospaced = false
    private var filtering = false
    private var onChoose: ((Int) -> Void)?

    /// Shows `rows`. `placeholder` gives the list a filter field;
    /// nil leaves it out, which is right when the list is short enough
    /// to read. `onChoose` is handed the index of the chosen item among
    /// the items of `rows`, in the order they were given.
    func show(
        rows: [Row],
        over window: NSWindow?,
        title: String,
        placeholder: String? = nil,
        monospaced: Bool = false,
        onChoose: @escaping (Int) -> Void
    ) {
        self.allRows = rows
        self.monospaced = monospaced
        self.onChoose = onChoose
        self.filtering = placeholder != nil

        let panel = self.panel ?? makePanel()
        self.panel = panel
        panel.title = title
        queryField.isHidden = placeholder == nil
        queryField.placeholderString = placeholder ?? ""
        queryField.stringValue = ""
        layOut(in: panel)
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
        panel.makeFirstResponder(filtering ? queryField : table)
    }

    func close() {
        panel?.orderOut(nil)
    }

    // MARK: Rows

    private func applyFilter() {
        let query = filtering ? queryField.stringValue : ""
        var built: [(row: Row, item: Int?)] = []
        var item = 0
        for row in allRows {
            switch row {
            case .heading:
                // A heading over a filtered list describes a section
                // that is no longer there; it goes while filtering.
                if query.isEmpty { built.append((row, nil)) }
            case .item(let text):
                if query.isEmpty || Fuzzy.score(text, query: query) != nil {
                    built.append((row, item))
                }
                item += 1
            }
        }
        if !query.isEmpty {
            built.sort {
                let left = Fuzzy.score(Self.text(of: $0.row), query: query) ?? 0
                let right = Fuzzy.score(Self.text(of: $1.row), query: query) ?? 0
                return left > right
            }
        }
        shown = built
        table.reloadData()
        selectFirstItem()
    }

    private static func text(of row: Row) -> String {
        switch row {
        case .heading(let text), .item(let text): return text
        }
    }

    private func selectFirstItem() {
        guard let first = shown.firstIndex(where: { $0.item != nil }) else { return }
        table.selectRowIndexes([first], byExtendingSelection: false)
        table.scrollRowToVisible(first)
    }

    @objc private func chooseSelection() {
        let row = table.selectedRow
        guard shown.indices.contains(row), let item = shown[row].item else { return }
        panel?.orderOut(nil)
        onChoose?(item)
    }

    /// Moves the selection by `step`, over headings rather than into
    /// them.
    private func moveSelection(by step: Int) {
        var at = table.selectedRow + step
        while shown.indices.contains(at) {
            if shown[at].item != nil {
                table.selectRowIndexes([at], byExtendingSelection: false)
                table.scrollRowToVisible(at)
                return
            }
            at += step
        }
    }

    // MARK: Panel

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 360),
            styleMask: [.titled, .closable, .resizable, .utilityWindow],
            backing: .buffered,
            defer: false
        )
        panel.isFloatingPanel = true
        panel.contentMinSize = NSSize(width: 360, height: 200)

        queryField.delegate = self
        queryField.onKey = { [weak self] event in
            guard let self else { return false }
            switch event.keyCode {
            case 125: self.moveSelection(by: 1); return true  // down
            case 126: self.moveSelection(by: -1); return true  // up
            case 36, 76: self.chooseSelection(); return true  // return, enter
            default: return false
            }
        }

        table.addTableColumn(NSTableColumn(identifier: .init("row")))
        table.onReturn = { [weak self] in self?.chooseSelection() }
        table.headerView = nil
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.doubleAction = #selector(chooseSelection)
        table.rowHeight = 22
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.contentInsets = NSEdgeInsets(top: 6, left: 6, bottom: 6, right: 6)
        return panel
    }

    /// The field is there or it is not, and the list fills what is
    /// left.
    private func layOut(in panel: NSPanel) {
        let content = NSView()
        queryField.translatesAutoresizingMaskIntoConstraints = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(scroll)
        var constraints: [NSLayoutConstraint] = [
            scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: content.bottomAnchor),
        ]
        if filtering {
            content.addSubview(queryField)
            constraints += [
                queryField.topAnchor.constraint(equalTo: content.topAnchor, constant: 10),
                queryField.leadingAnchor.constraint(
                    equalTo: content.leadingAnchor, constant: 10),
                queryField.trailingAnchor.constraint(
                    equalTo: content.trailingAnchor, constant: -10),
                scroll.topAnchor.constraint(
                    equalTo: queryField.bottomAnchor, constant: 8),
            ]
        } else {
            constraints.append(scroll.topAnchor.constraint(equalTo: content.topAnchor))
        }
        NSLayoutConstraint.activate(constraints)
        panel.contentView = content
    }
}

extension ListPanel: NSTableViewDataSource, NSTableViewDelegate, NSTextFieldDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int { shown.count }

    func tableView(
        _ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int
    ) -> NSView? {
        let isHeading = shown[row].item == nil
        let identifier = NSUserInterfaceItemIdentifier(isHeading ? "heading" : "item")
        let cell =
            tableView.makeView(withIdentifier: identifier, owner: nil) as? NSTextField
            ?? {
                let field = NSTextField(labelWithString: "")
                field.identifier = identifier
                field.lineBreakMode = .byTruncatingTail
                if isHeading {
                    field.font = .systemFont(ofSize: 11, weight: .semibold)
                    field.textColor = .secondaryLabelColor
                } else {
                    field.font =
                        self.monospaced
                        ? .monospacedSystemFont(ofSize: 12, weight: .regular)
                        : .systemFont(ofSize: 13)
                }
                return field
            }()
        cell.stringValue = Self.text(of: shown[row].row)
        return cell
    }

    /// Headings are read, not chosen: a click on one selects nothing.
    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        shown[row].item != nil
    }

    /// ↑/↓ step over a heading rather than stopping at it. Refusing the
    /// selection is not enough on its own — that leaves the arrow key
    /// doing nothing and the rows past the heading unreachable.
    func tableView(
        _ tableView: NSTableView, selectionIndexesForProposedSelection proposed: IndexSet
    ) -> IndexSet {
        guard let index = proposed.first, shown.indices.contains(index) else { return proposed }
        if shown[index].item != nil { return proposed }
        let current = tableView.selectedRow
        let step = index >= current ? 1 : -1
        var at = index + step
        while shown.indices.contains(at) {
            if shown[at].item != nil { return IndexSet(integer: at) }
            at += step
        }
        return current >= 0 ? IndexSet(integer: current) : IndexSet()
    }

    func controlTextDidChange(_ notification: Notification) {
        applyFilter()
    }
}
