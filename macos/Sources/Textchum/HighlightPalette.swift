import AppKit
import TextchumKit

/// The core's style table resolved to `NSColor`s, cached per appearance.
enum HighlightPalette {
    private static let light: [NSColor] = CoreTheme.styles.map { color(rgba: $0.lightRGBA) }
    private static let dark: [NSColor] = CoreTheme.styles.map { color(rgba: $0.darkRGBA) }

    /// The color for a style index under the given appearance.
    static func color(forStyle index: Int, darkAppearance: Bool) -> NSColor? {
        let table = darkAppearance ? dark : light
        guard table.indices.contains(index) else { return nil }
        return table[index]
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
