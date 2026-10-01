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

    /// The selection's fixed end, whoever made the selection.
    ///
    /// AppKit keeps an anchor of its own for ⇧ arrows, private, and
    /// forgets it whenever the selection is set from outside — which is
    /// how the word commands below set theirs. Two anchors that do not
    /// know about each other meant a selection made one way and
    /// continued the other found no anchor on record, and extended
    /// whichever end the arrow pointed at: ⇧← then ⌥⇧→ grew the
    /// selection at both ends, and it could only grow. So this one is
    /// kept for every selection the view can see being made, by
    /// noticing which end stood still (see `setSelectedRanges`), and
    /// every extending command consults it.
    private var selectionAnchor: Int?

    /// True while a selection is being changed by something known to
    /// keep one end where it was — AppKit's own extending commands, a
    /// shift-click — so the end that stood still can be taken as the
    /// anchor. Any other change is a new selection.
    private var keepingAnEnd = false

    /// True when the last selection was set by this view rather than
    /// by AppKit's own extending commands, whose private anchor is
    /// therefore stale: the next ⇧ arrow has to be worked out here.
    private var appKitAnchorIsStale = false

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
        appKitAnchorIsStale = true
        scrollRangeToVisible(NSRange(location: caret, length: 0))
    }

    /// One of AppKit's own extending commands. While its anchor is in
    /// step with the one on record here, the command runs as AppKit
    /// wrote it, which keeps what it does well — the column a run of
    /// ⇧↓ remembers across a short line. After a selection this view
    /// set itself, AppKit has no anchor to extend from, so the free
    /// end is moved here instead: the selection collapses to it, the
    /// plain movement says where that lands, and the selection is set
    /// again from the anchor to there.
    private func extend(
        pointing forward: Bool, natively native: () -> Void, plainly plain: () -> Void
    ) {
        guard appKitAnchorIsStale else {
            // AppKit moves one end and keeps the other; which one it
            // kept is read off the result.
            keepingAnEnd = true
            native()
            keepingAnEnd = false
            return
        }
        let selection = selectedRange()
        let anchor = selectionAnchor ?? (forward ? selection.location : NSMaxRange(selection))
        let caret = anchor == selection.location ? NSMaxRange(selection) : selection.location
        extendingSelection = true
        // Still selecting: the collapsed caret is a step, not a
        // selection anyone should hear about.
        super.setSelectedRanges(
            [NSValue(range: NSRange(location: caret, length: 0))], affinity: .downstream,
            stillSelecting: true)
        plain()
        let landed = selectedRange().location
        extendingSelection = false
        extendSelection(to: landed, anchor: anchor)
    }

    override func moveLeftAndModifySelection(_ sender: Any?) {
        extend(
            pointing: false, natively: { super.moveLeftAndModifySelection(sender) },
            plainly: { super.moveLeft(sender) })
    }
    override func moveRightAndModifySelection(_ sender: Any?) {
        extend(
            pointing: true, natively: { super.moveRightAndModifySelection(sender) },
            plainly: { super.moveRight(sender) })
    }
    override func moveBackwardAndModifySelection(_ sender: Any?) {
        extend(
            pointing: false, natively: { super.moveBackwardAndModifySelection(sender) },
            plainly: { super.moveBackward(sender) })
    }
    override func moveForwardAndModifySelection(_ sender: Any?) {
        extend(
            pointing: true, natively: { super.moveForwardAndModifySelection(sender) },
            plainly: { super.moveForward(sender) })
    }
    override func moveUpAndModifySelection(_ sender: Any?) {
        extend(
            pointing: false, natively: { super.moveUpAndModifySelection(sender) },
            plainly: { super.moveUp(sender) })
    }
    override func moveDownAndModifySelection(_ sender: Any?) {
        extend(
            pointing: true, natively: { super.moveDownAndModifySelection(sender) },
            plainly: { super.moveDown(sender) })
    }
    override func moveToBeginningOfLineAndModifySelection(_ sender: Any?) {
        extend(
            pointing: false,
            natively: { super.moveToBeginningOfLineAndModifySelection(sender) },
            plainly: { super.moveToBeginningOfLine(sender) })
    }
    override func moveToEndOfLineAndModifySelection(_ sender: Any?) {
        extend(
            pointing: true, natively: { super.moveToEndOfLineAndModifySelection(sender) },
            plainly: { super.moveToEndOfLine(sender) })
    }
    override func moveToLeftEndOfLineAndModifySelection(_ sender: Any?) {
        extend(
            pointing: false,
            natively: { super.moveToLeftEndOfLineAndModifySelection(sender) },
            plainly: { super.moveToLeftEndOfLine(sender) })
    }
    override func moveToRightEndOfLineAndModifySelection(_ sender: Any?) {
        extend(
            pointing: true,
            natively: { super.moveToRightEndOfLineAndModifySelection(sender) },
            plainly: { super.moveToRightEndOfLine(sender) })
    }
    override func moveToBeginningOfParagraphAndModifySelection(_ sender: Any?) {
        extend(
            pointing: false,
            natively: { super.moveToBeginningOfParagraphAndModifySelection(sender) },
            plainly: { super.moveToBeginningOfParagraph(sender) })
    }
    override func moveToEndOfParagraphAndModifySelection(_ sender: Any?) {
        extend(
            pointing: true,
            natively: { super.moveToEndOfParagraphAndModifySelection(sender) },
            plainly: { super.moveToEndOfParagraph(sender) })
    }
    override func moveParagraphBackwardAndModifySelection(_ sender: Any?) {
        extend(
            pointing: false,
            natively: { super.moveParagraphBackwardAndModifySelection(sender) },
            plainly: { super.moveToBeginningOfParagraph(sender) })
    }
    override func moveParagraphForwardAndModifySelection(_ sender: Any?) {
        extend(
            pointing: true,
            natively: { super.moveParagraphForwardAndModifySelection(sender) },
            plainly: { super.moveToEndOfParagraph(sender) })
    }
    override func moveToBeginningOfDocumentAndModifySelection(_ sender: Any?) {
        extend(
            pointing: false,
            natively: { super.moveToBeginningOfDocumentAndModifySelection(sender) },
            plainly: { super.moveToBeginningOfDocument(sender) })
    }
    override func moveToEndOfDocumentAndModifySelection(_ sender: Any?) {
        extend(
            pointing: true,
            natively: { super.moveToEndOfDocumentAndModifySelection(sender) },
            plainly: { super.moveToEndOfDocument(sender) })
    }
    override func pageUpAndModifySelection(_ sender: Any?) {
        extend(
            pointing: false, natively: { super.pageUpAndModifySelection(sender) },
            plainly: { super.pageUp(sender) })
    }
    override func pageDownAndModifySelection(_ sender: Any?) {
        extend(
            pointing: true, natively: { super.pageDownAndModifySelection(sender) },
            plainly: { super.pageDown(sender) })
    }

    override func setSelectedRanges(
        _ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool
    ) {
        let before = selectedRange()
        super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
        if !extendingSelection {
            // A selection somebody else made. When it was made by
            // moving one end, the end that stood still is its anchor.
            // Anything else — a click, a double-click, a find, a jump —
            // is a new selection with no anchor yet, and the next arrow
            // decides which end holds still.
            selectionAnchor = keepingAnEnd ? endThatStoodStill(from: before) : nil
            appKitAnchorIsStale = false
        }
        if !tintedLine.isEmpty { setNeedsDisplay(tintedLine) }
        if let line = caretLineRect() { setNeedsDisplay(line) }
    }

    /// The anchor of a selection that was just changed by moving one
    /// end: the one already on record if it is still an end, else
    /// whichever end of the old selection is an end of the new one. A
    /// selection that shares no end with the last, or both, or is
    /// empty, has none.
    private func endThatStoodStill(from before: NSRange) -> Int? {
        let after = selectedRange()
        guard after.length > 0 else { return nil }
        let known = selectionAnchor.map { [$0] } ?? [before.location, NSMaxRange(before)]
        let still = Set(known).filter { $0 == after.location || $0 == NSMaxRange(after) }
        return still.count == 1 ? still.first : nil
    }

    /// A drag selects from where the button went down, and that place
    /// is its anchor; a shift-click moves one end of what was selected.
    /// Both run inside `super.mouseDown`, which tracks the mouse until
    /// the button comes up, so the anchor is settled when it returns.
    override func mouseDown(with event: NSEvent) {
        let before = selectedRange()
        let anchorBefore = selectionAnchor
        let pressed = characterIndexForInsertion(at: convert(event.locationInWindow, from: nil))
        let extending = event.modifierFlags.contains(.shift)
        super.mouseDown(with: event)
        let after = selectedRange()
        if extending {
            selectionAnchor = anchorBefore
            selectionAnchor = endThatStoodStill(from: before)
        } else if after.length > 0, event.clickCount == 1,
            pressed == after.location || pressed == NSMaxRange(after)
        {
            selectionAnchor = pressed
        }
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
