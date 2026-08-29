import CTextchum
import Foundation

/// The core's workspace model: which project a file belongs to.
public enum CoreWorkspace {
    /// The project root for a file or directory path, resolved under the
    /// given workspace settings JSON (the configuration's `workspace`
    /// section; "{}" for defaults) — or nil for loose files outside any
    /// project.
    public static func projectRoot(forPath path: String, settingsJSON: String = "{}") -> String?
    {
        var path = path
        var settings = settingsJSON
        let cString: UnsafeMutablePointer<CChar>? = path.withUTF8 { pathBytes in
            settings.withUTF8 { settingsBytes in
                tc_project_root_for_path(
                    pathBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(pathBytes.count),
                    settingsBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(settingsBytes.count)
                )
            }
        }
        guard let cString else { return nil }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }

    /// A boolean workspace flag for a project root, resolved with the
    /// standard rules: the root's own entry, else the top-level default,
    /// else false.
    public static func flag(_ key: String, root: String, settingsJSON: String) -> Bool {
        var key = key
        var root = root
        var settings = settingsJSON
        return settings.withUTF8 { settingsBytes in
            root.withUTF8 { rootBytes in
                key.withUTF8 { keyBytes in
                    tc_workspace_flag(
                        settingsBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(settingsBytes.count),
                        rootBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(rootBytes.count),
                        keyBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(keyBytes.count)
                    )
                }
            }
        }
    }

    /// Points the language-server debug log at a file, created (with
    /// parent directories) on first write and appended to.
    /// A Markdown document's headings — level, position, and text —
    /// for the outline a post deserves when no server answers.
    public static func markdownHeadings(in text: String) -> [
        (level: Int, line: Int, character: Int, text: String)
    ] {
        var text = text
        let joined: String? = text.withUTF8 { bytes in
            guard
                let cString = tc_markdown_headings(
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count))
            else { return nil }
            defer { tc_string_free(cString) }
            return String(cString: cString)
        }
        guard let joined, !joined.isEmpty else { return [] }
        return joined.split(separator: "\n").compactMap { line in
            let parts = line.split(separator: "\u{1f}", maxSplits: 3)
            guard parts.count == 4, let level = Int(parts[0]), let row = Int(parts[1]),
                let character = Int(parts[2])
            else { return nil }
            return (level, row, character, String(parts[3]))
        }
    }

    /// UTF-16 ranges a spell checker must skip in a Hugo document:
    /// front matter and shortcode calls.
    public static func hugoNonProseRanges(in text: String) -> [NSRange] {
        var text = text
        let joined: String? = text.withUTF8 { bytes in
            guard
                let cString = tc_hugo_non_prose_ranges(
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count))
            else { return nil }
            defer { tc_string_free(cString) }
            return String(cString: cString)
        }
        guard let joined, !joined.isEmpty else { return [] }
        return joined.split(separator: "\n").compactMap { line in
            let parts = line.split(separator: "\u{1f}")
            guard parts.count == 2, let start = Int(parts[0]), let end = Int(parts[1]),
                end > start
            else { return nil }
            return NSRange(location: start, length: end - start)
        }
    }

    /// Whether `name` is hidden by any of the navigator globs.
    public static func isHidden(name: String, globs: [String]) -> Bool {
        var name = name
        var joined = globs.joined(separator: "\n")
        return name.withUTF8 { nameBytes in
            joined.withUTF8 { globBytes in
                tc_workspace_is_hidden(
                    nameBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(nameBytes.count),
                    globBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(globBytes.count)
                )
            }
        }
    }

    public static func setLSPLogPath(_ path: String) {
        var path = path
        path.withUTF8 { bytes in
            tc_lsp_set_log_path(
                bytes.baseAddress.map {
                    UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                },
                UInt(bytes.count)
            )
        }
    }
}

