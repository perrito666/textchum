import AppKit
import SwiftUI
import TextchumKit

/// Clipboard helpers for the shapes a file's location takes: bare name,
/// project-relative, absolute, or a URL on the repository's forge
/// (GitHub, GitLab, Forgejo and friends).
enum PathActions {
    static func copy(_ string: String) {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(string, forType: .string)
    }

    /// The path relative to the project root when the file is inside it,
    /// otherwise the absolute path with the home directory abbreviated.
    static func relativePath(_ path: String, projectRoot: String?) -> String {
        if let projectRoot, path.hasPrefix(projectRoot + "/") {
            return String(path.dropFirst(projectRoot.count + 1))
        }
        return (path as NSString).abbreviatingWithTildeInPath
    }

    /// True when the path sits inside some git repository — a cheap
    /// ancestor walk, good enough to decide whether to offer a forge URL.
    static func isInGitRepository(_ path: String) -> Bool {
        var url = URL(fileURLWithPath: path)
        while url.path != "/" {
            if FileManager.default.fileExists(
                atPath: url.appendingPathComponent(".git").path)
            {
                return true
            }
            url.deleteLastPathComponent()
        }
        return false
    }

    /// The file's page on the repository's forge: host and repository
    /// from the `origin` remote (or the first remote there is), current
    /// branch, and the file's path inside the repository. The URL shape
    /// follows the host — GitHub's `blob`/`tree`, GitLab's `-/blob`, and
    /// the `src/branch` layout Forgejo and Gitea share.
    static func forgeURL(forPath path: String, isDirectory: Bool, line: Int? = nil) -> String? {
        guard let url = forgeURL(forPath: path, isDirectory: isDirectory) else { return nil }
        guard let line, !isDirectory else { return url }
        // Each forge spells a line its own way.
        if url.hasPrefix("https://github.com/") || url.contains("gitlab") {
            return "\(url)#L\(line)"
        }
        return "\(url)#lines-\(line)"
    }

    static func forgeURL(forPath path: String, isDirectory: Bool) -> String? {
        let dir = isDirectory ? path : (path as NSString).deletingLastPathComponent
        guard let top = git(in: dir, "rev-parse", "--show-toplevel"),
            var branch = git(in: dir, "rev-parse", "--abbrev-ref", "HEAD"),
            let remote = remoteURL(in: dir),
            let (host, repo) = parse(remote: remote)
        else { return nil }
        let detached = branch == "HEAD"
        if detached {
            branch = git(in: dir, "rev-parse", "--short", "HEAD") ?? branch
        }
        let relative = String(path.dropFirst(top.count))
            .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let base = "https://\(host)/\(repo)"
        guard !relative.isEmpty else { return base }
        let location = "\(escape(branch))/" + relative.split(separator: "/")
            .map { escape(String($0)) }
            .joined(separator: "/")
        if host == "github.com" {
            return "\(base)/\(isDirectory ? "tree" : "blob")/\(location)"
        }
        if host.contains("gitlab") {
            return "\(base)/-/\(isDirectory ? "tree" : "blob")/\(location)"
        }
        return "\(base)/src/\(detached ? "commit" : "branch")/\(location)"
    }

    private static func escape(_ component: String) -> String {
        component.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? component
    }

    private static func remoteURL(in dir: String) -> String? {
        if let origin = git(in: dir, "remote", "get-url", "origin") { return origin }
        guard let first = git(in: dir, "remote")?.split(separator: "\n").first
        else { return nil }
        return git(in: dir, "remote", "get-url", String(first))
    }

    /// Host plus "owner/repo" out of the remote spellings in the wild:
    /// scp-like `git@host:owner/repo.git`, `ssh://git@host/owner/repo`,
    /// and plain `https://host/owner/repo.git`.
    private static func parse(remote: String) -> (host: String, repo: String)? {
        var remote = remote
        if remote.hasSuffix(".git") { remote = String(remote.dropLast(4)) }
        if remote.contains("://"), let url = URL(string: remote), let host = url.host {
            let repo = url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            return repo.isEmpty ? nil : (host, repo)
        }
        guard let colon = remote.firstIndex(of: ":") else { return nil }
        var host = String(remote[..<colon])
        if let at = host.firstIndex(of: "@") {
            host = String(host[host.index(after: at)...])
        }
        let repo = String(remote[remote.index(after: colon)...])
            .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard !host.isEmpty, !repo.isEmpty else { return nil }
        return (host, repo)
    }

    /// One item of the path menu.
    struct MenuEntry {
        let title: String
        let run: () -> Void
    }

    /// The path menu's items, nil where a divider goes. One list, so
    /// the SwiftUI rows and the tree's AppKit rows offer the same menu.
    static func menuEntries(
        path: String, projectRoot: String?, isDirectory: Bool,
        onReveal: ((String) -> Void)? = nil
    ) -> [MenuEntry?] {
        var entries: [MenuEntry?] = []
        if let onReveal {
            entries.append(MenuEntry(title: t("Reveal in Tree")) { onReveal(path) })
        }
        entries.append(
            MenuEntry(title: t("Reveal in Finder")) {
                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
            })
        entries.append(nil)
        entries.append(
            MenuEntry(title: t("Copy File Name")) { copy((path as NSString).lastPathComponent) })
        entries.append(
            MenuEntry(title: t("Copy Relative Path")) {
                copy(relativePath(path, projectRoot: projectRoot))
            })
        entries.append(MenuEntry(title: t("Copy Absolute Path")) { copy(path) })
        if isInGitRepository(path) {
            // The path a commit, a review or a CI log names the file
            // by, which is not the project's when the project is a
            // crate or a package inside the repository.
            entries.append(
                MenuEntry(title: t("Copy Path from Git Root")) {
                    if let fromRoot = CoreChanges.pathFromRepositoryRoot(path) {
                        copy(fromRoot)
                    } else {
                        NSSound.beep()
                    }
                })
            entries.append(
                MenuEntry(title: t("Copy Forge URL")) {
                    if let url = forgeURL(forPath: path, isDirectory: isDirectory) {
                        copy(url)
                    } else {
                        NSSound.beep()
                    }
                })
        }
        return entries
    }

    private static func git(in directory: String, _ arguments: String...) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = ["-C", directory] + arguments
        let out = Pipe()
        process.standardOutput = out
        process.standardError = Pipe()
        do { try process.run() } catch { return nil }
        let data = out.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { return nil }
        let value = String(decoding: data, as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return value.isEmpty ? nil : value
    }
}

/// The copy items shared by buffer rows, tree rows, and the File menu.
struct PathCopyMenu: View {
    let path: String
    let projectRoot: String?
    let isDirectory: Bool
    /// Present where revealing makes sense (buffer rows); the tree's
    /// own rows leave it nil.
    var onReveal: ((String) -> Void)? = nil

    var body: some View {
        let entries = PathActions.menuEntries(
            path: path, projectRoot: projectRoot, isDirectory: isDirectory, onReveal: onReveal)
        ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
            if let entry {
                Button(entry.title, action: entry.run)
            } else {
                Divider()
            }
        }
    }
}
