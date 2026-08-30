import AppKit
import TextchumKit

/// Symbol definitions from Universal Ctags, the fallback for projects
/// whose language has no server running. Off by default; the Projects
/// settings opt a project (or everything) in. One index per project
/// root, cached briefly so repeated jumps stay instant and edits are
/// picked up soon after.
@MainActor
final class CtagsIndex {
    static let shared = CtagsIndex()

    private struct Index {
        let built: Date
        /// Symbol name → definition sites, in ctags output order.
        let symbols: [String: [(path: String, line: Int)]]
    }

    private var indexes: [String: Index] = [:]
    /// The resolved Universal Ctags executable; missing after a search is
    /// remembered (and alerted once), like a missing language server.
    private var binary: String??
    private var warnedMissingBinary = false

    /// The first definition site of `name` inside `root`'s project, with
    /// an absolute path and zero-based line — or nil when the symbol is
    /// unknown or ctags is unavailable.
    func definition(of name: String, in root: String) -> (path: String, line: Int)? {
        guard let found = index(for: root)?.symbols[name]?.first else { return nil }
        return (
            (root as NSString).appendingPathComponent(found.path),
            max(0, found.line - 1)
        )
    }

    private func index(for root: String) -> Index? {
        if let cached = indexes[root], Date().timeIntervalSince(cached.built) < 30 {
            return cached
        }
        guard let executable = resolveBinary() else {
            warnOnce()
            return indexes[root]
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = [
            executable, "-R", "--output-format=json", "--fields=+n", "-f", "-", ".",
        ]
        process.currentDirectoryURL = URL(fileURLWithPath: root)
        let out = Pipe()
        process.standardOutput = out
        process.standardError = Pipe()
        do { try process.run() } catch { return indexes[root] }
        // Reading concurrently with the run (rather than waiting first)
        // keeps large outputs from deadlocking on a full pipe.
        let data = out.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { return indexes[root] }

        var symbols: [String: [(path: String, line: Int)]] = [:]
        for line in data.split(separator: UInt8(ascii: "\n")) {
            guard
                let tag = try? JSONSerialization.jsonObject(with: Data(line))
                    as? [String: Any],
                tag["_type"] as? String == "tag",
                let name = tag["name"] as? String,
                let path = tag["path"] as? String,
                let lineNumber = tag["line"] as? Int
            else { continue }
            symbols[name, default: []].append((path, lineNumber))
        }
        let index = Index(built: Date(), symbols: symbols)
        indexes[root] = index
        NSLog("ctags indexed \(symbols.count) symbols under \(root)")
        return index
    }

    /// The first candidate that really is Universal Ctags — its JSON
    /// output is what this index reads, and the BSD ctags macOS ships
    /// under the same name cannot produce it. Both PATH names and the
    /// usual install locations are probed, so a `/usr/bin/ctags` earlier
    /// on PATH does not hide a Homebrew one further along.
    private func resolveBinary() -> String? {
        if let binary { return binary }
        let candidates = [
            "ctags", "uctags",
            "/opt/homebrew/bin/ctags", "/usr/local/bin/ctags",
            "/opt/homebrew/bin/uctags", "/usr/local/bin/uctags",
        ]
        let found = candidates.first { candidate in
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
            process.arguments = [candidate, "--version"]
            let out = Pipe()
            process.standardOutput = out
            process.standardError = Pipe()
            guard (try? process.run()) != nil else { return false }
            let data = out.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            return String(decoding: data, as: UTF8.self).contains("Universal Ctags")
        }
        if let found {
            NSLog("ctags: using \(found)")
        }
        binary = .some(found)
        return found
    }

    private func warnOnce() {
        guard !warnedMissingBinary else { return }
        warnedMissingBinary = true
        let alert = NSAlert()
        alert.alertStyle = .informational
        alert.messageText = t("Universal Ctags is not installed")
        alert.informativeText =
            t("The ctags fallback needs Universal Ctags (install with: brew install universal-ctags).")
        alert.runModal()
    }
}