/// Telling a test apart from the code it tests, by the naming
/// conventions of the languages this editor knows — a `tests` directory,
/// a `parser_test.go`, a `Button.test.ts`. A heuristic, and a cautious
/// one: `latest.rs` is not a test.
public enum CoreReferences {
    public static func isTest(path: String) -> Bool {
        path.withCString { pointer in
            tc_path_is_test(pointer, UInt(strlen(pointer)))
        }
    }
}

/// The other places the selected word appears.
///
/// Selecting a word and then typing it into the find bar to see where
/// else it is asks twice for one thing.
public enum CoreOccurrences {
    /// A span in the document, in UTF-16 units.
    public struct Span {
        public let start: Int
        public let end: Int
    }

    /// Occurrences of the selected word inside `text`, which is the
    /// visible stretch and starts at `base` in the document. Empty
    /// when the selection is not exactly one word.
    public static func marks(
        in text: String, selection: Range<Int>, base: Int,
        caseSensitive: Bool, wholeWord: Bool
    ) -> [Span] {
        var text = text
        let raw = text.withUTF8 { bytes -> UnsafeMutablePointer<CChar>? in
            tc_occurrences_of_selection(
                bytes.baseAddress.map {
                    UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                },
                UInt(bytes.count),
                UInt(max(0, selection.lowerBound)), UInt(max(0, selection.upperBound)),
                UInt(max(0, base)), caseSensitive, wholeWord)
        }
        guard let raw else { return [] }
        defer { tc_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        let parsed = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] ?? []
        return parsed.compactMap { item in
            guard let start = item["start"] as? Int, let end = item["end"] as? Int
            else { return nil }
            return Span(start: start, end: end)
        }
    }
}

/// What to do with a definition answer.
///
/// Jump to Definition has nowhere to go when the caret is already on
/// the definition, so the same key asks who uses the symbol instead.
/// The answer already in hand decides it.
public enum CoreDefinition {
    /// A place a server pointed at, in LSP terms.
    public struct Target {
        public let path: String
        public let line: Int
        public let character: Int
    }

    public enum Decision {
        /// The server pointed nowhere.
        case nothing
        /// One place, elsewhere.
        case jump(Target)
        /// The caret is on the definition: ask who uses it.
        case references
        /// Several places; the reader picks.
        case choose([Target])
    }

    /// Reads a `textDocument/definition` result. `line` and `character`
    /// are the caret, zero-based and in UTF-16 units.
    public static func decide(
        result: String, path: String, line: Int, character: Int
    ) -> Decision {
        let json = call(
            tc_definition_decide, result: result, path: path,
            line: line, character: character)
        guard let json,
            let parsed = try? JSONSerialization.jsonObject(with: Data(json.utf8))
                as? [String: Any],
            let action = parsed["action"] as? String
        else { return .nothing }
        let targets = targets(from: parsed["targets"])
        switch action {
        case "jump": return targets.first.map(Decision.jump) ?? .nothing
        case "references": return .references
        case "choose": return targets.isEmpty ? .nothing : .choose(targets)
        default: return .nothing
        }
    }

    /// The reference locations that are not the one the caret is in.
    /// Find References includes the declaration, and a definition
    /// nobody calls answers with the line the caret is on.
    public static func elsewhere(
        result: String, path: String, line: Int, character: Int
    ) -> [Target] {
        let json = call(
            tc_references_elsewhere, result: result, path: path,
            line: line, character: character)
        guard let json else { return [] }
        return targets(from: try? JSONSerialization.jsonObject(with: Data(json.utf8)))
    }

    private static func call(
        _ function: (UnsafePointer<CChar>?, UInt, UnsafePointer<CChar>?, UInt, UInt32, UInt32)
            -> UnsafeMutablePointer<CChar>?,
        result: String, path: String, line: Int, character: Int
    ) -> String? {
        var result = result
        let raw = result.withUTF8 { bytes -> UnsafeMutablePointer<CChar>? in
            path.withCString { pathPointer in
                function(
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count),
                    pathPointer, UInt(strlen(pathPointer)),
                    UInt32(max(0, line)), UInt32(max(0, character)))
            }
        }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    private static func targets(from parsed: Any?) -> [Target] {
        guard let items = parsed as? [[String: Any]] else { return [] }
        return items.compactMap { item in
            guard let path = item["path"] as? String,
                let line = item["line"] as? Int,
                let character = item["character"] as? Int
            else { return nil }
            return Target(path: path, line: line, character: character)
        }
    }
}

