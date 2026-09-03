import AppKit
import TextchumKit

/// Lines of files shown away from their file — a reference, a place in
/// the jump history — read once per file and coloured the way the
/// editor would colour them.
@MainActor
final class SnippetRows {
    private var lines: [String: [Substring]] = [:]

    /// The line's text, trimmed, or empty past the end of the file.
    func lineText(path: String, line: Int) -> String {
        if lines[path] == nil {
            let contents = (try? String(contentsOfFile: path, encoding: .utf8)) ?? ""
            lines[path] = contents.split(separator: "\n", omittingEmptySubsequences: false)
        }
        let all = lines[path] ?? []
        guard all.indices.contains(line) else { return "" }
        return all[line].trimmingCharacters(in: .whitespaces)
    }

    /// `name:line: text`, the text in the language's colours.
    func row(path: String, line: Int) -> ListPanel.Row {
        let text = lineText(path: path, line: line)
        let name = (path as NSString).lastPathComponent
        let plain = "\(name):\(line + 1): \(text)"
        let font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        let styled = NSMutableAttributedString(
            string: "\(name):\(line + 1): ",
            attributes: [.font: font, .foregroundColor: NSColor.secondaryLabelColor])
        styled.append(
            Self.styled(text, language: CoreLanguages.detected(forPath: path), font: font))
        return .item(plain, styled: styled)
    }

    /// `text` in the colours of `language`; plain when there is none.
    static func styled(_ text: String, language: String?, font: NSFont) -> NSAttributedString {
        let painted = NSMutableAttributedString(
            string: text, attributes: [.font: font, .foregroundColor: NSColor.textColor])
        guard let language else { return painted }
        let darkAppearance =
            NSApp.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        let length = (text as NSString).length
        for span in CoreSnippet.highlights(language: language, text: text) {
            let range = NSIntersectionRange(span.range, NSRange(location: 0, length: length))
            guard range.length > 0 else { continue }
            if let color = HighlightPalette.color(
                forStyle: span.styleIndex, darkAppearance: darkAppearance)
            {
                painted.addAttribute(.foregroundColor, value: color, range: range)
            }
        }
        return painted
    }
}
