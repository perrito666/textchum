import AppKit
import TextchumKit

/// The core's style table resolved to `NSColor`s, cached per appearance.
enum HighlightPalette {
    private static var light: [NSColor] = CoreTheme.styles.map { color(rgba: $0.lightRGBA) }
    private static var dark: [NSColor] = CoreTheme.styles.map { color(rgba: $0.darkRGBA) }

    /// Rebuilds the cache after a theme switch (CoreTheme.reload first).
    static func reload() {
        light = CoreTheme.styles.map { color(rgba: $0.lightRGBA) }
        dark = CoreTheme.styles.map { color(rgba: $0.darkRGBA) }
    }

    /// The color for a style index under the given appearance.
    static func color(forStyle index: Int, darkAppearance: Bool) -> NSColor? {
        let table = darkAppearance ? dark : light
        guard table.indices.contains(index) else { return nil }
        return table[index]
    }

    /// The typographic traits a style asks for. The core has carried
    /// these all along; until now the overlay dropped them on the
    /// floor, so a theme asking for italic comments got the colour and
    /// silence.
    static func traits(forStyle index: Int) -> (bold: Bool, italic: Bool) {
        let styles = CoreTheme.styles
        guard styles.indices.contains(index) else { return (false, false) }
        return (styles[index].isBold, styles[index].isItalic)
    }

    /// Whether any style in the active theme asks for bold or italic —
    /// themes that do not get the cheap colour-only path.
    static var hasTypographicStyles: Bool {
        CoreTheme.styles.contains { $0.isBold || $0.isItalic }
    }

    private static func color(rgba: UInt32) -> NSColor {
        NSColor(
            srgbRed: CGFloat((rgba >> 24) & 0xFF) / 255,
            green: CGFloat((rgba >> 16) & 0xFF) / 255,
            blue: CGFloat((rgba >> 8) & 0xFF) / 255,
            alpha: CGFloat(rgba & 0xFF) / 255
        )
    }
}

/// User theme files: `~/Library/Application Support/Textchum/themes/`,
/// one JSON per theme, selected by file name (without extension). A file
/// sharing a built-in theme's name overrides it.
enum ThemeFiles {
    static var directory: URL { AppPaths.themesDirectory }

    static var names: [String] {
        let entries =
            (try? FileManager.default.contentsOfDirectory(
                at: directory, includingPropertiesForKeys: nil)) ?? []
        return
            entries
            .filter { $0.pathExtension == "json" }
            .map { $0.deletingPathExtension().lastPathComponent }
            .sorted()
    }

    static func json(named name: String) -> String? {
        try? String(
            contentsOf: directory.appendingPathComponent(name + ".json"),
            encoding: .utf8)
    }
}
