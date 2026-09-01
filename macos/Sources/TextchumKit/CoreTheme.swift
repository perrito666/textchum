import AppKit
import CTextchum
import Foundation

/// One style of the core's theme, with a color per system appearance
/// (0xRRGGBBAA) and typographic flags.
public struct CoreStyle {
    public let lightRGBA: UInt32
    public let darkRGBA: UInt32
    public let isBold: Bool
    public let isItalic: Bool
}

/// The core's theme engine: the active style table, the built-in theme
/// set, and user-theme JSON.
public enum CoreTheme {
    /// The active theme's style table. Highlight spans index into it.
    /// Cached; changing themes invalidates it via ``reload()``.
    public private(set) static var styles: [CoreStyle] = fetchStyles()

    /// Re-reads the style table after a theme switch.
    public static func reload() {
        styles = fetchStyles()
    }

    /// The style id a capture resolves to, or nil when it is unstyled.
    ///
    /// Ask rather than count: the ids are positions in an alphabetical
    /// table, so adding a capture moves every one after it. Looked up
    /// once per name, since the table only changes when the core does.
    public static func styleID(for capture: String) -> UInt32? {
        if let cached = idCache[capture] { return cached }
        var capture = capture
        let id: Int32 = capture.withUTF8 { bytes in
            tc_theme_style_id(
                bytes.baseAddress.map {
                    UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                },
                UInt(bytes.count)
            )
        }
        let resolved = id < 0 ? nil : UInt32(id)
        idCache[capture] = resolved
        return resolved
    }

    private static var idCache: [String: UInt32?] = [:]

    /// The comment style, which the prose spell checker asks for often
    /// enough to be worth naming.
    public static var commentStyleID: UInt32? { styleID(for: "comment") }

    private static func fetchStyles() -> [CoreStyle] {
        var count: UInt = 0
        guard let table = tc_style_table(&count) else { return [] }
        return (0..<Int(count)).map { index in
            let style = table[index]
            return CoreStyle(
                lightRGBA: style.light,
                darkRGBA: style.dark,
                isBold: style.flags & UInt32(TC_STYLE_BOLD) != 0,
                isItalic: style.flags & UInt32(TC_STYLE_ITALIC) != 0
            )
        }
    }

    /// Built-in theme names, in presentation order.
    public static var builtinNames: [String] {
        guard let joined = tc_theme_builtin_names() else { return [] }
        defer { tc_string_free(joined) }
        return String(cString: joined).split(separator: "\n").map(String.init)
    }

    /// Activates a built-in theme; false for unknown names. Call
    /// ``reload()`` happens automatically.
    @discardableResult
    public static func setBuiltin(named name: String) -> Bool {
        var name = name
        let applied = name.withUTF8 { bytes in
            tc_theme_set_builtin(
                bytes.baseAddress.map {
                    UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                },
                UInt(bytes.count)
            )
        }
        if applied { reload() }
        return applied
    }

    /// Activates a user theme from its JSON. Returns the parse error, or
    /// nil on success (the active theme is unchanged on failure).
    public static func setJSON(_ json: String) -> String? {
        var json = json
        var errorPointer: UnsafeMutablePointer<CChar>?
        let applied = json.withUTF8 { bytes in
            tc_theme_set_json(
                bytes.baseAddress.map {
                    UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                },
                UInt(bytes.count),
                &errorPointer
            )
        }
        if applied {
            reload()
            return nil
        }
        guard let errorPointer else { return "unreadable theme" }
        defer { tc_string_free(errorPointer) }
        return String(cString: errorPointer)
    }

    /// A complete starter theme as pretty-printed JSON — every styled
    /// capture with the default palette's values.
    /// Which editor a theme file came from.
    public enum ImportSource: UInt32 {
        case vsCode = 0
        case textMate = 1

        /// The name to put in a menu and a file chooser.
        public var label: String {
            switch self {
            case .vsCode: return "VS Code"
            case .textMate: return "TextMate"
            }
        }

        /// The extensions a theme of this kind is kept in.
        public var extensions: [String] {
            switch self {
            case .vsCode: return ["json"]
            case .textMate: return ["tmTheme", "thTheme"]
            }
        }
    }

    /// What an import did.
    public struct ImportOutcome {
        /// The themes now available to choose, in the order they were
        /// read.
        public let written: [String]
        /// Which side of the palette each one filled; the other keeps
        /// the default palette's colours.
        public let appearances: [String]
        /// Scopes no capture answers to — a gap in the mapping rather
        /// than a fault in the file.
        public let unmapped: [String]
        /// One line per file that could not be read.
        public let errors: [String]
    }

