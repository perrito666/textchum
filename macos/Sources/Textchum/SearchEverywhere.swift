import AppKit
import TextchumKit

/// Search Everywhere (⇧⌘A): files, symbols, commands and settings in
/// one box, each in a group of its own, the group with the best match
/// first. The order is the core's; this gathers the candidates — the
/// project's files, the language server's symbols, the menus, the
/// core's switches — and shows them in a list with the selected
/// place's code beside it.
@MainActor
final class SearchEverywhere {
    private let panel = ListPanel()

    /// What a row stands for, in the order the list numbers its items.
    private enum Target {
        case file(String)
        case symbol(CoreSymbol)
        case command(CommandPalettePanel.Command)
        case setting(CoreConfig.Switch)
    }

    private var root = ""
    /// The project's files, relative to the root, once walked.
    private var files: [String] = []
    private var recentFiles: [String] = []
    private var commands: [CommandPalettePanel.Command] = []
    private var recentCommands: [String] = []
    private var symbols: [CoreSymbol] = []
    private var query = ""
    private var targets: [Target] = []
    /// Counts the questions put to the language server, so an answer to
    /// a query since changed is dropped.
    private var asked = 0

    /// Asks the language server for the symbols matching a query;
    /// delivers nil when there is no server to ask.
    var symbolSearch: ((String, @escaping ([CoreSymbol]?) -> Void) -> Void)?
    /// Opens a file at a zero-based line.
    var onOpen: ((String, Int) -> Void)?
    var config: CoreConfig?
    /// Called after a switch was flipped and saved.
    var onSettingChanged: (() -> Void)?
    private var projectRoot: String?

    /// The rows on show, for the smoke test.
    var shownTexts: [String] { panel.debugShownTexts }

    func show(root: String, projectRoot: String?, over window: NSWindow?) {
        self.root = root
        self.projectRoot = projectRoot
        query = ""
        symbols = []
        files = []
        commands = CommandPalettePanel.harvest()
        recentCommands = ProjectState.recent("commands", projectRoot: projectRoot)
        recentFiles = ProjectState.recent("files", projectRoot: projectRoot).compactMap { path in
            path.hasPrefix(root + "/") ? String(path.dropFirst(root.count + 1)) : nil
        }
        panel.show(
            rows: rows(), over: window, title: t("Search Everywhere"),
            placeholder: t("file, symbol, command or setting…"), monospaced: false,
            place: { [weak self] item in self?.place(of: item) },
            search: { [weak self] typed in self?.search(typed) }
        ) { [weak self] item in
            self?.choose(item)
        }
        // The walk is the slow part; the other groups are there at once.
        let walked = root
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let paths = CoreSearch.listFiles(root: walked)
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    guard let self, self.root == walked else { return }
                    self.files = paths
                    self.panel.replace(rows: self.rows())
                }
            }
        }
    }

    /// For the smoke test: what typing `typed` shows, once the files
    /// are walked.
    func debugSearch(_ typed: String, files: [String]) {
        self.files = files
        search(typed)
    }

    private func search(_ typed: String) {
        query = typed
        symbols = []
        panel.replace(rows: rows())
        guard !typed.trimmingCharacters(in: .whitespaces).isEmpty, let symbolSearch else { return }
        asked += 1
        let question = asked
        symbolSearch(typed) { [weak self] found in
            guard let self, self.asked == question, let found else { return }
            self.symbols = found
            self.panel.replace(rows: self.rows())
        }
    }

    /// The groups as the core orders them, each under its heading.
    private func rows() -> [ListPanel.Row] {
        let typed = query.trimmingCharacters(in: .whitespaces)
        let fileLabels =
            typed.isEmpty
            ? recentFiles : CoreSearch.matchFiles(paths: files, query: typed, limit: 60)
        let commandOrder =
            typed.isEmpty
            ? CoreMemory.recentFirst(commands.map(\.title), recent: recentCommands).map { commands[$0] }
            : commands
        let switches = config?.switches ?? []
        let groups = CoreEverywhere.arrange(
            query: typed,
            groups: [
                ("files", fileLabels),
                ("symbols", symbols.map(\.name)),
                ("commands", commandOrder.map(\.title)),
                ("settings", switches.map(\.label)),
            ])
        var rows: [ListPanel.Row] = []
        targets = []
        for group in groups {
            rows.append(.heading(group.title))
            for index in group.rows {
                switch group.kind {
                case "files":
                    rows.append(.item(fileLabels[index]))
                    targets.append(.file(root + "/" + fileLabels[index]))
                case "symbols":
                    let symbol = symbols[index]
                    let relative =
                        symbol.path.hasPrefix(root + "/")
                        ? String(symbol.path.dropFirst(root.count + 1)) : symbol.path
                    rows.append(.item("\(symbol.name)   \(symbol.kind) — \(relative):\(symbol.line + 1)"))
                    targets.append(.symbol(symbol))
                case "commands":
                    let command = commandOrder[index]
                    rows.append(.item(command.shortcut.isEmpty ? command.title : "\(command.title)   \(command.shortcut)"))
                    targets.append(.command(command))
                default:
                    let setting = switches[index]
                    rows.append(.item("\(setting.label)   \(setting.on ? t("on") : t("off"))"))
                    targets.append(.setting(setting))
                }
            }
        }
        return rows
    }

    private func place(of item: Int) -> (path: String, line: Int)? {
        guard targets.indices.contains(item) else { return nil }
        switch targets[item] {
        case .file(let path): return (path, 0)
        case .symbol(let symbol): return (symbol.path, symbol.line)
        case .command, .setting: return nil
        }
    }

    private func choose(_ item: Int) {
        guard targets.indices.contains(item) else { return }
        switch targets[item] {
        case .file(let path):
            onOpen?(path, 0)
        case .symbol(let symbol):
            onOpen?(symbol.path, symbol.line)
        case .command(let command):
            CommandPalettePanel.run(command, projectRoot: projectRoot)
        case .setting(let setting):
            // A switch flips where it is listed; the change is saved and
            // put into effect the way a Settings change is.
            config?.setSwitch(setting.id, on: !setting.on)
            onSettingChanged?()
        }
    }
}
