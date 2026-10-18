import AppKit
import TextchumKit

/// The code around a place, with its line marked: what a list of places
/// shows beside or under the row that is selected (3.5). Read from the
/// open document when there is one, which may be ahead of the disk,
/// else from the file.
@MainActor
final class PlacePreview {
    let scroll = NSScrollView()
    private let text = NSTextView()

    /// What it shows, for the smoke test.
    var shown: String { text.string }

    init() {
        text.isEditable = false
        text.isSelectable = true
        text.drawsBackground = false
        text.textContainerInset = NSSize(width: 8, height: 8)
        text.textContainer?.widthTracksTextView = false
        text.isHorizontallyResizable = true
        text.textContainer?.containerSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.borderType = .noBorder
    }

    /// Shows the zero-based `line` of `path` among the lines around it;
    /// nil clears it.
    func show(path: String?, line: Int) {
        guard let path else {
            text.string = ""
            return
        }
        let contents =
            (NSApp.delegate as? AppDelegate)?.openText(ofPath: path)
            ?? (try? String(contentsOfFile: path, encoding: .utf8)) ?? ""
        let (first, lines) = CoreReferences.preview(of: contents, line: line)
        let font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        let numberWidth = String(first + lines.count).count
        let shownText = NSMutableAttributedString()
        for (offset, content) in lines.enumerated() {
            let number = String(first + offset + 1)
            let gutter = String(repeating: " ", count: numberWidth - number.count) + number + "  "
            let piece = NSMutableAttributedString(
                string: gutter, attributes: [.font: font, .foregroundColor: NSColor.tertiaryLabelColor])
            piece.append(
                NSAttributedString(
                    string: content + "\n", attributes: [.font: font, .foregroundColor: NSColor.labelColor]))
            if first + offset == line {
                piece.addAttribute(
                    .backgroundColor, value: NSColor.selectedTextBackgroundColor.withAlphaComponent(0.6),
                    range: NSRange(location: 0, length: piece.length))
            }
            shownText.append(piece)
        }
        text.textStorage?.setAttributedString(shownText)
        text.scroll(.zero)
    }

    /// Shows unified diff text: added lines green, removed red, the
    /// hunk headers dimmed.
    func show(diff: String) {
        let font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        let shownText = NSMutableAttributedString()
        for line in diff.split(separator: "\n", omittingEmptySubsequences: false) {
            let colour: NSColor =
                line.hasPrefix("+") ? .systemGreen
                : line.hasPrefix("-") ? .systemRed
                : line.hasPrefix("@@") ? .tertiaryLabelColor : .labelColor
            shownText.append(
                NSAttributedString(string: line + "\n", attributes: [.font: font, .foregroundColor: colour]))
        }
        text.textStorage?.setAttributedString(shownText)
        text.scroll(.zero)
    }
}
