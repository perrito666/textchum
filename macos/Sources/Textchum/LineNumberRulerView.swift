import AppKit
import TextchumKit

/// A line-number gutter for a TextKit 2 text view, with a change bar
/// down its left edge saying which lines differ from the file as it
/// stands in git.
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
    /// Change marks by one-based line, and the lines a deletion sits
    /// above. Empty when the file has no committed version to compare
    /// against, which is not an error.
    private var changeKinds: [Int: CoreChanges.Kind] = [:]

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

    /// The git change marks to draw. Passing an empty array clears
    /// them, which is what a file outside a repository gets.
    func setChangeMarks(_ marks: [CoreChanges.Mark]) {
        var kinds: [Int: CoreChanges.Kind] = [:]
        for mark in marks {
            // The core counts from zero; the gutter counts from one.
            kinds[mark.line + 1] = mark.kind
        }
        guard kinds != changeKinds else { return }
        changeKinds = kinds
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

    /// The zero-based line holding the UTF-16 `offset`. The status bar
    /// and the pinned context ask; the cache is already here.
    func lineIndex(forOffset offset: Int) -> Int {
        lineNumber(forOffset: offset) - 1
    }

    /// Where the zero-based `line` starts, in UTF-16, clamped to the
    /// last line.
    func lineStart(ofLine line: Int) -> Int {
        lineStarts[max(0, min(line, lineStarts.count - 1))]
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
            // A folded line is laid out at a hundredth of a point, so
            // its number would land on the one above it.
            if frame.height < 1 {
                return true
            }
            if line != lastLine, top >= -frame.height {
                lastLine = line
                self.drawChangeMark(forLine: line, top: top, height: frame.height)
                let label = NSAttributedString(string: String(line), attributes: attributes)
                let x = self.bounds.maxX - label.size().width - 8
                label.draw(at: NSPoint(x: x, y: top + 1))
            }
            return true
        }
    }

    /// The change bar for one line: a stripe down the gutter's left
    /// edge for a line that is new or edited, and a wedge on the
    /// boundary where lines were deleted — deleted lines occupy no
    /// height, so a stripe would have nothing to cover.
    private func drawChangeMark(forLine line: Int, top: CGFloat, height: CGFloat) {
        guard let kind = changeKinds[line] else { return }
        let barWidth: CGFloat = 3
        switch kind {
        case .added, .modified:
            (kind == .added ? Self.addedColor : Self.modifiedColor).setFill()
            NSRect(x: 0, y: top, width: barWidth, height: height).fill()
        case .removed:
            Self.removedColor.setFill()
            let wedge = NSBezierPath()
            wedge.move(to: NSPoint(x: 0, y: top - 3))
            wedge.line(to: NSPoint(x: barWidth + 2, y: top))
            wedge.line(to: NSPoint(x: 0, y: top + 3))
            wedge.close()
            wedge.fill()
        }
    }

    // Deliberately not the theme's colours: these say what git thinks,
    // not what the language means, and a reader should not have to
    // wonder whether a green bar is a string.
    private static let addedColor = NSColor.systemGreen.withAlphaComponent(0.85)
    private static let modifiedColor = NSColor.systemBlue.withAlphaComponent(0.85)
    private static let removedColor = NSColor.systemRed.withAlphaComponent(0.85)
}
