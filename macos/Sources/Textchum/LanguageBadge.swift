import SwiftUI
import TextchumKit
import UniformTypeIdentifiers

/// The icon for a file row, in the order of who knows most about the
/// file: an icon pack if one is loaded (it knows hundreds of types, and
/// knows `Dockerfile` by name), then the type's own Finder icon where
/// macOS actually differentiates it (LaunchServices knows Python,
/// Markdown, HTML, and whatever installed apps registered), then the
/// colored language badge where the system would serve the same generic
/// document for everything — which is the genericness the icons exist
/// to avoid.
struct FileTypeIcon: View {
    let filename: String
    /// What the editor decided the file is, when it knows: a pack lists
    /// some types by language rather than by name, and a file whose
    /// language was set by hand should get the icon it was told about.
    var language: String? = nil
    @Environment(\.colorScheme) private var colorScheme
    /// Drawn again when a type's system icon has been worked out.
    @ObservedObject private var iconNews = FileIconNews.shared

    var body: some View {
        if let packed = CoreIcons.icon(
            forFilename: filename, language: language, light: colorScheme == .light)
        {
            Image(nsImage: packed)
                .resizable()
                .interpolation(.high)
                .frame(width: 16, height: 16)
        } else if let icon = SystemFileIcon.icon(forFilename: filename) {
            Image(nsImage: icon)
                .frame(width: 17)
        } else {
            LanguageBadge(filename: filename)
        }
    }
}

/// Finder icons per extension, cached — with two crucial filters. An
/// icon equal to a generic baseline (plain data, plain text, generic
/// source code) means "the system has nothing". And an icon shared by
/// several *different* known types is a default handler stamping its
/// own document icon on everything it claims (an IDE claiming .py,
/// .md, and .yml alike) — identical everywhere and misleading, so it
/// counts as nothing too. Only genuinely type-specific icons survive.
///
/// Telling icons apart means comparing their TIFF data, and that is
/// dear: three quarters of a second for the known types together, and
/// some twenty milliseconds for each extension met after them. It used
/// to be paid by the first row that drew a file, on the main thread,
/// which is where a window opening on a project stood still. It is
/// worked out in the background now: until an answer is in, a file
/// wears its badge, and the rows are told when there is a better one.
@MainActor
enum SystemFileIcon {
    /// What is known so far, by extension: an icon, or nil for "the
    /// system has nothing to say". An extension not here has not been
    /// answered yet.
    private static var answers: [String: NSImage?] = [:]
    private static var asked: Set<String> = []
    /// The generic icons' data, once the known types have been worked
    /// out; extensions met before then wait for it.
    private static var baselines: [Data]?
    private static var waiting: Set<String> = []
    /// Whether the known types have been answered, for the tests.
    static var isWarm: Bool { baselines != nil }
    private static let work = DispatchQueue(label: "textchum.file-icons", qos: .utility)

    private static let knownExtensions = [
        "rs", "py", "go", "js", "ts", "json", "yaml", "yml", "toml",
        "html", "css", "md", "sh", "c", "h", "swift", "zig", "mk",
    ]

    /// Starts working out the known types' icons. Called at launch, so
    /// the answers are usually in before the first row asks; harmless
    /// to call again.
    static func warmUp() {
        guard baselines == nil, asked.isEmpty else { return }
        asked.formUnion(knownExtensions)
        let extensions = knownExtensions
        work.async {
            let generic = [UTType.data, .item, .plainText, .sourceCode, .text].compactMap {
                NSWorkspace.shared.icon(for: $0).tiffRepresentation
            }
            // The badge-known extensions resolved together, keeping
            // only icons that are non-generic AND unique to their type.
            var byData: [Data: [(String, NSImage)]] = [:]
            for ext in extensions {
                guard let type = UTType(filenameExtension: ext) else { continue }
                let icon = NSWorkspace.shared.icon(for: type)
                guard let data = icon.tiffRepresentation, !generic.contains(data) else {
                    continue
                }
                byData[data, default: []].append((ext, icon))
            }
            var unique: [String: NSImage] = [:]
            for owners in byData.values where owners.count == 1 {
                let (ext, icon) = owners[0]
                icon.size = NSSize(width: 16, height: 16)
                unique[ext] = icon
            }
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    baselines = generic
                    for ext in extensions { answers[ext] = .some(unique[ext]) }
                    let held = waiting
                    waiting = []
                    for ext in held { resolve(ext, against: generic) }
                    FileIconNews.shared.announce()
                }
            }
        }
    }

    /// The icon for a file's type, as far as it is known right now.
    /// Never waits: an extension not yet answered gets nil, is looked
    /// into, and `FileIconNews` says when the answer is in.
    static func icon(forFilename filename: String) -> NSImage? {
        let ext = (filename as NSString).pathExtension.lowercased()
        guard !ext.isEmpty else { return nil }
        if let answer = answers[ext] { return answer }
        warmUp()
        guard !asked.contains(ext) else { return nil }
        guard LanguageBadge.badge(for: filename) == nil else {
            // A known language whose system icon failed the filters:
            // the badge is the more honest picture.
            answers[ext] = .some(nil)
            return nil
        }
        // Unknown-to-us types have no badge to offer, so any
        // non-generic system icon is an upgrade over the plain glyph.
        asked.insert(ext)
        if let baselines {
            resolve(ext, against: baselines)
        } else {
            waiting.insert(ext)
        }
        return nil
    }

    private static func resolve(_ ext: String, against generic: [Data]) {
        work.async {
            var result: NSImage?
            if let type = UTType(filenameExtension: ext) {
                let icon = NSWorkspace.shared.icon(for: type)
                if let data = icon.tiffRepresentation, !generic.contains(data) {
                    icon.size = NSSize(width: 16, height: 16)
                    result = icon
                }
            }
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    answers[ext] = .some(result)
                    // Only an icon changes what a row shows.
                    if result != nil { FileIconNews.shared.announce() }
                }
            }
        }
    }
}

/// Says when a file type's icon has been worked out, so rows drawn
/// before it can draw again. Announcements made in one turn of the
/// runloop arrive as one.
@MainActor
final class FileIconNews: ObservableObject {
    static let shared = FileIconNews()
    @Published private(set) var edition = 0
    private var owed = false

    func announce() {
        guard !owed else { return }
        owed = true
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                self.owed = false
                self.edition += 1
            }
        }
    }
}

/// A small language badge: the language's conventional short label on
/// its (linguist-style) color. At sidebar sizes a real logo would be
/// mush; two letters on the community color reads instantly. Unknown
/// files keep the generic document glyph.
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
