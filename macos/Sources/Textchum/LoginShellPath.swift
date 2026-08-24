import Foundation

/// Rebuilds PATH the way a terminal would see it. Finder launches apps
/// with `/usr/bin:/bin:/usr/sbin:/sbin`, so anything installed through
/// Homebrew, npm, cargo, go, or pip is invisible to a plain `Process` —
/// including every language server. Asking the user's login shell for
/// its PATH (with a short timeout, in case a shell profile hangs) and
/// appending a few conventional tool directories fixes that for the
/// whole process, language servers and ctags alike.
func adoptLoginShellPath() {
    var components = (ProcessInfo.processInfo.environment["PATH"] ?? "")
        .split(separator: ":")
        .map(String.init)
    var seen = Set(components)
    func add(_ directory: String) {
        guard !seen.contains(directory),
            FileManager.default.fileExists(atPath: directory)
        else { return }
        seen.insert(directory)
        components.append(directory)
    }

    for directory in loginShellPath()?.split(separator: ":").map(String.init) ?? [] {
        add(directory)
    }
    let home = NSHomeDirectory()
    for directory in [
        "/opt/homebrew/bin", "/usr/local/bin",
        "\(home)/.cargo/bin", "\(home)/go/bin", "\(home)/.local/bin",
    ] {
        add(directory)
    }
    setenv("PATH", components.joined(separator: ":"), 1)
}

/// The login shell's PATH, or nil if the shell cannot answer quickly.
private func loginShellPath() -> String? {
    let shell = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
    let process = Process()
    process.executableURL = URL(fileURLWithPath: shell)
    // A login+interactive shell reads the same profiles a terminal does;
    // the marker separates the PATH from any profile chatter.
    process.arguments = ["-ilc", "printf '::PATH::%s' \"$PATH\""]
    let out = Pipe()
    process.standardOutput = out
    process.standardError = Pipe()
    do { try process.run() } catch { return nil }

    let deadline = Date().addingTimeInterval(3)
    while process.isRunning, Date() < deadline {
        usleep(50_000)
    }
    if process.isRunning {
        process.terminate()
        return nil
    }
    let output = String(
        decoding: out.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
    guard let range = output.range(of: "::PATH::") else { return nil }
    let path = String(output[range.upperBound...])
        .trimmingCharacters(in: .whitespacesAndNewlines)
    return path.isEmpty ? nil : path
}