    /// Imports every theme at `path` into `directory`, one JSON file per
    /// theme, named after the theme. `path` is a theme file or a folder
    /// to look inside: a VS Code extension directory, or a `.tmbundle`.
    public static func importThemes(
        at path: String,
        from source: ImportSource,
        into directory: String
    ) -> ImportOutcome {
        let json = path.withCString { pathPointer in
            directory.withCString { directoryPointer in
                tc_theme_import(
                    pathPointer,
                    UInt(strlen(pathPointer)),
                    source.rawValue,
                    directoryPointer,
                    UInt(strlen(directoryPointer))
                )
            }
        }
        guard let json else {
            return ImportOutcome(
                written: [], appearances: [], unmapped: [],
                errors: ["the import could not be started"])
        }
        defer { tc_string_free(json) }
        let data = Data(String(cString: json).utf8)
        let parsed = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
        return ImportOutcome(
            written: parsed["written"] as? [String] ?? [],
            appearances: parsed["appearances"] as? [String] ?? [],
            unmapped: parsed["unmapped"] as? [String] ?? [],
            errors: parsed["errors"] as? [String] ?? []
        )
    }

    public static var templateJSON: String {
        guard let template = tc_theme_template_json() else { return "{}" }
        defer { tc_string_free(template) }
        return String(cString: template)
    }
}

/// File icons from a VS Code icon pack: the file tree's rows, when the
/// desktop's own icons run out (which is early — LaunchServices knows
/// Python from Markdown and has never heard of `Dockerfile`).
///
/// The pack is loaded once and lives in the core, so both shells draw
/// the same icon for the same file.
@MainActor
public enum CoreIcons {
    /// Loads the pack at `path` — an icon theme JSON, or the extension
    /// folder holding one — returning a line describing what was
    /// loaded, or throwing with why it could not be.
    @discardableResult
    public static func load(at path: String) throws -> String {
        var error: UnsafeMutablePointer<CChar>?
        let summary = path.withCString { pointer in
            tc_icons_load(pointer, UInt(strlen(pointer)), &error)
        }
        guard let summary else {
            let message =
                error.map { pointer -> String in
                    defer { tc_string_free(pointer) }
                    return String(cString: pointer)
                } ?? "the icon pack could not be read"
            throw CoreIOError(message: message)
        }
        defer { tc_string_free(summary) }
        cache.removeAll()
        imagesByFile.removeAll()
        return String(cString: summary)
    }

    /// Forgets the pack, returning the tree to the desktop's icons.
    public static func clear() {
        tc_icons_clear()
        cache.removeAll()
        imagesByFile.removeAll()
    }

    /// Whether a pack is loaded.
    public static var isActive: Bool { tc_icons_active() }

    /// The icon for a file, or nil when no pack is loaded or it has
    /// nothing for this one. Images are cached by what was asked, since
    /// a file tree asks for the same handful over and over as it
    /// scrolls.
    public static func icon(
        forFilename filename: String, language: String?, light: Bool
    ) -> NSImage? {
        guard tc_icons_active() else { return nil }
        let key = "\(light ? "l" : "d")\u{1f}\(language ?? "")\u{1f}\(filename)"
        if let cached = cache[key] { return cached }
        let path = filename.withCString { name -> UnsafeMutablePointer<CChar>? in
            guard let language else {
                return tc_icons_for_file(name, UInt(strlen(name)), nil, 0, light)
            }
            return language.withCString { languagePointer in
                tc_icons_for_file(
                    name, UInt(strlen(name)),
                    languagePointer, UInt(strlen(languagePointer)), light)
            }
        }
        var image: NSImage?
        if let path {
            defer { tc_string_free(path) }
            let file = String(cString: path)
            // A pack maps thousands of filenames onto a handful of
            // icon files; loading the image once per icon file — not
            // once per filename — is what keeps the first scroll
            // through a monorepo from reading the disk per row.
            if let shared = imagesByFile[file] {
                image = shared
            } else {
                image = NSImage(contentsOfFile: file)
                image?.size = NSSize(width: 16, height: 16)
                imagesByFile[file] = image
            }
        }
        cache[key] = image
        return image
    }

    private static var cache: [String: NSImage?] = [:]
    private static var imagesByFile: [String: NSImage?] = [:]
}
