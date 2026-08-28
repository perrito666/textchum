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
    public static var templateJSON: String {
        guard let template = tc_theme_template_json() else { return "{}" }
        defer { tc_string_free(template) }
        return String(cString: template)
    }
}
