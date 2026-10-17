import AppKit
import TextchumKit

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
        /// `styled`, when given, is what the row shows; `text` is what
        /// the filter matches and what the row falls back to.
        case item(String, styled: NSAttributedString? = nil)
    }

    /// ⏎ chooses; ⎋ falls through to the panel, which closes. A letter
    /// typed at a list without a field starts filtering it.
    private final class KeyableTableView: NSTableView {
        var onReturn: (() -> Void)?
        var onType: ((String) -> Void)?
        override func keyDown(with event: NSEvent) {
            if event.charactersIgnoringModifiers == "\r" {
                onReturn?()
                return
            }
            if let typed = event.characters, !typed.isEmpty,
                event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
                typed.unicodeScalars.allSatisfy({
                    !CharacterSet.controlCharacters.contains($0) && $0.value < 0xF700
                })
            {
                onType?(typed)
                return
            }
            super.keyDown(with: event)
        }
    }

    private var panel: NSPanel?
    private let queryField = NSTextField()
    private let table = KeyableTableView()
    private let scroll = NSScrollView()
    /// Every row given, headings included.
    private var allRows: [Row] = []
    /// What is on screen: the row, and the item index when it is one.
    private var shown: [(row: Row, item: Int?)] = []
    private var monospaced = false
    private var filtering = false
    private var onChoose: ((Int) -> Void)?
    /// Where an item goes, for the lists of places: the place is shown
    /// beside the list, so the right one can be told before going.
    private var placeOf: ((Int) -> (path: String, line: Int)?)?
    private let previewScroll = NSScrollView()
    private let previewText = NSTextView()

    /// Shows `rows`. `placeholder` gives the list a filter field;
    /// nil leaves it out, which is right when the list is short enough
    /// to read. `onChoose` is handed the index of the chosen item among
    /// the items of `rows`, in the order they were given.
    ///
    /// `place`, for a list of places, says where an item goes; the
    /// selected one is then shown beside the list.
    func show(
        rows: [Row],
        over window: NSWindow?,
        title: String,
        placeholder: String? = nil,
        monospaced: Bool = false,
        place: ((Int) -> (path: String, line: Int)?)? = nil,
        onChoose: @escaping (Int) -> Void
    ) {
        self.allRows = rows
        self.monospaced = monospaced
        self.onChoose = onChoose
        self.filtering = placeholder != nil
        self.placeOf = place

        let panel = self.panel ?? makePanel()
        self.panel = panel
        panel.title = title
        queryField.isHidden = placeholder == nil
        queryField.placeholderString = placeholder ?? ""
        queryField.stringValue = ""
        layOut(in: panel)
        // Wide enough for a preview beside the list, when there is one.
        var size = panel.frame.size
        size.width = place == nil ? 640 : 1040
        panel.setContentSize(NSSize(width: size.width, height: panel.contentRect(forFrameRect: panel.frame).height))
        applyFilter()
        refreshPreview()

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

    /// What the smoke test reads back: the rows on show, whether the
    /// filter field is up, and the preview's text.
    var debugShownTexts: [String] { shown.map { Self.text(of: $0.row) } }
    var debugFiltering: Bool { filtering && !queryField.isHidden }
    var debugPreview: String { previewText.string }

    /// Debug hook: a key pressed in the panel, sent the way the window
    /// server sends one, so it takes the path a real key takes — to the
    /// field editor when the caret is in the filter field.
    func debugPress(keyCode: UInt16, characters: String) {
        guard let panel,
            let event = NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
                windowNumber: panel.windowNumber, context: nil, characters: characters,
                charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode)
        else { return }
        panel.sendEvent(event)
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
            case .item(let text, _):
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
        case .heading(let text), .item(let text, _): return text
        }
    }

    private static func styled(of row: Row) -> NSAttributedString? {
        if case .item(_, let styled) = row { return styled }
        return nil
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

        table.addTableColumn(NSTableColumn(identifier: .init("row")))
        table.onReturn = { [weak self] in self?.chooseSelection() }
        table.onType = { [weak self] typed in self?.startFiltering(with: typed) }
        previewText.isEditable = false
        previewText.isSelectable = true
        previewText.drawsBackground = false
        previewText.textContainerInset = NSSize(width: 8, height: 8)
        previewText.textContainer?.widthTracksTextView = false
        previewText.isHorizontallyResizable = true
        previewText.textContainer?.containerSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        previewScroll.documentView = previewText
        previewScroll.hasVerticalScroller = true
        previewScroll.hasHorizontalScroller = true
        previewScroll.borderType = .noBorder
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

    /// A letter typed at a list that has no field: the field comes in
    /// with that letter, and the list filters from there (3.3).
    private func startFiltering(with typed: String) {
        guard let panel else { return }
        if !filtering {
            filtering = true
            queryField.isHidden = false
            queryField.placeholderString = ""
            queryField.stringValue = ""
            layOut(in: panel)
        }
        panel.makeFirstResponder(queryField)
        queryField.stringValue += typed
        let length = (queryField.stringValue as NSString).length
        queryField.currentEditor()?.selectedRange = NSRange(location: length, length: 0)
        applyFilter()
    }

    /// The selected item's place, around its line, with the line
    /// marked: read from the open document when there is one, which
    /// may differ from the disk, else from the file.
    private func refreshPreview() {
        guard let placeOf else { return }
        let row = table.selectedRow
        guard shown.indices.contains(row), let item = shown[row].item, let place = placeOf(item)
        else {
            previewText.string = ""
            return
        }
        let text =
            (NSApp.delegate as? AppDelegate)?.openText(ofPath: place.path)
            ?? (try? String(contentsOfFile: place.path, encoding: .utf8)) ?? ""
        let (first, lines) = CoreReferences.preview(of: text, line: place.line)
        let font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        let numberWidth = String(first + lines.count).count
        let shownText = NSMutableAttributedString()
        for (offset, line) in lines.enumerated() {
            let number = String(first + offset + 1)
            let gutter = String(repeating: " ", count: numberWidth - number.count) + number + "  "
            let piece = NSMutableAttributedString(
                string: gutter, attributes: [.font: font, .foregroundColor: NSColor.tertiaryLabelColor])
            piece.append(
                NSAttributedString(
                    string: line + "\n", attributes: [.font: font, .foregroundColor: NSColor.labelColor]))
            if first + offset == place.line {
                piece.addAttribute(
                    .backgroundColor, value: NSColor.selectedTextBackgroundColor.withAlphaComponent(0.6),
                    range: NSRange(location: 0, length: piece.length))
            }
            shownText.append(piece)
        }
        previewText.textStorage?.setAttributedString(shownText)
        previewText.scroll(.zero)
    }

    /// The field is there or it is not, the list fills what is left,
    /// and for a list of places the preview takes the right half.
    private func layOut(in panel: NSPanel) {
        let content = NSView()
        queryField.translatesAutoresizingMaskIntoConstraints = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        previewScroll.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(scroll)
        var constraints: [NSLayoutConstraint] = [
            scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            scroll.bottomAnchor.constraint(equalTo: content.bottomAnchor),
        ]
        if placeOf != nil {
            content.addSubview(previewScroll)
            constraints += [
                scroll.widthAnchor.constraint(equalTo: content.widthAnchor, multiplier: 0.45),
                previewScroll.leadingAnchor.constraint(equalTo: scroll.trailingAnchor, constant: 1),
                previewScroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
                previewScroll.topAnchor.constraint(equalTo: scroll.topAnchor),
                previewScroll.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            ]
        } else {
            constraints.append(scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor))
        }
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
        if let styled = Self.styled(of: shown[row].row) {
            cell.attributedStringValue = styled
        } else {
            cell.stringValue = Self.text(of: shown[row].row)
        }
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

    func tableViewSelectionDidChange(_ notification: Notification) {
        refreshPreview()
    }

    /// The arrows and ⏎ typed in the field go to the list, so the list
    /// can be walked without leaving what is being typed.
    ///
    /// They are heard here and not in the field's `keyDown`: a field
    /// with the caret in it is not the first responder, its editor is,
    /// and a key never reaches the field itself.
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
            chooseSelection()
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            panel?.orderOut(nil)
            return true
        default:
            return false
        }
    }
}
