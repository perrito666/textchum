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
