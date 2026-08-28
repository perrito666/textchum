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
    /// Where the session lives. It belongs to the same profile as the
    /// configuration, so a run pointed at a scratch config (tests,
    /// screenshots, `--config`) keeps its own session and can never
    /// overwrite the one the real app owns.
    static let defaultDirectory: URL =
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("Textchum", isDirectory: true)

    static var directory: URL = defaultDirectory

    /// Whether a profile was named explicitly, by `--config`. Such a
    /// profile is its own isolation and needs no further guard.
    private(set) static var hasExplicitProfile = false

    /// Points the store at the configuration's own directory.
    static func useProfile(ofConfigAt configPath: String) {
        directory = URL(fileURLWithPath: configPath).deletingLastPathComponent()
        hasExplicitProfile = true
    }

    /// Puts the store back where it started. The app names a profile
    /// once at launch; this is for the smoke test, which borrows the
    /// store and has to give it back as it found it.
    static func useDefaultProfile() {
        directory = defaultDirectory
        hasExplicitProfile = false
    }

    /// Whether this is the installed application rather than a build
    /// run from the checkout. An installed app runs from inside a
    /// `.app`; `swift build` leaves a bare executable.
    static var isInstalledApplication: Bool = {
        Bundle.main.bundleURL.pathExtension == "app"
    }()

    /// The session file.
    ///
    /// A build run from the checkout keeps its own, beside the real
    /// one. It has no business in the file the installed app owns: it
    /// is a test run, a `make run`, a screenshot session or a
    /// measurement, and opening one scratch file with it used to be
    /// enough to replace a day's worth of open documents. Scoping the
    /// session to the configuration's directory only protects runs that
    /// pass `--config`; a build launched with a file and nothing else
    /// looks exactly like the real thing.
    ///
    /// A run that *was* given a profile keeps the plain name: that
    /// directory is already the isolation, and making the file name
    /// depend on how the binary was built would be a surprise for no
    /// gain.
    static var path: String {
        let separate = !isInstalledApplication && !hasExplicitProfile
        let name = separate ? "session-development.json" : "session.json"
        return directory.appendingPathComponent(name).path
    }

    static func load() -> SessionState? {
        guard let data = FileManager.default.contents(atPath: path) else { return nil }
        // A file that fails to parse is ignored, never overwritten with
        // garbage: the next save replaces it wholesale anyway.
        return try? JSONDecoder().decode(SessionState.self, from: data)
    }

    /// The previous contents of the session file, kept beside it.
    ///
    /// The list of open documents is written eagerly and often, and
    /// until now a single bad write ended it: there was no copy to go
    /// back to, and nothing else on the system remembers what was open.
    /// The configuration has kept a `.bak` since it was written; this
    /// is the same idea for the file that is easier to lose and harder
    /// to reconstruct.
    static var backupPath: String { path + ".bak" }

    static func save(_ state: SessionState) {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? encoder.encode(state) else { return }
        let url = URL(fileURLWithPath: path)
        try? FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        // Keep what is there before replacing it, but only when it says
        // something: backing up an empty list over a good backup would
        // lose the very thing worth keeping.
        if let previous = SessionStore.load(), !previous.windows.isEmpty,
            let previousData = try? encoder.encode(previous)
        {
            try? previousData.write(to: URL(fileURLWithPath: backupPath), options: .atomic)
        }
        try? data.write(to: url, options: .atomic)
    }
}
