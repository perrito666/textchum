import CTextchum
import Foundation

/// Moving through code by the shapes code has; the rules are the
/// core's, so both shells move alike.
public enum CoreMotion {
    /// Where a word move from `offset` (UTF-16) lands.
    public static func wordBoundary(in text: String, from offset: Int, forward: Bool) -> Int {
        var text = text
        return text.withUTF8 { bytes in
            Int(
                tc_word_boundary(
                    bytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                    UInt(bytes.count), UInt(max(0, offset)), forward))
        }
    }

    /// What a closing bracket typed at `offset` asks of its line: the
    /// leading blanks to replace and the indentation to put there, or
    /// nil when the line already carries text or no opener matches.
    public static func closingBracketIndent(
        in text: String, at offset: Int, closer: Character
    ) -> (blanks: NSRange, indent: String)? {
        guard let scalar = closer.unicodeScalars.first else { return nil }
        var text = text
        let raw = text.withUTF8 { bytes in
            tc_closing_bracket_indent(
                bytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                UInt(bytes.count), UInt(max(0, offset)), scalar.value)
        }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        let json = String(cString: raw)
        guard !json.isEmpty, let data = json.data(using: .utf8),
            let parsed = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
            let start = parsed["start"] as? Int, let end = parsed["end"] as? Int,
            let indent = parsed["indent"] as? String
        else { return nil }
        return (NSRange(location: start, length: end - start), indent)
    }
}
