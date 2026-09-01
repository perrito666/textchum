import AppKit
import TextchumKit

/// The editor's text view: NSTextView plus a faint tint on the caret's
/// line, so the caret can be found in a long file at a glance.
final class EditorTextView: NSTextView {
    /// The line the tint sat on when last drawn, so a caret move
    /// invalidates only what changed.
    private var tintedLine: NSRect = .zero

    override func drawBackground(in rect: NSRect) {
        super.drawBackground(in: rect)
        guard let line = caretLineRect(), line.intersects(rect) else { return }
        NSColor.textColor.withAlphaComponent(0.045).setFill()
        line.fill()
        tintedLine = line
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

    override func moveWordForwardAndModifySelection(_ sender: Any?) {
        extendSelection(to: wordBoundaryFromCaret(forward: true))
    }
    override func moveWordBackwardAndModifySelection(_ sender: Any?) {
        extendSelection(to: wordBoundaryFromCaret(forward: false))
    }
    override func moveWordLeftAndModifySelection(_ sender: Any?) {
        extendSelection(to: wordBoundaryFromCaret(forward: false))
    }
    override func moveWordRightAndModifySelection(_ sender: Any?) {
        extendSelection(to: wordBoundaryFromCaret(forward: true))
    }

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

    private func wordBoundaryFromCaret(forward: Bool) -> Int {
        let selection = selectedRange()
        let anchor = selectionAnchor ?? selection.location
        let caret = anchor == selection.location ? NSMaxRange(selection) : selection.location
        return CoreMotion.wordBoundary(in: string, from: caret, forward: forward)
    }

    private func extendSelection(to caret: Int) {
        let selection = selectedRange()
        if selection.length == 0 { selectionAnchor = selection.location }
        let anchor = selectionAnchor ?? selection.location
        let range = NSRange(location: min(anchor, caret), length: abs(caret - anchor))
        setSelectedRange(
            range, affinity: caret < anchor ? .upstream : .downstream, stillSelecting: false)
        // The anchor survives the collapse check below: this set is ours.
        selectionAnchor = anchor
        scrollRangeToVisible(NSRange(location: caret, length: 0))
    }

    override func setSelectedRanges(
        _ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool
    ) {
        super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
        // A collapsed selection ends any word-wise extension.
        if let first = ranges.first?.rangeValue, first.length == 0 { selectionAnchor = nil }
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
