import CTextchum

/// One style of the core's theme, with a color per system appearance
/// (0xRRGGBBAA) and typographic flags.
public struct CoreStyle {
    public let lightRGBA: UInt32
    public let darkRGBA: UInt32
    public let isBold: Bool
    public let isItalic: Bool
}

/// The core's style table. Highlight spans index into ``styles``.
public enum CoreTheme {
    public static let styles: [CoreStyle] = {
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
    }()
}
