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
}
