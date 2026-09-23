import CTextchum
import Foundation

/// The delimiters that come in pairs.
///
/// Typing an opening delimiter with text selected wraps the selection
/// instead of replacing it. The table is the core's, so both shells
/// wrap the same things.
public enum CorePairs {
    /// The closing half of `open`, or nil when `open` is not a
    /// delimiter that wraps — which includes a paste of several
    /// characters, since that replaces a selection as it always did.
    public static func closing(of open: String) -> String? {
        let answer = open.withCString { pointer in
            tc_pair_closing(pointer, UInt(strlen(pointer)))
        }
        guard let answer else { return nil }
        defer { tc_string_free(answer) }
        let text = String(cString: answer)
        return text.isEmpty ? nil : text
    }

    /// The closing half to put after the caret when `typed` is typed in
    /// `language` between `before` and `after` (UTF-16 units, nil at an
    /// edge), or nil when nothing should be. The rules are the core's:
    /// brackets always, quotes by language, never into a word.
    public static func autoClose(
        language: String?, typed: unichar, before: unichar?, after: unichar?
    ) -> String? {
        var language = language ?? ""
        let close = language.withUTF8 { bytes in
            tc_pairs_auto_close(
                bytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                UInt(bytes.count), UInt32(typed), UInt32(before ?? 0), UInt32(after ?? 0))
        }
        guard close != 0, let scalar = UnicodeScalar(close) else { return nil }
        return String(Character(scalar))
    }

    /// Whether typing `typed` with `after` already there steps over it.
    public static func skipsCloser(typed: unichar, after: unichar?) -> Bool {
        tc_pairs_skips_closer(UInt32(typed), UInt32(after ?? 0))
    }

    /// Whether Backspace between `before` and `after` takes both.
    public static func deletesPair(before: unichar?, after: unichar?) -> Bool {
        tc_pairs_deletes_pair(UInt32(before ?? 0), UInt32(after ?? 0))
    }
}
