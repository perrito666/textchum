import CTextchum
import Foundation

/// Colour for text shown away from its file: a line in the references
/// list, a place in the jump history. The core paints it the way a
/// document of that language would be painted.
public enum CoreSnippet {
    public static func highlights(language: String, text: String) -> [CoreDocument.HighlightSpan] {
        var language = language
        var text = text
        var spans: UnsafeMutablePointer<TcHighlightSpan>?
        var count: UInt = 0
        let ok = language.withUTF8 { languageBytes in
            text.withUTF8 { textBytes in
                tc_highlight_snippet(
                    languageBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(languageBytes.count),
                    textBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(textBytes.count),
                    &spans,
                    &count)
            }
        }
        guard ok, let spans else { return [] }
        defer { tc_highlight_spans_free(spans, count) }
        return (0..<Int(count)).map { index in
            let span = spans[index]
            return CoreDocument.HighlightSpan(
                range: NSRange(location: Int(span.start), length: Int(span.end - span.start)),
                styleIndex: Int(span.style))
        }
    }
}
