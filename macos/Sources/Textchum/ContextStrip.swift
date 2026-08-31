import AppKit

/// The pinned context: the first line of each construct enclosing what
/// the view shows, stacked at the top of the editor — the `class` line
/// and the `def` line while a long method scrolls. A breadcrumb made of
/// the file's own lines.
///
/// An overlay over the scroll view rather than a sibling above it, so
/// switching it off costs no layout and the text keeps its geometry.
/// Clicking a row scrolls its line to the top.
final class ContextStrip: NSView {
    /// The lines shown, zero-based, outermost first.
    private(set) var lines: [Int] = []
    private var rows: [NSTextField] = []
    private lazy var heightConstraint: NSLayoutConstraint = {
        let constraint = heightAnchor.constraint(equalToConstant: 0)
        constraint.isActive = true
        return constraint
    }()
    var onSelect: ((Int) -> Void)?

    override var isFlipped: Bool { true }

    /// At most this many rows; the innermost win beyond it.
    static let maxRows = 5

    /// Forces the next `show` to rebuild: the lines kept their numbers
    /// but an edit or a recolour changed what they say.
    func invalidateText() {
        rows.forEach { $0.removeFromSuperview() }
        rows = []
    }

    /// Replaces the rows. `text(line)` renders one line the way the
    /// editor shows it, colours included.
    func show(lines: [Int], text: (Int) -> NSAttributedString, rowHeight: CGFloat) {
        guard lines != self.lines || rows.isEmpty else { return }
        self.lines = lines
        rows.forEach { $0.removeFromSuperview() }
        rows = []
        isHidden = lines.isEmpty
        var y: CGFloat = 0
        for line in lines {
            let row = NSTextField(labelWithAttributedString: text(line))
            row.lineBreakMode = .byTruncatingTail
            row.maximumNumberOfLines = 1
            row.translatesAutoresizingMaskIntoConstraints = false
            row.drawsBackground = true
            row.backgroundColor = .textBackgroundColor
            addSubview(row)
            NSLayoutConstraint.activate([
                row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
                row.trailingAnchor.constraint(equalTo: trailingAnchor),
                row.topAnchor.constraint(equalTo: topAnchor, constant: y),
                row.heightAnchor.constraint(equalToConstant: rowHeight),
            ])
            rows.append(row)
            y += rowHeight
        }
        heightConstraint.constant = y + (lines.isEmpty ? 0 : 1)
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        guard !lines.isEmpty else { return }
        NSColor.textBackgroundColor.setFill()
        bounds.fill()
        // A hairline under the pins, so they read as a shelf and not as
        // the first lines of the view.
        NSColor.separatorColor.withAlphaComponent(0.6).setFill()
        NSRect(x: 0, y: bounds.maxY - 1, width: bounds.width, height: 1).fill()
    }

    override func mouseDown(with event: NSEvent) {
        let y = convert(event.locationInWindow, from: nil).y
        guard !rows.isEmpty else { return }
        let height = rows[0].frame.height
        let index = Int(y / max(height, 1))
        if lines.indices.contains(index) {
            onSelect?(lines[index])
        }
    }
}
