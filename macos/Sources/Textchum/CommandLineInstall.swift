import AppKit
import TextchumKit

extension AppDelegate {
    /// Textchum → Install chum Command…: puts `chum` on the PATH at
    /// `/usr/local/bin/chum`. A plain copy is tried first; when the
    /// directory needs more rights than we have, macOS asks for them
    /// with the standard administrator prompt.
    @objc func installCommandLineTool(_ sender: Any?) {
        guard let source = Self.chumScript() else {
            let alert = NSAlert()
            alert.alertStyle = .warning
            alert.messageText = t("Could not find the chum script")
            alert.informativeText =
                t(
                    "The app bundle is missing its chum resource; rebuild with "
                        + "`make app`, or install from a checkout with `make install-cli`.")
            alert.runModal()
            return
        }
        let target = "/usr/local/bin/chum"
        var failure: String?
        do {
            try FileManager.default.createDirectory(
                atPath: "/usr/local/bin", withIntermediateDirectories: true)
            if FileManager.default.fileExists(atPath: target) {
                try FileManager.default.removeItem(atPath: target)
            }
            try FileManager.default.copyItem(atPath: source, toPath: target)
            try FileManager.default.setAttributes(
                [.posixPermissions: 0o755], ofItemAtPath: target)
        } catch {
            failure = escalatedInstall(source: source)
        }
        let alert = NSAlert()
        if let failure {
            alert.alertStyle = .warning
            alert.messageText = t("chum was not installed")
            alert.informativeText = failure
        } else {
            alert.alertStyle = .informational
            alert.messageText = t("chum installed")
            alert.informativeText =
                "\(target) is ready. From a terminal:\n\n"
                + "chum notes.md — open a file\n"
                + "chum +120 main.rs — open at a line\n"
                + "chum -w draft.md — open in its own window"
        }
        alert.runModal()
    }

    /// The install rerun with administrator rights, via the standard
    /// system prompt. Returns a failure message, or nil on success.
    private func escalatedInstall(source: String) -> String? {
        let command =
            "install -d /usr/local/bin && install -m 0755 '\(source)' /usr/local/bin/chum"
        let escaped = command.replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        let script = NSAppleScript(
            source: "do shell script \"\(escaped)\" with administrator privileges")
        var error: NSDictionary?
        script?.executeAndReturnError(&error)
        guard let error else { return nil }
        let message = error[NSAppleScript.errorMessage] as? String ?? "unknown error"
        // A canceled password prompt is a decision, not a failure worth
        // alarming about — but the alert still says nothing was installed.
        return message
    }

    /// The bundled script — or, running from a checkout, the one in the
    /// repository (the build products live a few directories below it).
    private static func chumScript() -> String? {
        if let bundled = Bundle.main.resourceURL?.appendingPathComponent("chum").path,
            FileManager.default.fileExists(atPath: bundled)
        {
            return bundled
        }
        var directory = Bundle.main.bundleURL.deletingLastPathComponent()
        for _ in 0..<6 {
            let candidate = directory.appendingPathComponent("scripts/chum")
            if FileManager.default.fileExists(atPath: candidate.path) {
                return candidate.path
            }
            directory.deleteLastPathComponent()
        }
        return nil
    }

    /// Textchum → Open Themes Folder: where user theme files live —
    /// dropping a JSON there (see `--emit-theme`) adds it to the picker.
    @objc func openThemesFolder(_ sender: Any?) {
        let directory = ThemeFiles.directory
        try? FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true)
        NSWorkspace.shared.open(directory)
    }
}
