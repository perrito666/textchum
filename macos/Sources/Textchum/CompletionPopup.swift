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
        let insertText: String
        /// Where the caret belongs after insertion — the first snippet
        /// tabstop's placeholder (selected so typing replaces it), or
        /// the `$0` exit point. Relative to `insertText`, UTF-16 units.
        let selection: NSRange?
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
    /// Snippet placeholders (`${1:x}`, `$0`) are flattened to plain text.
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
            let (expanded, selection) = Self.expandSnippet(insert)
            return Item(
                label: label,
                detail: raw["detail"] as? String ?? "",
                insertText: expanded,
                selection: selection,
                sortText: raw["sortText"] as? String ?? label,
                filterText: (raw["filterText"] as? String ?? label).lowercased()
            )
        }
        .sorted { $0.sortText < $1.sortText }
    }

    /// Expands LSP snippet syntax to plain text and remembers where the
    /// caret should land: `${1:placeholder}` keeps its placeholder (the
    /// lowest-numbered one comes back selected, so typing replaces it),
    /// bare `$1`/`$0` vanish (`$0` marking the exit point), and `\$`
    /// stays a dollar sign. Later tabstops are plain text — one honest
    /// stop, not a tabstop mode.
    static func expandSnippet(_ text: String) -> (text: String, selection: NSRange?) {
        var out = ""
        out.reserveCapacity(text.count)
        // (tabstop number, location, length) in UTF-16 units of `out`.
        var stops: [(number: Int, location: Int, length: Int)] = []
        var scanner = Substring(text)
        func utf16Length(_ string: String) -> Int { string.utf16.count }
        while let dollar = scanner.firstIndex(of: "$") {
            let before = scanner[..<dollar]
            // A backslash right before the dollar escapes it.
            if before.last == "\\" {
                out += before.dropLast()
                out += "$"
                scanner = scanner[scanner.index(after: dollar)...]
                continue
            }
            out += before
            var rest = scanner[scanner.index(after: dollar)...]
            if rest.first == "{" {
                // ${n} or ${n:placeholder} (no nesting).
                guard let close = rest.firstIndex(of: "}") else {
                    out += "$"
                    scanner = rest
                    continue
                }
                let body = rest[rest.index(after: rest.startIndex)..<close]
                let halves = body.split(separator: ":", maxSplits: 1)
                let number = Int(halves.first ?? "") ?? 0
                let placeholder = halves.count > 1 ? String(halves[1]) : ""
                stops.append(
                    (number, utf16Length(out), utf16Length(placeholder)))
                out += placeholder
                scanner = rest[rest.index(after: close)...]
            } else {
                var digits = ""
                while let first = rest.first, first.isNumber {
                    digits.append(first)
                    rest = rest.dropFirst()
                }
                if digits.isEmpty {
                    out += "$"
                } else {
                    stops.append((Int(digits) ?? 0, utf16Length(out), 0))
                }
                scanner = rest
            }
        }
        out += scanner
        // The first real tabstop wins; $0 (the exit point) is the
        // fallback caret position.
        let first = stops
            .filter { $0.number > 0 }
            .min { $0.number < $1.number }
            ?? stops.first { $0.number == 0 }
        return (
            out,
            first.map { NSRange(location: $0.location, length: $0.length) }
        )
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
