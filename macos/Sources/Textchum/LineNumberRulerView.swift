import AppKit

/// A line-number gutter for a TextKit 2 text view.
///
/// Deliberately *not* an `NSRulerView`: installing a ruler mutates the
/// scroll geometry under TextKit 2's viewport and blanks the text. The
/// gutter is a plain sibling view left of the scroll view, redrawn when
/// the editor scrolls or its text changes. Line starts are cached so
/// drawing is a binary search per visible layout fragment; the gutter
/// widens itself as the document grows digits.
final class LineNumberGutterView: NSView {
    private weak var textView: NSTextView?
    /// UTF-16 offsets where each line starts; always begins with 0.
    private var lineStarts: [Int] = [0]
    private var widthConstraint: NSLayoutConstraint?
    private var visibleWidth: CGFloat = 40
    private var shown = true

    override var isFlipped: Bool { true }

    init(textView: NSTextView) {
        self.textView = textView
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let constraint = widthAnchor.constraint(equalToConstant: visibleWidth)
        constraint.isActive = true
        widthConstraint = constraint
        invalidateLineStarts()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("LineNumberGutterView is created in code")
    }

    func setVisible(_ visible: Bool) {
        shown = visible
        widthConstraint?.constant = visible ? visibleWidth : 0
        needsDisplay = true
    }

    /// Recomputes the line-start cache; call on every text change.
    func invalidateLineStarts() {
        guard let textView else { return }
        let text = textView.string as NSString
        var starts: [Int] = [0]
        var index = 0
        while index < text.length {
            let lineRange = text.lineRange(for: NSRange(location: index, length: 0))
            index = NSMaxRange(lineRange)
            if index < text.length || text.hasSuffix("\n") {
                starts.append(index)
            }
        }
        lineStarts = starts
        let digits = max(2, String(starts.count).count)
        visibleWidth = CGFloat(digits) * 8 + 16
        if shown {
            widthConstraint?.constant = visibleWidth
        }
        needsDisplay = true
    }

    private func lineNumber(forOffset offset: Int) -> Int {
        var low = 0
        var high = lineStarts.count - 1
        while low < high {
            let mid = (low + high + 1) / 2
            if lineStarts[mid] <= offset { low = mid } else { high = mid - 1 }
        }
        return low + 1
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.textBackgroundColor.setFill()
        bounds.fill()
        guard shown, let textView,
            let layoutManager = textView.textLayoutManager,
            let contentManager = layoutManager.textContentManager
        else { return }

        // Hairline separating gutter from text.
        NSColor.separatorColor.withAlphaComponent(0.5).setFill()
        NSRect(x: bounds.maxX - 1, y: 0, width: 1, height: bounds.height).fill()

        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(
                ofSize: (textView.font?.pointSize ?? 13) * 0.8, weight: .regular),
            .foregroundColor: NSColor.tertiaryLabelColor,
        ]

        // Only fragments the viewport already laid out; never force
        // layout from inside a draw pass.
        var lastLine = -1
        let origin = textView.textContainerOrigin
        let documentStart = layoutManager.documentRange.location
        let viewport = layoutManager.textViewportLayoutController.viewportRange
        layoutManager.enumerateTextLayoutFragments(
            from: viewport?.location ?? documentStart,
            options: []
        ) { fragment in
            let frame = fragment.layoutFragmentFrame
            let top = self.convert(
                NSPoint(x: 0, y: frame.minY + origin.y), from: textView
            ).y
            if top > self.bounds.maxY {
                return false
            }
            let offset = contentManager.offset(
                from: documentStart, to: fragment.rangeInElement.location)
            let line = self.lineNumber(forOffset: offset)
            if line != lastLine, top >= -frame.height {
                lastLine = line
                let label = NSAttributedString(string: String(line), attributes: attributes)
                let x = self.bounds.maxX - label.size().width - 8
                label.draw(at: NSPoint(x: x, y: top + 1))
            }
            return true
        }
    }
}