/// Which lines of a file differ from the same file at `HEAD` — what the
/// gutter draws beside the line numbers.
public enum CoreChanges {
    public enum Kind: String {
        case added
        case modified
        /// Lines that are gone. Nothing occupies their place, so the
        /// mark sits on the boundary above `line`.
        case removed
    }

    /// A mark on a zero-based line.
    public struct Mark {
        public let line: Int
        public let kind: Kind
    }

    /// The marks for `path` given the buffer's current `text`. Empty
    /// when there is nothing to compare against: a file with no
    /// committed version, one outside a repository, or a machine with
    /// no git. None of those is an error.
    public static func marks(forPath path: String, text: String) -> [Mark] {
        let json = path.withCString { pathPointer -> UnsafeMutablePointer<CChar>? in
            var text = text
            return text.withUTF8 { bytes in
                tc_changes_for_file(
                    pathPointer, UInt(strlen(pathPointer)),
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count))
            }
        }
        guard let json else { return [] }
        defer { tc_string_free(json) }
        let data = Data(String(cString: json).utf8)
        let parsed = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] ?? []
        return parsed.compactMap { item in
            guard let line = item["line"] as? Int,
                let name = item["kind"] as? String,
                let kind = Kind(rawValue: name)
            else { return nil }
            return Mark(line: line, kind: kind)
        }
    }
}

/// What git knows about one line: who wrote it, when, and on which
/// commit.
public enum CoreBlame {
    public struct Line {
        /// The line this is about. Not always the one asked for: a
        /// caret past the end of the file is answered about the last
        /// line there is.
        public let line: Int
        public let commit: String
        public let abbreviated: String
        public let author: String
        public let authorMail: String
        public let authorDate: String
        /// Set only when the committer is a different story from the
        /// author — a rebase, a cherry-pick, a patch applied by hand.
        public let committer: String
        public let committerDate: String
        public let summary: String
        public let body: String
        /// Set only when the file has been renamed since.
        public let renamedFrom: String
        /// Typed and not yet committed; carries no commit.
        public let uncommitted: Bool
    }

    /// Blames one-based `line` of `path` against `text` — the buffer's
    /// contents, so the answer is about the line on screen rather than
    /// the one at that number in the saved file.
    public static func line(
        _ line: Int, ofPath path: String, text: String
    ) throws -> Line {
        var error: UnsafeMutablePointer<CChar>?
        let json = path.withCString { pathPointer -> UnsafeMutablePointer<CChar>? in
            var text = text
            return text.withUTF8 { bytes in
                tc_blame_line(
                    pathPointer, UInt(strlen(pathPointer)), UInt(line),
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count), &error)
            }
        }
        guard let json else {
            let message =
                error.map { pointer -> String in
                    defer { tc_string_free(pointer) }
                    return String(cString: pointer)
                } ?? "git could not blame this line"
            throw CoreIOError(message: message)
        }
        defer { tc_string_free(json) }
        let data = Data(String(cString: json).utf8)
        let parsed = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
        func string(_ key: String) -> String { parsed[key] as? String ?? "" }
        return Line(
            line: parsed["line"] as? Int ?? line,
            commit: string("commit"),
            abbreviated: string("abbreviated"),
            author: string("author"),
            authorMail: string("authorMail"),
            authorDate: string("authorDate"),
            committer: string("committer"),
            committerDate: string("committerDate"),
            summary: string("summary"),
            body: string("body"),
            renamedFrom: string("renamedFrom"),
            uncommitted: parsed["uncommitted"] as? Bool ?? false
        )
    }
}
