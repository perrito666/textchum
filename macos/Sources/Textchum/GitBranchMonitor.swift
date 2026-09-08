import Foundation
import TextchumKit

extension Notification.Name {
    /// A watched repository's branch changed; `object` is its root.
    static let textchumBranchChanged = Notification.Name("textchumBranchChanged")
}

/// The branch checked out in each repository the editor shows, learned
/// once per repository and again when it changes — not on a timer: the
/// gitdir's HEAD file is rewritten on every checkout, and watching it
/// says when to ask git again. Git replaces the file rather than
/// writing into it, so each event closes the descriptor and opens the
/// path anew.
@MainActor
final class GitBranchMonitor {
    static let shared = GitBranchMonitor()

    private struct Watched {
        var branch: String?
        var headPath: String
        var source: DispatchSourceFileSystemObject?
    }

    private var watched: [String: Watched] = [:]

    /// The branch of the repository at `root`, or nil when there is
    /// none or HEAD is detached. The first ask starts the watch.
    func branch(forRoot root: String) -> String? {
        if let known = watched[root] { return known.branch }
        guard let info = CoreChanges.repositoryInfo(near: root) else {
            watched[root] = Watched(branch: nil, headPath: "", source: nil)
            return nil
        }
        watched[root] = Watched(branch: info.branch, headPath: info.headPath, source: nil)
        arm(root: root)
        return info.branch
    }

    /// Debug and tests: forgets a repository, so the next ask reads it
    /// fresh.
    func forget(root: String) {
        watched[root]?.source?.cancel()
        watched[root] = nil
    }

    private func arm(root: String) {
        guard var entry = watched[root], !entry.headPath.isEmpty else { return }
        entry.source?.cancel()
        let descriptor = open(entry.headPath, O_EVTONLY)
        guard descriptor >= 0 else {
            watched[root] = entry
            return
        }
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .delete, .rename, .extend, .attrib],
            queue: .main)
        source.setEventHandler { [weak self] in
            MainActor.assumeIsolated { self?.headChanged(root: root) }
        }
        source.setCancelHandler { close(descriptor) }
        source.resume()
        entry.source = source
        watched[root] = entry
    }

    private func headChanged(root: String) {
        let before = watched[root]?.branch
        let info = CoreChanges.repositoryInfo(near: root)
        watched[root]?.branch = info?.branch
        if let head = info?.headPath, !head.isEmpty { watched[root]?.headPath = head }
        // The file was replaced under the descriptor; watch the new one.
        arm(root: root)
        if before != info?.branch {
            NotificationCenter.default.post(name: .textchumBranchChanged, object: root)
        }
    }
}
