import Foundation

/// Runs the configured save-preprocessor chain: each command receives
/// the document on stdin and must write the whole document to stdout
/// (the way `black -`, `gofmt`, or `prettier` work), and the chain
/// pipes one command's output into the next. Any failure aborts the
/// chain and reports which link broke — the buffer is never replaced
/// with a partial or empty result.
enum Preprocessors {
    struct Failure: Error {
        let command: String
        let details: String
    }

    /// How long one command may take before it counts as hung. Real
    /// formatters answer in tenths of a second; the cap only exists so
    /// a broken configuration cannot wedge a save forever.
    static let timeout: TimeInterval = 10

    /// Runs `commands` over `text` in order, in `directory` (the
    /// project root, so tools pick up their own config files).
    /// Blocking — call it off the main thread or accept the stall.
    static func run(
        commands: [String], on text: String, in directory: String?
    ) -> Result<String, Failure> {
        var current = text
        for command in commands {
            let words = split(commandLine: command)
            guard !words.isEmpty else { continue }
            switch pipe(current, through: words, in: directory) {
            case .success(let output):
                // A formatter that answers nothing to a non-empty
                // document did not format — it failed quietly (or
                // writes to files, which this contract does not do).
                if output.isEmpty, !current.isEmpty {
                    return .failure(
                        Failure(
                            command: command,
                            details: "the command produced no output; save preprocessors "
                                + "must write the document to standard output"))
                }
                current = output
            case .failure(let failure):
                return .failure(failure)
            }
        }
        return .success(current)
    }

    private static func pipe(
        _ input: String, through words: [String], in directory: String?
    ) -> Result<String, Failure> {
        let commandLine = words.joined(separator: " ")
        let process = Process()
        // `env` resolves the tool against the login-shell PATH the app
        // adopted at startup, same as language servers.
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = words
        if let directory, FileManager.default.fileExists(atPath: directory) {
            process.currentDirectoryURL = URL(fileURLWithPath: directory)
        }
        let stdin = Pipe()
        let stdout = Pipe()
        let stderr = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        process.standardError = stderr
        do {
            try process.run()
        } catch {
            return .failure(Failure(command: commandLine, details: "\(error)"))
        }

        // Feed and read on background queues: a document larger than
        // the pipe buffer deadlocks if written and read from one thread.
        DispatchQueue.global().async {
            stdin.fileHandleForWriting.write(Data(input.utf8))
            try? stdin.fileHandleForWriting.close()
        }
        var outputData = Data()
        var errorData = Data()
        let readers = DispatchGroup()
        DispatchQueue.global().async(group: readers) {
            outputData = stdout.fileHandleForReading.readDataToEndOfFile()
        }
        DispatchQueue.global().async(group: readers) {
            errorData = stderr.fileHandleForReading.readDataToEndOfFile()
        }

        let deadline = Date().addingTimeInterval(timeout)
        while process.isRunning, Date() < deadline {
            usleep(20_000)
        }
        if process.isRunning {
            process.terminate()
            return .failure(
                Failure(
                    command: commandLine,
                    details: "timed out after \(Int(timeout))s"))
        }
        readers.wait()

        guard process.terminationStatus == 0 else {
            let stderrText = String(decoding: errorData.suffix(2000), as: UTF8.self)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            return .failure(
                Failure(
                    command: commandLine,
                    details: stderrText.isEmpty
                        ? "exited with status \(process.terminationStatus)"
                        : stderrText))
        }
        return .success(String(decoding: outputData, as: UTF8.self))
    }

    /// Splits a command line into words, honoring single and double
    /// quotes — enough shell for formatter invocations, without a shell.
    static func split(commandLine: String) -> [String] {
        var words: [String] = []
        var current = ""
        var quote: Character?
        var hasContent = false
        for character in commandLine {
            if let active = quote {
                if character == active {
                    quote = nil
                } else {
                    current.append(character)
                }
            } else if character == "\"" || character == "'" {
                quote = character
                hasContent = true
            } else if character == " " || character == "\t" {
                if hasContent { words.append(current) }
                current = ""
                hasContent = false
            } else {
                current.append(character)
                hasContent = true
            }
        }
        if hasContent { words.append(current) }
        return words
    }
}
