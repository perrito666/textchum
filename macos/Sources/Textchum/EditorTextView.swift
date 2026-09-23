import AppKit
import TextchumKit

/// The editor's text view: NSTextView plus a faint tint on the caret's
/// line, so the caret can be found in a long file at a glance.
final class EditorTextView: NSTextView {
    /// The line the tint sat on when last drawn, so a caret move
    /// invalidates only what changed.
    private var tintedLine: NSRect = .zero

    /// Backgrounds behind ranges of text — the selected word's other
    /// occurrences, find matches, spelling, diagnostics — drawn by the
    /// view from the layout's segment rectangles. The text system's
    /// own rendering-attribute backgrounds were not reliable for a
    /// range inside a line.
    struct BackgroundMark {
        let range: NSRange
        let color: NSColor
    }

    var backgroundMarks: [BackgroundMark] = [] {
        didSet { needsDisplay = true }
    }

    override func drawBackground(in rect: NSRect) {
        super.drawBackground(in: rect)
        if let line = caretLineRect(), line.intersects(rect) {
            NSColor.textColor.withAlphaComponent(0.045).setFill()
            line.fill()
            tintedLine = line
        }
        drawBackgroundMarks(in: rect)
    }

    private func drawBackgroundMarks(in rect: NSRect) {
        guard !backgroundMarks.isEmpty, let layoutManager = textLayoutManager,
            let contentManager = layoutManager.textContentManager
        else { return }
        let origin = textContainerOrigin
        let length = (string as NSString).length
        for mark in backgroundMarks {
            guard mark.range.length > 0, NSMaxRange(mark.range) <= length,
                let start = contentManager.location(
                    layoutManager.documentRange.location, offsetBy: mark.range.location),
                let end = contentManager.location(start, offsetBy: mark.range.length),
                let textRange = NSTextRange(location: start, end: end)
            else { continue }
            mark.color.setFill()
            layoutManager.enumerateTextSegments(in: textRange, type: .highlight, options: [.rangeNotRequired]) {
                _, frame, _, _ in
                let box = frame.offsetBy(dx: origin.x, dy: origin.y)
                if box.intersects(rect) { box.fill() }
                return true
            }
        }
    }

    // MARK: Word movement by code's boundaries

    /// ⌥→ / ⌥← and their selecting and deleting forms stop at every
    /// change of character class — identifier, symbol, blank — and at
    /// a line break, the way code editors do; the text system's words
    /// lump a run of symbols together and read across lines.
    private func wordTarget(forward: Bool) -> Int {
        let selection = selectedRange()
        let from = forward ? NSMaxRange(selection) : selection.location
        return CoreMotion.wordBoundary(in: string, from: from, forward: forward)
    }

    override func moveWordForward(_ sender: Any?) { moveWord(forward: true) }
    override func moveWordBackward(_ sender: Any?) { moveWord(forward: false) }

    // Option+Arrow binds to the direction-aware selectors, not the
    // backward/forward ones; the editor is left-to-right code, so left
    // is backward. Without these, ⌥← used the text system's own words.
    override func moveWordLeft(_ sender: Any?) { moveWord(forward: false) }
    override func moveWordRight(_ sender: Any?) { moveWord(forward: true) }

    override func moveWordForwardAndModifySelection(_ sender: Any?) { extendByWord(forward: true) }
    override func moveWordBackwardAndModifySelection(_ sender: Any?) { extendByWord(forward: false) }
    override func moveWordLeftAndModifySelection(_ sender: Any?) { extendByWord(forward: false) }
    override func moveWordRightAndModifySelection(_ sender: Any?) { extendByWord(forward: true) }

    private func moveWord(forward: Bool) {
        setSelectedRange(NSRange(location: wordTarget(forward: forward), length: 0))
        scrollRangeToVisible(selectedRange())
    }

