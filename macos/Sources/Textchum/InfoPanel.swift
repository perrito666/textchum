import AppKit
import TextchumKit

/// A panel docked under the editor, the way a second view stacks in the
/// column, with two faces: the documentation the hover bubble would
/// have shown — for the symbol under the pointer, and at the caret as
/// it moves — and the document's diagnostics as a list. While it is
/// shown, the bubble stays away and what it would have said lands here.
@MainActor
final class InfoPanel: NSView {
    enum Mode: Int {
        case documentation = 0
        case diagnostics = 1
    }

    /// One row of the diagnostics face and what choosing it does.
    struct Row {
        let text: NSAttributedString
        let choose: () -> Void
    }

    var mode: Mode = .documentation {
        didSet {
            guard mode != oldValue else { return }
            picker.selectedSegment = mode.rawValue
            showFace()
            onModeChange?(mode)
        }
    }
    var onModeChange: ((Mode) -> Void)?

    /// What the documentation face shows, for the smoke test to read.
    private(set) var documentationText = NSAttributedString()
    private(set) var rows: [Row] = []

    private let picker: NSSegmentedControl
    private let documentation = NSTextView()
    private let documentationScroll = NSScrollView()
    private let table = NSTableView()
    private let tableScroll = NSScrollView()

    override init(frame: NSRect) {
        picker = NSSegmentedControl(
            labels: [t("Documentation"), t("Diagnostics")], trackingMode: .selectOne,
            target: nil, action: nil)
        super.init(frame: frame)
        picker.target = self
        picker.action = #selector(pickFace(_:))
        picker.selectedSegment = 0
        picker.controlSize = .small
        picker.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        picker.translatesAutoresizingMaskIntoConstraints = false
        addSubview(picker)

        documentation.isEditable = false
        documentation.isSelectable = true
        documentation.drawsBackground = false
        documentation.textContainerInset = NSSize(width: 8, height: 6)
        documentation.isVerticallyResizable = true
        documentation.isHorizontallyResizable = false
        documentation.autoresizingMask = [.width]
        documentation.textContainer?.widthTracksTextView = true
        documentation.minSize = .zero
        documentation.maxSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        documentationScroll.documentView = documentation
        documentationScroll.hasVerticalScroller = true
        documentationScroll.drawsBackground = false
        documentationScroll.translatesAutoresizingMaskIntoConstraints = false
        addSubview(documentationScroll)

        table.addTableColumn(NSTableColumn(identifier: .init("row")))
        table.headerView = nil
        table.rowHeight = 20
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.action = #selector(chooseRow(_:))
        table.doubleAction = #selector(chooseRow(_:))
        tableScroll.documentView = table
        tableScroll.hasVerticalScroller = true
        tableScroll.drawsBackground = false
        tableScroll.translatesAutoresizingMaskIntoConstraints = false
        addSubview(tableScroll)

        for scroll in [documentationScroll, tableScroll] {
            NSLayoutConstraint.activate([
                scroll.topAnchor.constraint(equalTo: picker.bottomAnchor, constant: 4),
                scroll.leadingAnchor.constraint(equalTo: leadingAnchor),
                scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
                scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
            ])
        }
        NSLayoutConstraint.activate([
            picker.topAnchor.constraint(equalTo: topAnchor, constant: 4),
            picker.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 80),
        ])
        showFace()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("InfoPanel is built in code") }

    override var isFlipped: Bool { true }

    /// The documentation face shows `attributed`; the switch stays where
    /// it is, so documentation arriving while the list is up waits.
    func showDocumentation(_ attributed: NSAttributedString) {
        documentationText = attributed
        documentation.textStorage?.setAttributedString(attributed)
    }

    /// Grey words for when there is nothing to say yet.
    func showPlaceholder(_ text: String) {
        showDocumentation(
            NSAttributedString(
                string: text,
                attributes: [
                    .font: NSFont.systemFont(ofSize: 12),
                    .foregroundColor: NSColor.secondaryLabelColor,
                ]))
    }

    func showDiagnostics(_ rows: [Row]) {
        self.rows = rows
        table.reloadData()
    }

    private func showFace() {
        documentationScroll.isHidden = mode != .documentation
        tableScroll.isHidden = mode != .diagnostics
    }

    @objc private func pickFace(_ sender: Any?) {
        mode = Mode(rawValue: picker.selectedSegment) ?? .documentation
    }

    @objc private func chooseRow(_ sender: Any?) {
        let row = table.clickedRow >= 0 ? table.clickedRow : table.selectedRow
        guard rows.indices.contains(row) else { return }
        rows[row].choose()
    }
}

extension InfoPanel: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    func tableView(
        _ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int
    ) -> NSView? {
        let identifier = NSUserInterfaceItemIdentifier("diagnostic")
        let cell =
            tableView.makeView(withIdentifier: identifier, owner: nil) as? NSTextField
            ?? {
                let field = NSTextField(labelWithString: "")
                field.identifier = identifier
                field.lineBreakMode = .byTruncatingTail
                return field
            }()
        cell.attributedStringValue = rows[row].text
        return cell
    }
}
