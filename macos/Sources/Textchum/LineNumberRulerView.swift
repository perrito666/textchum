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
    /// The worst diagnostic on each line, by line number from one:
    /// 1 error, 2 warning, 3 information, 4 hint.
    private var diagnosticSeverities: [Int: Int] = [:]
    /// The line where the server offers something to do at the caret: a
    /// fix, a refactoring. A bulb is drawn there in place of the
    /// finding's dot, and a click on it lists what is offered (12.1).
    private(set) var fixLine: Int?
    private var fixMarkRect: NSRect?
    /// A click on the bulb.
    var onFixMark: (() -> Void)?
    /// The lines, from one, where something can be run — a `main`, a
    /// test — marked with a triangle that runs it; and where each was
    /// drawn, for clicks.
    private(set) var runLines: Set<Int> = []
    private var runMarkRects: [Int: NSRect] = [:]
    /// A click on a run mark, with its line from one.
    var onRunMark: ((Int) -> Void)?
    /// The lines, from one, with a breakpoint: a red disc, drawn in place
    /// of any other mark on the line.
    private(set) var breakpointLines: Set<Int> = []
    /// A click in the marks column where there is no other mark to click:
    /// a breakpoint put or taken away on that line, from one.
    var onBreakpoint: ((Int) -> Void)?
    /// The line, from one, the debugged program is stopped at.
    private(set) var stoppedLine: Int?

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

    /// The lines with diagnostics and how bad: their numbers take the
    /// severity's colour and a mark beside them says the level.
    func setDiagnostics(_ severities: [Int: Int]) {
        guard severities != diagnosticSeverities else { return }
        diagnosticSeverities = severities
        needsDisplay = true
    }

    func diagnosticSeverity(ofLine line: Int) -> Int? { diagnosticSeverities[line] }

    func setRunLines(_ lines: Set<Int>) {
        guard lines != runLines else { return }
        runLines = lines
        runMarkRects = [:]
        needsDisplay = true
    }

    func setBreakpointLines(_ lines: Set<Int>) {
        guard lines != breakpointLines else { return }
        breakpointLines = lines
        needsDisplay = true
    }

    func setStoppedLine(_ line: Int?) {
        guard line != stoppedLine else { return }
        stoppedLine = line
        needsDisplay = true
    }

    /// The line, from one, at a point of the gutter.
    private func line(at point: NSPoint) -> Int? {
        guard let textView else { return nil }
        let inText = textView.convert(NSPoint(x: textView.textContainerOrigin.x, y: point.y), from: self)
        return lineNumber(forOffset: textView.characterIndexForInsertion(at: inText))
    }

    func setFixLine(_ line: Int?) {
        guard line != fixLine else { return }
        fixLine = line
        if line == nil { fixMarkRect = nil }
        needsDisplay = true
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if let rect = fixMarkRect, rect.insetBy(dx: -3, dy: -3).contains(point), let onFixMark {
            onFixMark()
            return
        }
        if let (line, _) = runMarkRects.first(where: { $0.value.insetBy(dx: -3, dy: -3).contains(point) }),
            let onRunMark
        {
            onRunMark(line)
            return
        }
        if point.x < 16, let onBreakpoint, let line = line(at: point) {
            onBreakpoint(line)
            return
        }
        super.mouseDown(with: event)
    }

    private static func diagnosticColor(_ severity: Int) -> NSColor {
        switch severity {
        case 1: .systemRed
        case 2: .systemOrange
        default: .systemBlue
        }
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

    /// Patches the cache around one edit instead of rebuilding it: the
    /// starts after the edit move by a constant, and only the edited
    /// stretch's own line breaks change. Typing pays for the keystroke,
    /// not for the file. Anything that is not a single replacement —
    /// multi-range edits, reloads — still takes the full rebuild.
    func noteEdit(at range: NSRange, replacementLength: Int) {
        guard let textView else { return }
        let text: NSString =
            textView.textStorage?.mutableString ?? (textView.string as NSString)
        let delta = replacementLength - range.length
        let oldEnd = NSMaxRange(range)
        // Starts at or before the edit's first character are untouched:
        // the newline that made each of them sits before the edit.
        var patched: [Int] = []
        patched.reserveCapacity(lineStarts.count + 4)
        for start in lineStarts {
            if start <= range.location {
                patched.append(start)
            } else if start > oldEnd {
                patched.append(start + delta)
            }
            // Starts inside the replaced stretch are gone with it.
        }
        // The replacement's own line breaks, plus the boundary right
        // after it — an edit ending in a newline makes one.
        let newEnd = range.location + replacementLength
        var index = range.location
        while index < newEnd {
            var found = text.range(
                of: "\n", options: [], range: NSRange(location: index, length: newEnd - index))
            if found.location == NSNotFound { break }
            found.location += 1
            if found.location < text.length || text.hasSuffix("\n") {
                patched.append(found.location)
            }
            index = found.location
        }
        patched.sort()
        // The edit can also have created or removed the boundary at its
        // trailing edge (deleting up to a newline, say); one cheap local
        // check settles it.
        if newEnd > 0, newEnd <= text.length,
            text.character(at: newEnd - 1) == 10,
            newEnd < text.length || text.hasSuffix("\n"),
            !patched.contains(newEnd)
        {
            patched.append(newEnd)
            patched.sort()
        }
        lineStarts = patched.isEmpty ? [0] : patched
        #if DEBUG
            let before = lineStarts
            invalidateLineStarts()
            assert(
                before == lineStarts,
                "patched line starts diverge from the rebuild")
        #else
            let digits = max(2, String(lineStarts.count).count)
            visibleWidth = CGFloat(digits) * 8 + 16
            if shown {
                widthConstraint?.constant = visibleWidth
            }
            needsDisplay = true
        #endif
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

        // Where the marks are is where this pass draws them; a mark not
        // drawn this time is not there to click.
        fixMarkRect = nil
        runMarkRects = [:]
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
                var labelAttributes = attributes
                let numberHeight = NSAttributedString(string: "0", attributes: attributes).size().height
                if line == self.stoppedLine {
                    NSColor.systemYellow.withAlphaComponent(0.35).setFill()
                    NSRect(x: 0, y: top, width: self.bounds.width - 1, height: frame.height).fill()
                }
                if self.breakpointLines.contains(line) {
                    let size: CGFloat = 10
                    let disc = NSRect(x: 3, y: top + 1 + (numberHeight - size) / 2, width: size, height: size)
                    NSColor.systemRed.setFill()
                    NSBezierPath(ovalIn: disc).fill()
                    if let severity = self.diagnosticSeverities[line] {
                        labelAttributes[.foregroundColor] = Self.diagnosticColor(severity)
                    }
                } else if line == self.fixLine,
                    let bulb = NSImage(systemSymbolName: "lightbulb.fill", accessibilityDescription: t("Code Actions"))
                {
                    let size: CGFloat = 11
                    let rect = NSRect(x: 3, y: top + 1 + (numberHeight - size) / 2, width: size, height: size)
                    let tinted = bulb.withSymbolConfiguration(
                        .init(paletteColors: [.systemYellow])) ?? bulb
                    tinted.draw(in: rect)
                    self.fixMarkRect = rect
                    if let severity = self.diagnosticSeverities[line] {
                        labelAttributes[.foregroundColor] = Self.diagnosticColor(severity)
                    }
                } else if self.runLines.contains(line),
                    let play = NSImage(systemSymbolName: "play.fill", accessibilityDescription: t("Run"))
                {
                    // The bulb wins where both are; a finding still
                    // colours the number.
                    let size: CGFloat = 9
                    let rect = NSRect(x: 4, y: top + 1 + (numberHeight - size) / 2, width: size, height: size)
                    let tinted = play.withSymbolConfiguration(.init(paletteColors: [.systemGreen])) ?? play
                    tinted.draw(in: rect)
                    self.runMarkRects[line] = rect
                    if let severity = self.diagnosticSeverities[line] {
                        labelAttributes[.foregroundColor] = Self.diagnosticColor(severity)
                    }
                } else if let severity = self.diagnosticSeverities[line] {
                    // The number in the severity's colour, and a mark
                    // for the level: a disc for an error or warning, a
                    // ring for information and hints. Centred on the
                    // number, not the fragment: the file's last
                    // fragment spans the empty line after it.
                    let color = Self.diagnosticColor(severity)
                    labelAttributes[.foregroundColor] = color
                    let size: CGFloat = 6
                    let dot = NSRect(
                        x: 6, y: top + 1 + (numberHeight - size) / 2, width: size, height: size)
                    let path = NSBezierPath(ovalIn: dot)
                    if severity <= 2 {
                        color.setFill()
                        path.fill()
                    } else {
                        color.setStroke()
                        path.lineWidth = 1.2
                        path.stroke()
                    }
                }
                let label = NSAttributedString(string: String(line), attributes: labelAttributes)
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