    override func deleteWordForward(_ sender: Any?) {
        let selection = selectedRange()
        guard selection.length == 0 else { return super.deleteWordForward(sender) }
        let end = CoreMotion.wordBoundary(in: string, from: selection.location, forward: true)
        let range = NSRange(location: selection.location, length: end - selection.location)
        if shouldChangeText(in: range, replacementString: "") {
            textStorage?.replaceCharacters(in: range, with: "")
            didChangeText()
        }
    }

    override func deleteWordBackward(_ sender: Any?) {
        let selection = selectedRange()
        guard selection.length == 0 else { return super.deleteWordBackward(sender) }
        let start = CoreMotion.wordBoundary(in: string, from: selection.location, forward: false)
        let range = NSRange(location: start, length: selection.location - start)
        if shouldChangeText(in: range, replacementString: "") {
            textStorage?.replaceCharacters(in: range, with: "")
            didChangeText()
        }
    }

    /// The selection's fixed end while ⌥⇧ arrows extend it; AppKit
    /// keeps its own anchor private.
    private var selectionAnchor: Int?

    /// Moves the selection's free end a word the way the arrow points.
    ///
    /// A selection this view did not make — a double-clicked word, a
    /// drag, a shift-click — has no anchor on record. It used to be
    /// taken as anchored at its start, so ⌥⇧→ grew it and ⌥⇧← ate it
    /// from the end until nothing was left. The end that moves is the
    /// one the arrow points at; the other becomes the anchor and stays
    /// it, so a word taken back can be given back.
    private func extendByWord(forward: Bool) {
        let selection = selectedRange()
        if selectionAnchor == nil {
            selectionAnchor = forward ? selection.location : NSMaxRange(selection)
        }
        let anchor = selectionAnchor ?? selection.location
        let caret = anchor == selection.location ? NSMaxRange(selection) : selection.location
        extendSelection(
            to: CoreMotion.wordBoundary(in: string, from: caret, forward: forward), anchor: anchor)
    }

    /// True while this view sets a selection of its own making, which
    /// is the one kind that keeps the anchor.
    private var extendingSelection = false

    private func extendSelection(to caret: Int, anchor: Int) {
        let range = NSRange(location: min(anchor, caret), length: abs(caret - anchor))
        extendingSelection = true
        setSelectedRange(
            range, affinity: caret < anchor ? .upstream : .downstream, stillSelecting: false)
        extendingSelection = false
        selectionAnchor = anchor
        scrollRangeToVisible(NSRange(location: caret, length: 0))
    }

    override func setSelectedRanges(
        _ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool
    ) {
        super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
        // Any selection but our own — a click, a drag, a find, a jump —
        // is a new selection with no anchor yet; the next ⌥⇧ arrow
        // decides which end holds still.
        if !extendingSelection { selectionAnchor = nil }
        if !tintedLine.isEmpty { setNeedsDisplay(tintedLine) }
        if let line = caretLineRect() { setNeedsDisplay(line) }
    }

    /// The full-width rectangle of the line holding the caret, in view
    /// coordinates. Only what is laid out is asked; the tint is not
    /// worth forcing layout for.
    private func caretLineRect() -> NSRect? {
        let caret = selectedRange().location
        if let layoutManager = textLayoutManager,
            let contentManager = layoutManager.textContentManager
        {
            guard
                let location = contentManager.location(
                    contentManager.documentRange.location, offsetBy: caret)
            else { return nil }
            var frame: NSRect?
            layoutManager.enumerateTextLayoutFragments(from: location, options: []) {
                fragment in
                frame = fragment.layoutFragmentFrame
                return false
            }
            guard var line = frame else { return nil }
            line.origin.y += textContainerOrigin.y
            line.origin.x = 0
            line.size.width = bounds.width
            return line
        }
        guard let layoutManager, let container = textContainer else { return nil }
        let glyph = layoutManager.glyphIndexForCharacter(at: min(caret, max(0, string.utf16.count - 1)))
        var line = layoutManager.lineFragmentRect(forGlyphAt: glyph, effectiveRange: nil)
        _ = container
        line.origin.y += textContainerOrigin.y
        line.origin.x = 0
        line.size.width = bounds.width
        return line
    }
}
