import CTextchum
import Foundation

/// The delimiters that come in pairs.
///
/// Typing an opening delimiter with text selected wraps the selection
/// instead of replacing it; with nothing selected and the setting on,
/// it brings its closing half along. The rules are the core's — a
/// language's own table from the file when it has one (`table`, from
/// `CoreConfig.pairTables`), the built-in rule by language otherwise —
/// so both shells pair the same things in the same way.
public enum CorePairs {
    /// The closing half of `open` for wrapping a selection, or nil when
    /// `open` is not a delimiter under `table`.
    public static func closing(table: [String]?, open: unichar) -> String? {
        character(withTable(table) { tc_pairs_closing($0, $1, UInt32(open)) })
    }

    /// The closing half to put after the caret when `typed` is typed in
    /// `language` between `before` and `after` (UTF-16 units, nil at an
    /// edge), or nil when nothing should be: brackets always and quotes
    /// by language, never into a word — or whatever `table` says.
    public static func autoClose(
        table: [String]?, language: String?, typed: unichar, before: unichar?, after: unichar?
    ) -> String? {
        let close = withTable(table) { table, tableLength in
            (language ?? "").withCString { name in
                tc_pairs_auto_close(
                    table, tableLength, name, UInt(strlen(name)),
                    UInt32(typed), UInt32(before ?? 0), UInt32(after ?? 0))
            }
        }
        return character(close)
    }

    /// Whether typing `typed` with `after` already there steps over it.
    public static func skipsCloser(table: [String]?, typed: unichar, after: unichar?) -> Bool {
        withTable(table) { tc_pairs_skips_closer($0, $1, UInt32(typed), UInt32(after ?? 0)) }
    }

    /// Whether Backspace between `before` and `after` takes both.
    public static func deletesPair(table: [String]?, before: unichar?, after: unichar?) -> Bool {
        withTable(table) { tc_pairs_deletes_pair($0, $1, UInt32(before ?? 0), UInt32(after ?? 0)) }
    }

    /// `table` as the core takes it: one string of two characters per
    /// pair, or null for the built-in rule. An empty table is still a
    /// table — nothing pairs — so it goes over as an empty string, not
    /// as null. An entry that is not two characters is not a pair and
    /// is left out.
    private static func withTable<T>(
        _ table: [String]?, _ body: (UnsafePointer<CChar>?, UInt) -> T
    ) -> T {
        guard let table else { return body(nil, 0) }
        let flat = table.filter { $0.unicodeScalars.count == 2 }.joined()
        return flat.withCString { body($0, UInt(strlen($0))) }
    }

    /// The core's answer as a string, or nil for its 0.
    private static func character(_ scalar: UInt32) -> String? {
        guard scalar != 0, let scalar = UnicodeScalar(scalar) else { return nil }
        return String(Character(scalar))
    }
}
