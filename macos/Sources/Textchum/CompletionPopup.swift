import AppKit

/// The completion popup: a borderless child window under the caret with
/// the server's suggestions, filtered live as the user keeps typing.
///
/// The popup never owns the keyboard — the text view keeps first
/// responder, and the window controller forwards ↑/↓/⏎/⎋ here while the
/// popup is visible, so typing continues to flow through the normal
/// (core-synchronized) edit path.
@MainActor
final class CompletionPopup: NSObject {
    /// One suggestion, reduced from an LSP CompletionItem.
    struct Item {
        let label: String
        let detail: String
        /// What the server asked to be inserted, verbatim: a snippet
        /// body when `isSnippet`, plain text otherwise. Expanding a body
        /// is the core's job, so both shells expand it the same way.
        let insertText: String
        let isSnippet: Bool
        let sortText: String
        let filterText: String
    }

    private var window: NSWindow?
    private let table = NSTableView()
    private var allItems: [Item] = []
    private var filtered: [Item] = []
    private(set) var isVisible = false
    /// Called with the chosen item when the user accepts.
    var onAccept: ((Item) -> Void)?

    // MARK: Parsing

    /// Reduces an LSP completion response (`CompletionItem[]` or
    /// `CompletionList`) to items, sorted by the server's preference.
    /// Snippet bodies are carried through unexpanded; the core expands
    /// them when one is accepted.
    static func parse(resultJSON: String) -> [Item] {
        guard let data = resultJSON.data(using: .utf8),
            let parsed = try? JSONSerialization.jsonObject(with: data)
        else { return [] }
        let rawItems: [[String: Any]]
        if let list = parsed as? [String: Any] {
            rawItems = list["items"] as? [[String: Any]] ?? []
        } else {
            rawItems = parsed as? [[String: Any]] ?? []
        }
        return rawItems.compactMap { raw -> Item? in
            guard let label = raw["label"] as? String else { return nil }
            let insert =
                (raw["textEdit"] as? [String: Any])?["newText"] as? String
                ?? raw["insertText"] as? String
                ?? label
            return Item(
                label: label,
                detail: raw["detail"] as? String ?? "",
                insertText: insert,
                isSnippet: Self.isSnippet(raw, insert: insert),
                sortText: raw["sortText"] as? String ?? label,
                filterText: (raw["filterText"] as? String ?? label).lowercased()
            )
        }
        .sorted { $0.sortText < $1.sortText }
    }

    /// Whether `insert` is a snippet body. `insertTextFormat` 2 says so
    /// outright. Servers that leave the field out and write placeholders
    /// anyway are common enough that a body which plainly contains one
    /// is taken at its word; a lone `$`, as in a shell variable, is not.
    private static func isSnippet(_ raw: [String: Any], insert: String) -> Bool {
        if let format = raw["insertTextFormat"] as? Int { return format == 2 }
        let characters = Array(insert)
        var index = 0
        while index + 1 < characters.count {
            if characters[index] == "\\" {
                index += 2
                continue
            }
            if characters[index] == "$",
                characters[index + 1] == "{" || characters[index + 1].isNumber
            {
                return true
            }
            index += 1
        }
        return false
    }

    // MARK: Presentation

    /// Shows the popup with `items`, pre-filtered by `prefix`, anchored
    /// below the caret's screen rectangle.
    func show(items: [Item], prefix: String, below caretRect: NSRect, parent: NSWindow) {
        allItems = items
        buildWindowIfNeeded(parent: parent)
        filter(prefix: prefix)
        guard !filtered.isEmpty, let window else {
            dismiss()
            return
        }
        let height = min(CGFloat(filtered.count), 9) * (table.rowHeight + 2) + 8
        let originY = caretRect.minY - height - 4
        window.setFrame(
            NSRect(x: caretRect.minX - 24, y: originY, width: 460, height: height),
            display: true
        )
        if window.parent == nil {
            parent.addChildWindow(window, ordered: .above)
        }
        window.orderFront(nil)
        isVisible = true
    }

    /// Refilters against a new prefix; dismisses when nothing matches.
    func filter(prefix: String) {
        let needle = prefix.lowercased()
        filtered =
            needle.isEmpty
            ? allItems
            : allItems.filter { $0.filterText.contains(needle) }
                .sorted {
                    // Prefix matches first, then server order.
                    let a = $0.filterText.hasPrefix(needle)
                    let b = $1.filterText.hasPrefix(needle)
                    if a != b { return a }
                    return $0.sortText < $1.sortText
                }
        if filtered.isEmpty {
            dismiss()
            return
        }
        table.reloadData()
        table.selectRowIndexes([0], byExtendingSelection: false)
        table.scrollRowToVisible(0)
        if isVisible, let window {
            var frame = window.frame
            let height = min(CGFloat(filtered.count), 9) * (table.rowHeight + 2) + 8
            frame.origin.y += frame.height - height
            frame.size.height = height
            window.setFrame(frame, display: true)
        }
    }

    func dismiss() {
        guard let window else {
            isVisible = false
            return
        }
        window.parent?.removeChildWindow(window)
        window.orderOut(nil)
        isVisible = false
    }

    func moveSelection(by delta: Int) {
        guard !filtered.isEmpty else { return }
        let next = min(max(table.selectedRow + delta, 0), filtered.count - 1)
        table.selectRowIndexes([next], byExtendingSelection: false)
        table.scrollRowToVisible(next)
    }

    /// Accepts the current selection, if any.
    func acceptSelection() {
        let index = table.selectedRow >= 0 ? table.selectedRow : 0
        guard filtered.indices.contains(index) else { return }
        let item = filtered[index]
        dismiss()
        onAccept?(item)
    }

    private func buildWindowIfNeeded(parent: NSWindow) {
        guard window == nil else { return }
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 460, height: 160),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = true
        window.level = .floating

        table.addTableColumn(NSTableColumn(identifier: .init("completion")))
        table.headerView = nil
        table.dataSource = self
        table.delegate = self
        table.rowHeight = 20
        table.target = self
        table.doubleAction = #selector(tableDoubleClicked)
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false

        let container = NSVisualEffectView()
        container.material = .menu
        container.state = .active
        container.wantsLayer = true
        container.layer?.cornerRadius = 8
        container.layer?.masksToBounds = true
        container.addSubview(scroll)
        scroll.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: container.topAnchor, constant: 4),
            scroll.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -4),
            scroll.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 4),
            scroll.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -4),
        ])
        window.contentView = container
        self.window = window
    }

    @objc private func tableDoubleClicked() {
        acceptSelection()
    }
}

extension CompletionPopup: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        filtered.count
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
                field.lineBreakMode = .byTruncatingTail
                return field
            }()
        let item = filtered[row]
        let text = NSMutableAttributedString(string: item.label)
        if !item.detail.isEmpty {
            text.append(
                NSAttributedString(
                    string: "  \(item.detail)",
                    attributes: [
                        .foregroundColor: NSColor.secondaryLabelColor,
                        .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular),
                    ]))
        }
        cell.attributedStringValue = text
        return cell
    }
}
