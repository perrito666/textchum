import AppKit

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

    override func setSelectedRanges(
        _ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool
    ) {
        super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
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
