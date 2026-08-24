import AppKit

/// The command palette (⇧⌘P): every menu action, fuzzy-searchable from
/// the keyboard. Commands are harvested from the main menu when the
/// palette opens, so it always matches what the menus offer — including
/// dynamic entries like Open Recent — and each row shows its menu path
/// and keyboard shortcut. ↑/↓ move, ⏎ runs, ⎋ closes.
@MainActor
final class CommandPalettePanel: NSObject {
    private struct Command {
        /// "File › Save As…"
        let title: String
        let shortcut: String
        let action: Selector
        let target: AnyObject?
        let representedItem: NSMenuItem
    }

    private var panel: NSPanel?
    private let queryField = NSTextField()
    private let table = NSTableView()
    private var all: [Command] = []
    private var rows: [Command] = []

    func show(over window: NSWindow?) {
        all = Self.harvest()
        let panel = self.panel ?? makePanel()
        self.panel = panel
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

    /// Every actionable item of the main menu, depth-first, titled with
    /// its menu path.
    private static func harvest() -> [Command] {
        var commands: [Command] = []
        func walk(_ menu: NSMenu, path: [String]) {
            for item in menu.items {
                if item.isSeparatorItem || item.isHidden { continue }
                if let submenu = item.submenu {
                    walk(submenu, path: path + [item.title])
                } else if let action = item.action {
                    commands.append(
                        Command(
                            title: (path + [item.title]).joined(separator: " › "),
                            shortcut: shortcutLabel(of: item),
                            action: action,
                            target: item.target as AnyObject?,
                            representedItem: item
                        ))
                }
            }
        }
        if let mainMenu = NSApp.mainMenu {
            // The root's items are the menu-bar titles; the submenu's own
            // title is the authoritative one (holder items are untitled).
            for top in mainMenu.items {
                if let submenu = top.submenu {
                    walk(
                        submenu,
                        path: [submenu.title.isEmpty ? top.title : submenu.title])
                }
            }
        }
        return commands
    }

    private static func shortcutLabel(of item: NSMenuItem) -> String {
        guard !item.keyEquivalent.isEmpty else { return "" }
        var label = ""
        let mask = item.keyEquivalentModifierMask
        if mask.contains(.control) { label += "⌃" }
        if mask.contains(.option) { label += "⌥" }
        if mask.contains(.shift) { label += "⇧" }
        if mask.contains(.command) { label += "⌘" }
        let key: String
        switch item.keyEquivalent {
        case "\u{1b}": key = "⎋"
        case "\r": key = "⏎"
        case "\t": key = "⇥"
        case " ": key = "␣"
        case String(UnicodeScalar(NSUpArrowFunctionKey)!): key = "↑"
        case String(UnicodeScalar(NSDownArrowFunctionKey)!): key = "↓"
        case String(UnicodeScalar(NSLeftArrowFunctionKey)!): key = "←"
        case String(UnicodeScalar(NSRightArrowFunctionKey)!): key = "→"
        default: key = item.keyEquivalent.uppercased()
        }
        return label + key
    }

    // MARK: Filtering

    private func applyFilter() {
        let query = queryField.stringValue
        rows =
            all
            .compactMap { command in
                Fuzzy.score(command.title, query: query).map { (command, $0) }
            }
            .sorted { $0.1 > $1.1 }
            .map(\.0)
        table.reloadData()
        if !rows.isEmpty {
            table.selectRowIndexes([0], byExtendingSelection: false)
            table.scrollRowToVisible(0)
        }
    }

    // MARK: Panel

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 380),
            styleMask: [.titled, .closable, .utilityWindow],
            backing: .buffered,
            defer: false
        )
        panel.title = "Command Palette"
        panel.isFloatingPanel = true
        panel.becomesKeyOnlyIfNeeded = false

        queryField.font = .systemFont(ofSize: 15)
        queryField.placeholderString = "command…"
        queryField.delegate = self

        table.addTableColumn(NSTableColumn(identifier: .init("command")))
        table.headerView = nil
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.doubleAction = #selector(runSelection)
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

    @objc private func runSelection() {
        let index = table.selectedRow >= 0 ? table.selectedRow : 0
        guard rows.indices.contains(index) else { return }
        let command = rows[index]
        panel?.orderOut(nil)
        // Through the responder chain when the item has no fixed target,
        // exactly as the menu itself would dispatch it.
        NSApp.sendAction(command.action, to: command.target, from: command.representedItem)
    }

    /// Debug hook: force a query, for screenshot-driven verification.
    func debugSet(query: String) {
        queryField.stringValue = query
        applyFilter()
    }

    private func moveSelection(by delta: Int) {
        guard !rows.isEmpty else { return }
        let next = min(max(table.selectedRow + delta, 0), rows.count - 1)
        table.selectRowIndexes([next], byExtendingSelection: false)
        table.scrollRowToVisible(next)
    }
}

extension CommandPalettePanel: NSTextFieldDelegate {
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
            runSelection()
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            panel?.orderOut(nil)
            return true
        default:
            return false
        }
    }
}

extension CommandPalettePanel: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        rows.count
    }

    func tableView(
        _ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int
    ) -> NSView? {
        let identifier = NSUserInterfaceItemIdentifier("command-cell")
        let cell: NSStackView
        let title: NSTextField
        let shortcut: NSTextField
        if let reused = tableView.makeView(withIdentifier: identifier, owner: nil)
            as? NSStackView,
            reused.arrangedSubviews.count == 2,
            let reusedTitle = reused.arrangedSubviews[0] as? NSTextField,
            let reusedShortcut = reused.arrangedSubviews[1] as? NSTextField
        {
            cell = reused
            title = reusedTitle
            shortcut = reusedShortcut
        } else {
            title = NSTextField(labelWithString: "")
            title.font = .systemFont(ofSize: 13)
            title.lineBreakMode = .byTruncatingTail
            shortcut = NSTextField(labelWithString: "")
            shortcut.font = .systemFont(ofSize: 12)
            shortcut.textColor = .secondaryLabelColor
            shortcut.alignment = .right
            cell = NSStackView(views: [title, shortcut])
            cell.orientation = .horizontal
            cell.identifier = identifier
        }
        title.stringValue = rows[row].title
        shortcut.stringValue = rows[row].shortcut
        return cell
    }
}
