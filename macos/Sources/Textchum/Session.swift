import Foundation

/// The saved session: which files were open and where the user was in
/// them, so relaunching continues where things left off.
///
/// State lives in a plain, hand-readable JSON file next to the
/// configuration, written atomically and eagerly (on document changes and
/// window closes, not just at quit — a crash loses at most a moment of
/// position, never the file list). Deleting the file is a complete reset;
/// launching with `--fresh` (or holding ⇧) ignores it once.
struct SessionState: Codable {
    struct Window: Codable {
        let path: String
        /// Caret position in UTF-16 units.
        let caret: Int
        /// Vertical scroll offset in points.
        let scroll: Double
    }

    var version = 1
    var windows: [Window] = []
    /// Path of the frontmost document, if any.
    var frontmost: String?
    /// The sidebar's buffer-list/file-tree divider, as a fraction of the
    /// sidebar height. Shared by every window, so it is saved once.
    var sidebarSplit: Double?
}

enum SessionStore {
    static var path: String {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Textchum/session.json").path
    }

    static func load() -> SessionState? {
        guard let data = FileManager.default.contents(atPath: path) else { return nil }
        // A file that fails to parse is ignored, never overwritten with
        // garbage: the next save replaces it wholesale anyway.
        return try? JSONDecoder().decode(SessionState.self, from: data)
    }

    static func save(_ state: SessionState) {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? encoder.encode(state) else { return }
        let url = URL(fileURLWithPath: path)
        try? FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? data.write(to: url, options: .atomic)
    }
}
