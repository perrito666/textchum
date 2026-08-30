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

    /// One window: the files it held as tabs, and its columns.
    struct Layout: Codable {
        var tabs: [String] = []
        /// One entry per pane, from before windows held columns.
        var panes: [String] = []
        var columns: [ColumnLayout]?
    }

    /// One column: the file it showed, how many views of it were
    /// stacked, and where the dividers between them sat.
    struct ColumnLayout: Codable {
        var file: String
        var views: Int = 1
        var dividers: [Double] = []
    }

    var version = 1
    var windows: [Window] = []
    /// The windows, their tabs and their panes. Absent in sessions
    /// written before windows were recorded, which come back as one
    /// window holding everything.
    var layout: [Layout]?
    /// Path of the frontmost document, if any.
    var frontmost: String?
    /// The sidebar's buffer-list/file-tree divider, as a fraction of the
    /// sidebar height. Shared by every window, so it is saved once.
    var sidebarSplit: Double?
}

enum SessionStore {
    /// Where the session lives. It belongs to the same profile as the
    /// configuration, so a run pointed at a scratch config (tests,
    /// screenshots, `--config`) keeps its own session and can never
    /// overwrite the one the real app owns.
    static var directory: URL =
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("Textchum", isDirectory: true)

    /// Points the store at the configuration's own directory.
    static func useProfile(ofConfigAt configPath: String) {
        directory = URL(fileURLWithPath: configPath).deletingLastPathComponent()
    }

    static var path: String {
        directory.appendingPathComponent("session.json").path
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
