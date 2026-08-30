import AppKit
import Foundation
import TextchumKit

/// What files remember about themselves, kept per project.
///
/// How a file is split, where each view was looking, what is folded and
/// what language it was told it is: data about the file, not settings
/// and not session. The record lives in the profile by default, one per
/// project root, or with the checkout as `<root>/.tchum`.
///
/// The core owns the format and the sweep; this is where the editor
/// says which project a file belongs to and when to read or write.
@MainActor
enum ProjectState {
    /// Where records are kept when they are not with the checkout.
    /// Beside the session, so a scratch profile keeps its own.
    static var directory: URL {
        if let configured = (NSApp.delegate as? AppDelegate)?.config?.projectStateDirectory,
            !configured.isEmpty
        {
            return URL(fileURLWithPath: (configured as NSString).expandingTildeInPath)
        }
        return SessionStore.directory.appendingPathComponent("projects", isDirectory: true)
    }

    static var inProject: Bool {
        (NSApp.delegate as? AppDelegate)?.config?.projectStateInProject ?? false
    }

    /// What `path` remembers, or nil when the project has nothing to
    /// say about it — or when the file belongs to no project.
    static func state(forPath path: String, projectRoot: String?) -> CoreProjectState.FileState? {
        guard let root = projectRoot else { return nil }
        return CoreProjectState.fileState(
            root: root, directory: directory.path, inProject: inProject, path: path)
    }

    /// Writes down what a file remembers.
    @discardableResult
    static func record(
        _ state: CoreProjectState.FileState, forPath path: String, projectRoot: String?
    ) -> Bool {
        guard let root = projectRoot else { return false }
        return CoreProjectState.setFileState(
            state, root: root, directory: directory.path, inProject: inProject, path: path)
    }

    /// Forgets records for roots that are gone and those past their
    /// keep window, off the main thread: a machine that has seen a
    /// thousand checkouts should not keep a thousand records, and the
    /// editor should not wait while they are counted.
    static func sweepAtLaunch() {
        guard (NSApp.delegate as? AppDelegate)?.config?.projectStateSweep ?? true else { return }
        let days = UInt64((NSApp.delegate as? AppDelegate)?.config?.projectStateKeepDays ?? 90)
        let dir = directory.path
        DispatchQueue.global(qos: .utility).async {
            let gone = CoreProjectState.sweep(directory: dir, keepDays: days)
            if gone > 0 {
                NSLog("project records: forgot \(gone)")
            }
        }
    }
}
