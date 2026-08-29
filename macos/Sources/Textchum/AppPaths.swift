import Foundation

/// Where Textchum keeps what it owns, and how a run can be told to keep
/// it somewhere else.
///
/// Ordinarily that is `~/Library/Application Support/Textchum` with the
/// server log under `~/Library/Logs/Textchum`. `--data-dir <path>` puts
/// the lot under one directory instead — configuration, themes, icon
/// packs, the session and the log — so a run can be given a profile
/// built for the occasion and thrown away afterwards, without the real
/// one ever being opened.
///
/// `--config <path>` still points at one file, and the session follows
/// it; `--data-dir` is the broader answer, and `--config` wins over it
/// for the configuration alone.
enum AppPaths {
    /// The directory `--data-dir` named, if a run named one. Read once:
    /// the arguments do not change, and a path that moved mid-run would
    /// leave half the profile behind.
    static let dataDirectory: URL? = {
        guard let url = dataDirectory(from: CommandLine.arguments) else { return nil }
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }()

    /// The directory `--data-dir` names in `arguments`, if any. Kept
    /// apart from the stored answer so it can be asked about.
    static func dataDirectory(from arguments: [String]) -> URL? {
        guard let flag = arguments.firstIndex(of: "--data-dir"),
            arguments.count > flag + 1
        else { return nil }
        let path = (arguments[flag + 1] as NSString).expandingTildeInPath
        guard !path.isEmpty else { return nil }
        return URL(fileURLWithPath: path)
    }

    /// The flags that take a value, so a file argument is never mistaken
    /// for one.
    static let valueFlags = ["--config", "--data-dir"]

    private static var applicationSupport: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Textchum", isDirectory: true)
    }

    /// The configuration file. `--config` names it outright.
    static var configPath: String {
        let arguments = CommandLine.arguments
        if let flag = arguments.firstIndex(of: "--config"), arguments.count > flag + 1 {
            return arguments[flag + 1]
        }
        return (dataDirectory ?? applicationSupport)
            .appendingPathComponent("config.json").path
    }

    /// User theme files, one JSON per theme.
    static var themesDirectory: URL {
        (dataDirectory ?? applicationSupport)
            .appendingPathComponent("themes", isDirectory: true)
    }

    /// Imported icon packs.
    static var iconsDirectory: URL {
        (dataDirectory ?? applicationSupport)
            .appendingPathComponent("icons", isDirectory: true)
    }

    /// The language-server debug trail.
    static var logFile: URL {
        if let dataDirectory {
            return dataDirectory.appendingPathComponent("lsp.log")
        }
        let library = FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask)[0]
        return library.appendingPathComponent("Logs/Textchum/lsp.log")
    }

    /// What to tell someone looking for the log.
    static var logFileForDisplay: String {
        dataDirectory == nil ? "~/Library/Logs/Textchum/lsp.log" : logFile.path
    }
}
