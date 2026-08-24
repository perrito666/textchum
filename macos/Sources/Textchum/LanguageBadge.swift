import SwiftUI

/// A small language badge for file rows: the language's conventional
/// short label on its (linguist-style) color. At sidebar sizes a real
/// logo would be mush; two letters on the community color reads
/// instantly. Unknown files keep the generic document glyph.
struct LanguageBadge: View {
    let filename: String

    var body: some View {
        if let badge = Self.badge(for: filename) {
            Text(badge.label)
                .font(.system(size: 7.5, weight: .bold, design: .rounded))
                .foregroundColor(badge.darkText ? Color.black.opacity(0.72) : .white)
                .frame(width: 17, height: 13)
                .background(
                    RoundedRectangle(cornerRadius: 3).fill(badge.color)
                )
        } else {
            Image(systemName: "doc.text")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .frame(width: 17)
        }
    }

    struct Badge {
        let label: String
        let color: Color
        /// Light backgrounds (yellow, tan) need dark text.
        let darkText: Bool
    }

    static func badge(for filename: String) -> Badge? {
        // Files whose identity is their name, not an extension.
        switch filename {
        case "COMMIT_EDITMSG", "MERGE_MSG", "TAG_EDITMSG":
            return Badge(label: "git", color: Color(hex: 0xF05033), darkText: false)
        case "Makefile", "makefile", "GNUmakefile":
            return Badge(label: "mk", color: Color(hex: 0x6D8086), darkText: false)
        default:
            break
        }
        let ext = (filename as NSString).pathExtension.lowercased()
        switch ext {
        case "rs": return Badge(label: "rs", color: Color(hex: 0xDEA584), darkText: true)
        case "py", "pyi": return Badge(label: "py", color: Color(hex: 0x3572A5), darkText: false)
        case "go": return Badge(label: "go", color: Color(hex: 0x00ADD8), darkText: false)
        case "js", "mjs", "cjs", "jsx":
            return Badge(label: "js", color: Color(hex: 0xF1E05A), darkText: true)
        case "ts", "tsx": return Badge(label: "ts", color: Color(hex: 0x3178C6), darkText: false)
        case "json", "jsonc":
            return Badge(label: "{}", color: Color(hex: 0x8A8A8A), darkText: false)
        case "yaml", "yml":
            return Badge(label: "yml", color: Color(hex: 0xCB171E), darkText: false)
        case "toml": return Badge(label: "tml", color: Color(hex: 0x9C4221), darkText: false)
        case "html", "htm":
            return Badge(label: "ht", color: Color(hex: 0xE34C26), darkText: false)
        case "css": return Badge(label: "css", color: Color(hex: 0x663399), darkText: false)
        case "md", "markdown":
            return Badge(label: "md", color: Color(hex: 0x083FA1), darkText: false)
        case "sh", "bash", "zsh":
            return Badge(label: "sh", color: Color(hex: 0x89E051), darkText: true)
        case "c", "h": return Badge(label: "c", color: Color(hex: 0x555555), darkText: false)
        case "swift": return Badge(label: "sw", color: Color(hex: 0xF05138), darkText: false)
        case "zig": return Badge(label: "zig", color: Color(hex: 0xEC915C), darkText: true)
        case "mk", "mak": return Badge(label: "mk", color: Color(hex: 0x6D8086), darkText: false)
        default: return nil
        }
    }
}

extension Color {
    /// 0xRRGGBB.
    init(hex: UInt32) {
        self.init(
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}
