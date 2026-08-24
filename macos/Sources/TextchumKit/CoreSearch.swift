import CTextchum
import Foundation

/// Project search: fuzzy file finding and content grep. Pure functions
/// over the file system — safe to call from any thread, and worth calling
/// off the main thread for large scopes.
public enum CoreSearch {
    /// One content-search hit.
    public struct Hit: Sendable, Equatable {
        /// Path relative to the searched root.
        public let path: String
        /// One-based line number.
        public let line: Int
        /// The matching line, trimmed.
        public let text: String
    }

    /// Fuzzy-matches file paths under `root` against `query`, best first.
    /// An empty query lists files alphabetically.
    public static func fuzzyFiles(root: String, query: String, limit: Int = 100) -> [String] {
        let joined: String? = withUTF8(root) { root, rootLen in
            withUTF8(query) { query, queryLen in
                guard
                    let cString = tc_fuzzy_files(root, rootLen, query, queryLen, UInt(limit))
                else { return nil }
                defer { tc_string_free(cString) }
                return String(cString: cString)
            }
        }
        guard let joined, !joined.isEmpty else { return [] }
        return joined.components(separatedBy: "\n")
    }

    /// A stacked refinement over grep hits: the line's text or the file's
    /// relative path must (or must not) contain `pattern`, matched as a
    /// case-insensitive substring.
    public struct Filter: Codable, Sendable, Equatable {
        public enum Kind: String, Codable, Sendable {
            case line
            case file
        }

        public var kind: Kind
        public var include: Bool
        public var pattern: String

        public init(kind: Kind, include: Bool, pattern: String) {
            self.kind = kind
            self.include = include
            self.pattern = pattern
        }
    }

    /// Searches file contents under `root` for the regex `pattern`,
    /// narrowed by `filters`. Throws a ``CoreIOError`` with the core's
    /// message on a bad pattern.
    public static func grep(
        root: String, pattern: String, caseInsensitive: Bool = false, limit: Int = 200,
        filters: [Filter] = []
    ) throws -> [Hit] {
        let filtersJSON =
            (try? JSONEncoder().encode(filters)).flatMap { String(data: $0, encoding: .utf8) }
            ?? "[]"
        var error: UnsafeMutablePointer<CChar>?
        let joined: String? = withUTF8(root) { root, rootLen in
            withUTF8(pattern) { pattern, patternLen in
                withUTF8(filtersJSON) { filters, filtersLen in
                    guard
                        let cString = tc_grep(
                            root, rootLen, pattern, patternLen, caseInsensitive,
                            UInt(limit), filters, filtersLen, &error)
                    else { return nil }
                    defer { tc_string_free(cString) }
                    return String(cString: cString)
                }
            }
        }
        if let error {
            let message = String(cString: error)
            tc_string_free(error)
            throw CoreIOError(message: message)
        }
        guard let joined, !joined.isEmpty else { return [] }
        return joined.components(separatedBy: "\n").compactMap { row in
            let fields = row.components(separatedBy: "\u{1f}")
            guard fields.count >= 3, let line = Int(fields[1]) else { return nil }
            return Hit(path: fields[0], line: line, text: fields[2])
        }
    }
}

/// Runs `body` with a `(pointer, length)` view of the string's UTF-8.
private func withUTF8<R>(
    _ text: String, _ body: (UnsafePointer<CChar>?, UInt) -> R
) -> R {
    var text = text
    return text.withUTF8 { bytes in
        let pointer = bytes.baseAddress.map {
            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
        }
        return body(pointer, UInt(bytes.count))
    }
}
