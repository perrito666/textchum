import AppKit
import TextchumKit

/// Application lifecycle: the main menu, the core instance, configuration,
/// and the set of open editor windows.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private var coreApp: CoreApp?
    private var config: CoreConfig?
    private var settingsModel: SettingsModel?
    private var settingsWindowController: SettingsWindowController?
    /// Strong references to open editors; windows do not retain their
    /// controllers. Entries are removed as their windows close.
    private var editors: [EditorWindowController] = []

    /// `~/Library/Application Support/Textchum/config.json` — GUI-managed,
    /// hand-editable JSON. A hidden `--config <path>` points elsewhere,
    /// for tests that must not touch the real settings.
    private static var configPath: String {
        let arguments = CommandLine.arguments
        if let flag = arguments.firstIndex(of: "--config"), arguments.count > flag + 1 {
            return arguments[flag + 1]
        }
        return FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Textchum/config.json").path
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let mainMenu = makeMainMenu()
        NSApp.mainMenu = mainMenu
        registerMenuActions(in: mainMenu)

        let config = CoreConfig(path: Self.configPath)
        self.config = config
        applyKeyOverrides()
        let settingsModel = SettingsModel(config: config)
        settingsModel.onChange = { [weak self] in
            guard let self, let model = self.settingsModel else { return }
            self.applyAppearanceChoice()
            self.applyThemeChoice()
            self.coreApp?.lspConfigure(json: self.combinedLSPConfiguration)
            for editor in self.editors {
                editor.refreshProjectRoot()
            }
            for editor in self.editors {
                editor.apply(settings: model.currentSettings)
            }
        }
        self.settingsModel = settingsModel
        applyAppearanceChoice()
        applyThemeChoice()

        NotificationCenter.default.addObserver(
            forName: .textchumDocumentsChanged, object: nil, queue: .main
        ) { [weak self] notification in
            let changedEditor = notification.object as? EditorWindowController
            // Deferred a runloop turn: the notification can fire while
            // AppKit is mid-layout (e.g. from a window-title update), and
            // rebuilding the list reentrantly trips NSTableView.
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    self?.rebuildSidebar()
                    self?.saveSession()
                    // Save-as gives untitled documents a path; recents
                    // track it (the controller cannot reach this list).
                    if let path = changedEditor?.coreDocument.path {
                        self?.noteRecent(path: path)
                    }
                }
            }
        }

        // Language-server debug trail: every pool decision and status
        // transition, for when "why is there no server?" needs an answer.
        let logDirectory = FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask)
            .first?.appendingPathComponent("Logs/Textchum", isDirectory: true)
        if let logFile = logDirectory?.appendingPathComponent("lsp.log") {
            CoreWorkspace.setLSPLogPath(logFile.path)
        }

        // The core's event channel; ping once on launch so a broken
        // channel is caught immediately.
        let coreApp = CoreApp { [weak self] event in
            self?.handleCoreEvent(event)
        }
        coreApp.ping(sequence: 1)
        coreApp.lspConfigure(json: "{\"lsp\":\(config.lspJSON),\"workspace\":\(config.workspaceJSON)}")
        self.coreApp = coreApp

        settingsModel.onRestartServers = { [weak self] in
            self?.restartLanguageServers()
        }
        fileTreeState.onSplitCommitted = { [weak self] in
            self?.saveSession()
        }

        // Open files given on the command line — actual files only, not
        // directories, flags, or flag values. With none, defer the
        // decision one runloop turn: Finder-open events may still be in
        // flight, and session restore should not race them.
        let arguments = Array(CommandLine.arguments.dropFirst())
        var flagValueIndexes: Set<Int> = []
        if let flag = arguments.firstIndex(of: "--debug-panel") {
            flagValueIndexes = [flag + 1, flag + 2, flag + 3]
        }
        if let flag = arguments.firstIndex(of: "--config") {
            flagValueIndexes.insert(flag + 1)
        }
        let fileArguments = arguments.enumerated()
            .filter { index, argument in
                guard !argument.hasPrefix("--"), !flagValueIndexes.contains(index) else {
                    return false
                }
                var isDirectory: ObjCBool = false
                return FileManager.default.fileExists(atPath: argument, isDirectory: &isDirectory)
                    && !isDirectory.boolValue
            }
            .map(\.element)
        for path in fileArguments {
            open(path: path)
        }
        let skipRestore =
            !fileArguments.isEmpty
            || CommandLine.arguments.contains("--fresh")
            || NSEvent.modifierFlags.contains(.shift)
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.editors.isEmpty else { return }
                if !skipRestore {
                    self.restoreSession()
                }
                if self.editors.isEmpty {
                    self.newDocument(nil)
                }
            }
        }
        NSApp.activate(ignoringOtherApps: true)

        // Hidden debug hook for screenshot-driven UI verification:
        // --debug-panel files|grep <scope> <query>
        let allArguments = CommandLine.arguments
        if let flagIndex = allArguments.firstIndex(of: "--debug-panel"),
            allArguments.count > flagIndex + 3
        {
            let mode: QuickFinderPanel.Mode =
                allArguments[flagIndex + 1] == "grep" ? .grep : .files
            let scope = allArguments[flagIndex + 2]
            let query = allArguments[flagIndex + 3]
            let filters = Array(allArguments.dropFirst(flagIndex + 4))
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
                MainActor.assumeIsolated {
                    if allArguments[flagIndex + 1] == "hover" {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                            MainActor.assumeIsolated {
                                self?.editors.first?.debugShowHover()
                            }
                        }
                        return
                    }
                    if allArguments[flagIndex + 1] == "paths" {
                        self?.togglePathDisplay(nil)
                        return
                    }
                    if allArguments[flagIndex + 1] == "gather" {
                        // Merge every document into the first window's
                        // group — exercises the standalone-window path.
                        if let self, let first = self.editors.first {
                            self.mergeAsTabs(
                                documentIDs: self.editors.map(ObjectIdentifier.init),
                                into: ObjectIdentifier(first))
                        }
                        return
                    }
                    if allArguments[flagIndex + 1] == "outline" {
                        // Give the language server time to hand-shake.
                        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                            MainActor.assumeIsolated {
                                self?.editors.first?.showDocumentOutline(nil)
                            }
                        }
                        return
                    }
                    if allArguments[flagIndex + 1] == "palette" {
                        // scope doubles as the initial query.
                        self?.showCommandPalette(nil)
                        self?.commandPalette.debugSet(query: scope == "x" ? "" : scope)
                        return
                    }
                    if allArguments[flagIndex + 1] == "settings" {
                        // scope names the tab tag for this mode.
                        self?.settingsModel?.selectedTab = scope
                        self?.showSettings(nil)
                        return
                    }
                    if allArguments[flagIndex + 1] == "complete" {
                        // scope=line, query=character for this mode.
                        let line = Int(scope) ?? 0
                        let character = Int(query) ?? 0
                        if let editor = self?.editors.first {
                            editor.reveal(line: line, character: character)
                            editor.triggerCompletion(nil)
                        }
                        return
                    }
                    self?.showQuickFinder(mode: mode)
                    // "-" means "leave the scope the app chose", so the
                    // debug path exercises the real default.
                    self?.quickFinder.debugSet(
                        scope: scope == "-" ? (self?.currentScope ?? "") : scope,
                        query: query, filters: filters)
                }
            }
        }

        // A config file that exists but could not be used deserves exactly
        // one loud notice — the app is running on defaults meanwhile, and
        // the broken file is preserved for hand fixing.
        if let warning = config.loadWarning {
            let alert = NSAlert()
            alert.alertStyle = .warning
            alert.messageText = "Settings file could not be read"
            alert.informativeText = warning
            alert.runModal()
        }
    }

    /// Files opened from Finder (double-click, Open With, drag to icon)
    /// and `textchum://` URLs from the `chum` command.
    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls {
            if url.scheme == "textchum" {
                handleChumURL(url)
            } else if url.isFileURL {
                open(path: url.path)
            }
        }
    }

    /// `textchum://open?path=…[&line=N][&target=tab|window]` — the wire
    /// format behind the `chum` terminal command.
    private func handleChumURL(_ url: URL) {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return
        }
        var path: String?
        var line: Int?
        var target: CoreOpenTarget?
        for item in components.queryItems ?? [] {
            switch item.name {
            case "path": path = item.value
            case "line": line = item.value.flatMap(Int.init)
            case "target":
                target = item.value == "window" ? .window : item.value == "tab" ? .tab : nil
            default: break
            }
        }
        guard let path else { return }
        NSApp.activate(ignoringOtherApps: true)
        recordJumpOrigin()
        open(path: path, target: target, revealLine: line)
        // `chum --wait` blocks on this sentinel file; deleting it when
        // the document's window closes is what lets tools like
        // GIT_EDITOR read the edited file at the right moment.
        if let sentinel = components.queryItems?.first(where: { $0.name == "wait" })?.value {
            let absolute = URL(fileURLWithPath: path).standardizedFileURL.path
            if let editor = editors.first(where: { $0.coreDocument.path == absolute }) {
                chumWaitSentinels[ObjectIdentifier(editor)] = sentinel
            } else {
                // Nothing opened; never leave the caller hanging.
                try? FileManager.default.removeItem(atPath: sentinel)
            }
        }
    }

    /// Sentinel files whose deletion unblocks a waiting `chum --wait`,
    /// keyed by the editor whose window close releases them.
    private var chumWaitSentinels: [ObjectIdentifier: String] = [:]

    private func releaseChumWait(for editor: EditorWindowController) {
        if let sentinel = chumWaitSentinels.removeValue(forKey: ObjectIdentifier(editor)) {
            try? FileManager.default.removeItem(atPath: sentinel)
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        // Quitting releases every waiter — a hung git is worse than an
        // aborted commit.
        for sentinel in chumWaitSentinels.values {
            try? FileManager.default.removeItem(atPath: sentinel)
        }
        chumWaitSentinels.removeAll()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        switch menuItem.action {
        case #selector(goBack(_:)):
            return jumpStack.canGoBack
        case #selector(goForward(_:)):
            return jumpStack.canGoForward
        default:
            return true
        }
    }

    // MARK: Session

    /// Writes the current session: open files with their positions.
    /// Called eagerly on document changes and window closes, and at quit
    /// (which captures the freshest caret positions).
    private func saveSession() {
        var state = SessionState()
        for editor in editors {
            guard let path = editor.coreDocument.path else { continue }
            let position = editor.sessionPosition
            state.windows.append(
                SessionState.Window(
                    path: path, caret: position.caret, scroll: position.scroll))
        }
        state.frontmost =
            editors.first { $0.window?.isKeyWindow == true }?.coreDocument.path
            ?? state.windows.last?.path
        state.sidebarSplit = fileTreeState.splitFraction
        SessionStore.save(state)
    }

    /// Reopens the saved session's files and positions.
    private func restoreSession() {
        guard let state = SessionStore.load() else { return }
        if let split = state.sidebarSplit {
            fileTreeState.splitFraction = min(0.85, max(0.15, split))
        }
        var frontmostEditor: EditorWindowController?
        for window in state.windows
        where FileManager.default.fileExists(atPath: window.path) {
            open(path: window.path)
            guard let editor = editors.first(where: { $0.coreDocument.path == window.path })
            else { continue }
            editor.restoreSessionPosition(caret: window.caret, scroll: window.scroll)
            if window.path == state.frontmost {
                frontmostEditor = editor
            }
        }
        frontmostEditor?.window?.makeKeyAndOrderFront(nil)
    }

    /// Quitting reviews every dirty window through the same save/discard
    /// flow as closing it by hand, then records the session with the
    /// freshest positions.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        saveSession()
        for editor in editors {
            guard let window = editor.window else { continue }
            if !editor.windowShouldClose(window) {
                return .terminateCancel
            }
        }
        return .terminateNow
    }

    // MARK: Core events

    /// Servers already reported missing, so the alert shows once each.
    private var reportedMissingServers: Set<String> = []

    private func handleCoreEvent(_ event: CoreApp.Event) {
        switch event {
        case let .pong(sequence):
            NSLog("core \(Core.version) event channel verified (pong \(sequence))")
        case let .diagnostics(path, items):
            editors.first { $0.coreDocument.path == path }?.apply(diagnostics: items)
        case .lspResponse:
            break  // routed to its completion handler inside CoreApp
        case let .serverStatus(server, root, status, message):
            NSLog("lsp \(server) [\(root)]: \(status) \(message)")
            if status == "not-found", !reportedMissingServers.contains(server) {
                reportedMissingServers.insert(server)
                let alert = NSAlert()
                alert.alertStyle = .informational
                alert.messageText = "No language server for this project"
                alert.informativeText = message
                alert.runModal()
            }
            // A healthy server that stops running gets restarted with
            // backoff; "closed" is our own orderly shutdown and needs
            // nothing.
            if status == "running" {
                crashRestarts[server + "|" + root] = nil
            }
            if status == "exited", message.isEmpty {
                scheduleCrashRestart(server: server, root: root)
            }
            // A server that starts but dies before (or during) the
            // handshake deserves one loud notice too — with a pointer to
            // the log that holds its stderr.
            let diedEarly =
                status == "failed" || (status == "exited" && message == "during initialize")
            if diedEarly, !reportedMissingServers.contains(server) {
                reportedMissingServers.insert(server)
                let alert = NSAlert()
                alert.alertStyle = .warning
                alert.messageText = "Language server failed to start"
                alert.informativeText =
                    "\(server) exited during startup"
                    + (message.isEmpty || message == "during initialize"
                        ? "" : " (\(message))")
                    + ". Its own error output is in ~/Library/Logs/Textchum/lsp.log."
                alert.runModal()
            }
        }
    }

    /// Applies a rename's `WorkspaceEdit`: open documents edit in place
    /// (through the synchronized text-view path, so undo works), files
    /// nobody has open are rewritten on disk atomically. Returns whether
    /// anything was applied.
    func applyWorkspaceEdit(resultJSON: String) -> Bool {
        let byPath = LSPEdits.workspaceEdits(fromResultJSON: resultJSON)
        guard !byPath.isEmpty else { return false }
        for (path, edits) in byPath {
            if let editor = editors.first(where: { $0.coreDocument.path == path }) {
                editor.apply(textEdits: edits)
            } else if let contents = try? String(contentsOfFile: path, encoding: .utf8) {
                try? LSPEdits.applied(to: contents, edits: edits)
                    .write(toFile: path, atomically: true, encoding: .utf8)
            }
        }
        return true
    }

    /// Crash-restart attempts per (server, root), for backoff. Cleared
    /// when the instance reaches "running" again.
    private var crashRestarts: [String: Int] = [:]

    /// A server that died mid-session comes back on its own: retire the
    /// dead instance, wait 1 → 2 → 4 → 8 seconds across attempts, then
    /// re-announce the documents that were talking to it. Four failures
    /// in a row and it stays down until a restart or config change.
    private func scheduleCrashRestart(server: String, root: String) {
        let key = server + "|" + root
        let attempt = crashRestarts[key, default: 0]
        guard attempt < 4 else {
            NSLog("lsp \(server) [\(root)]: giving up after \(attempt) restarts")
            return
        }
        crashRestarts[key] = attempt + 1
        coreApp?.lspRetire(server: server, root: root)
        let delay = TimeInterval(1 << attempt)
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                NSLog("lsp \(server) [\(root)]: restart attempt \(attempt + 1)")
                for editor in self.editors {
                    let documentRoot =
                        editor.projectRoot
                        ?? editor.coreDocument.path.map {
                            ($0 as NSString).deletingLastPathComponent
                        }
                    if documentRoot == root {
                        editor.reannounceLSP()
                    }
                }
            }
        }
    }

    /// Retires every running server instance and re-announces the open
    /// documents, respawning them under the current configuration.
    private func restartLanguageServers() {
        guard let coreApp else { return }
        coreApp.lspRestartServers()
        reportedMissingServers.removeAll()
        for editor in editors {
            editor.reannounceLSP()
        }
    }

    @objc func toggleLineNumbers(_ sender: Any?) {
        settingsModel?.lineNumbers.toggle()
    }

    // MARK: Configurable key shortcuts

    /// Menu items by their stable action name, for `keys` overrides.
    private var menuActions: [String: NSMenuItem] = [:]

    /// Indexes every overridable menu item by a stable name.
    private func registerMenuActions(in menu: NSMenu) {
        let bySelector: [Selector: String] = [
            #selector(newDocument(_:)): "new",
            #selector(openDocument(_:)): "open",
            #selector(openQuickly(_:)): "openQuickly",
            #selector(EditorWindowController.saveDocument(_:)): "save",
            #selector(EditorWindowController.saveDocumentAs(_:)): "saveAs",
            #selector(EditorWindowController.revertToSaved(_:)): "revertToSaved",
            #selector(NSWindow.performClose(_:)): "close",
            #selector(EditorWindowController.performUndo(_:)): "undo",
            #selector(EditorWindowController.performRedo(_:)): "redo",
            #selector(EditorWindowController.jumpToDefinition(_:)): "jumpToDefinition",
            #selector(goBack(_:)): "goBack",
            #selector(goForward(_:)): "goForward",
            #selector(EditorWindowController.findReferences(_:)): "findReferences",
            #selector(EditorWindowController.renameSymbol(_:)): "renameSymbol",
            #selector(EditorWindowController.formatDocument(_:)): "formatDocument",
            #selector(EditorWindowController.goToBlockStart(_:)): "goToBlockStart",
            #selector(EditorWindowController.goToBlockEnd(_:)): "goToBlockEnd",
            #selector(EditorWindowController.triggerCompletion(_:)): "complete",
            #selector(findInProject(_:)): "findInProject",
            #selector(NSSplitViewController.toggleSidebar(_:)): "toggleNavigator",
            #selector(EditorWindowController.togglePreview(_:)): "togglePreview",
            #selector(toggleLineNumbers(_:)): "toggleLineNumbers",
            #selector(togglePathDisplay(_:)): "togglePathDisplay",
            #selector(EditorWindowController.redrawDocument(_:)): "redraw",
            #selector(EditorWindowController.showDocumentOutline(_:)): "documentOutline",
            #selector(showCommandPalette(_:)): "commandPalette",
            #selector(showSettings(_:)): "settings",
        ]
        let finderNames: [Int: String] = [
            NSTextFinder.Action.showFindInterface.rawValue: "find",
            NSTextFinder.Action.showReplaceInterface.rawValue: "findAndReplace",
            NSTextFinder.Action.nextMatch.rawValue: "findNext",
            NSTextFinder.Action.previousMatch.rawValue: "findPrevious",
            NSTextFinder.Action.setSearchString.rawValue: "useSelectionForFind",
        ]
        for item in menu.items {
            if let submenu = item.submenu {
                registerMenuActions(in: submenu)
            }
            guard let action = item.action else { continue }
            if action == #selector(NSResponder.performTextFinderAction(_:)) {
                if let name = finderNames[item.tag] {
                    menuActions[name] = item
                }
            } else if let name = bySelector[action] {
                menuActions[name] = item
            }
        }
    }

    /// Applies the configuration's `keys` overrides (`{action:
    /// "modifiers+key"}`) to the indexed menu items. Unknown actions and
    /// unparseable shortcuts are logged, never fatal.
    private func applyKeyOverrides() {
        guard let config,
            let data = config.keysJSON.data(using: .utf8),
            let overrides = try? JSONSerialization.jsonObject(with: data) as? [String: String]
        else { return }
        for (action, spec) in overrides {
            guard let item = menuActions[action] else {
                NSLog("keys: unknown action \(action) (known: \(menuActions.keys.sorted()))")
                continue
            }
            guard let (key, modifiers) = Self.parseShortcut(spec) else {
                NSLog("keys: could not parse shortcut \(spec) for \(action)")
                continue
            }
            item.keyEquivalent = key
            item.keyEquivalentModifierMask = modifiers
        }
    }

    /// Parses `"cmd+shift+f"`-style shortcut specs. Modifiers: cmd,
    /// shift, alt/option, ctrl. Keys: a single character, or up/down/
    /// left/right/return/escape/space/tab/delete.
    static func parseShortcut(_ spec: String) -> (String, NSEvent.ModifierFlags)? {
        var modifiers: NSEvent.ModifierFlags = []
        var key: String?
        for token in spec.lowercased().split(separator: "+").map(String.init) {
            switch token {
            case "cmd", "command": modifiers.insert(.command)
            case "shift": modifiers.insert(.shift)
            case "alt", "option", "opt": modifiers.insert(.option)
            case "ctrl", "control": modifiers.insert(.control)
            case "up": key = String(UnicodeScalar(NSUpArrowFunctionKey)!)
            case "down": key = String(UnicodeScalar(NSDownArrowFunctionKey)!)
            case "left": key = String(UnicodeScalar(NSLeftArrowFunctionKey)!)
            case "right": key = String(UnicodeScalar(NSRightArrowFunctionKey)!)
            case "return", "enter": key = "\r"
            case "escape", "esc": key = "\u{1b}"
            case "space": key = " "
            case "tab": key = "\t"
            case "delete", "backspace": key = "\u{8}"
            default:
                guard token.count == 1 else { return nil }
                key = token
            }
        }
        guard let key else { return nil }
        return (key, modifiers)
    }

    // MARK: Appearance & sidebar

    /// Applies the configured appearance app-wide. `system` (nil override)
    /// keeps following macOS live; the existing effective-appearance
    /// observation recolors syntax either way.
    private func applyAppearanceChoice() {
        switch config?.appearance ?? .system {
        case .system: NSApp.appearance = nil
        case .light: NSApp.appearance = NSAppearance(named: .aqua)
        case .dark: NSApp.appearance = NSAppearance(named: .darkAqua)
        }
    }

    /// One warning per launch when the chosen theme cannot be used.
    private var warnedBrokenTheme = false
    /// The theme name currently applied, to skip redundant recolors.
    private var appliedTheme: String?

    /// Applies the configured theme: a user file of that name (which
    /// overrides a same-named built-in), else the built-in, else the
    /// default — with one warning when the choice cannot be honored,
    /// never overwriting the broken file.
    private func applyThemeChoice() {
        let name = config?.theme ?? "Textchum"
        guard name != appliedTheme else { return }
        var problem: String?
        if let json = ThemeFiles.json(named: name) {
            problem = CoreTheme.setJSON(json)
        } else if !CoreTheme.setBuiltin(named: name) {
            problem = "no theme file or built-in theme has this name"
        }
        if let problem {
            CoreTheme.setBuiltin(named: "Textchum")
            if !warnedBrokenTheme {
                warnedBrokenTheme = true
                let alert = NSAlert()
                alert.alertStyle = .warning
                alert.messageText = "Theme “\(name)” could not be used"
                alert.informativeText = "\(problem) — using the default theme instead."
                alert.runModal()
            }
        }
        appliedTheme = name
        HighlightPalette.reload()
        for editor in editors {
            editor.refreshDecorations()
        }
    }

    /// Bare filenames collide when two open documents share one; those
    /// get as many trailing path components as it takes to tell them
    /// apart. Unique names get no entry.
    private func disambiguatedTitles() -> [ObjectIdentifier: String] {
        var byName: [String: [(id: ObjectIdentifier, components: [String])]] = [:]
        for editor in editors {
            guard let path = editor.coreDocument.path else { continue }
            let components = path.split(separator: "/").map(String.init)
            guard let name = components.last else { continue }
            byName[name, default: []].append((ObjectIdentifier(editor), components))
        }
        var result: [ObjectIdentifier: String] = [:]
        for documents in byName.values where documents.count > 1 {
            func suffix(_ components: [String], _ depth: Int) -> String {
                components.suffix(depth).joined(separator: "/")
            }
            var depth = 2
            let maxDepth = documents.map(\.components.count).max() ?? 1
            while depth < maxDepth,
                Set(documents.map { suffix($0.components, depth) }).count < documents.count
            {
                depth += 1
            }
            for document in documents {
                result[document.id] = suffix(document.components, depth)
            }
        }
        return result
    }

    /// Rebuilds every window's sidebar: each buffer list shows only the
    /// documents of that window's tab group, so separate windows keep
    /// separate worlds.
    private func rebuildSidebar() {
        // Titles first: the buffer rows below read window titles, and
        // colliding names should already carry their extra path.
        let overrides = disambiguatedTitles()
        for editor in editors {
            editor.setDisplayTitle(overrides[ObjectIdentifier(editor)])
        }
        for editor in editors {
            let group: [NSWindow]
            if let window = editor.window {
                group = window.tabGroup?.windows ?? [window]
            } else {
                group = []
            }
            let entries = editors
                .filter { peer in peer.window.map(group.contains) ?? false }
                .map { peer in
                    (
                        document: SidebarDocument(
                            id: ObjectIdentifier(peer),
                            title: peer.window?.title ?? "Untitled",
                            path: peer.coreDocument.path,
                            isDirty: peer.coreDocument.isDirty
                        ),
                        projectRoot: peer.projectRoot
                    )
                }
            editor.sidebarModel.rebuild(entries: entries)
        }
    }

    /// Cross-file navigation: front (or open) `path` and put the caret at
    /// an LSP position. Used by go-to-definition.
    private func openLocation(path: String, line: Int, character: Int) {
        recordJumpOrigin()
        navigate(to: JumpLocation(path: path, line: line, character: character))
    }

    /// Raw navigation — jump-stack traversal uses this directly so that
    /// going back is not itself a jump.
    private func navigate(to location: JumpLocation) {
        if let existing = editors.first(where: { $0.coreDocument.path == location.path }) {
            existing.window?.makeKeyAndOrderFront(nil)
            existing.reveal(line: location.line, character: location.character)
        } else {
            open(path: location.path)
            editors.first { $0.coreDocument.path == location.path }?
                .reveal(line: location.line, character: location.character)
        }
    }

    // MARK: Jump stack

    let jumpStack = JumpStack()

    /// The key editor's file and caret, as a jump-stack entry.
    private func currentJumpLocation() -> JumpLocation? {
        let editor =
            editors.first { $0.window?.isKeyWindow == true } ?? editors.first
        guard let editor, let path = editor.coreDocument.path else { return nil }
        let caret = editor.caretLSPPosition
        return JumpLocation(path: path, line: caret.line, character: caret.character)
    }

    /// Called by everything that jumps, before it navigates.
    func recordJumpOrigin() {
        if let origin = currentJumpLocation() {
            jumpStack.noteJump(from: origin)
        }
    }

    @objc func goBack(_ sender: Any?) {
        guard let current = currentJumpLocation(),
            let target = jumpStack.goBack(from: current)
        else {
            NSSound.beep()
            return
        }
        navigate(to: target)
    }

    @objc func goForward(_ sender: Any?) {
        guard let current = currentJumpLocation(),
            let target = jumpStack.goForward(from: current)
        else {
            NSSound.beep()
            return
        }
        navigate(to: target)
    }

    /// One explorer state for the whole app: the tree looks the same
    /// across tabs (and windows showing the same project).
    private let fileTreeState = FileTreeState()

    // MARK: Window arrangement (project group → windows/tabs)

    /// "Split into New Window": pulls the group's windows out of their
    /// tab groups and gathers them as tabs of a window of their own.
    private func splitIntoNewWindow(documentIDs: [ObjectIdentifier]) {
        let members = editors.filter { documentIDs.contains(ObjectIdentifier($0)) }
        guard let first = members.first, let anchor = first.window else { return }
        anchor.tabGroup?.removeWindow(anchor)
        for member in members.dropFirst() {
            guard let window = member.window else { continue }
            window.tabGroup?.removeWindow(window)
            anchor.addTabbedWindow(window, ordered: .above)
        }
        anchor.makeKeyAndOrderFront(nil)
        NotificationCenter.default.post(name: .textchumDocumentsChanged, object: nil)
    }

    /// "Gather Into …": adopts the group's windows into the target
    /// window's tab group. A window already tabbed with the target is
    /// left alone — but two standalone windows both have a nil tab
    /// group, so membership is checked, never group identity.
    private func mergeAsTabs(documentIDs: [ObjectIdentifier], into target: ObjectIdentifier) {
        guard
            let targetWindow = editors.first(where: { ObjectIdentifier($0) == target })?
                .window
        else { return }
        let members = editors.filter { documentIDs.contains(ObjectIdentifier($0)) }
        for member in members {
            guard let window = member.window, window != targetWindow else { continue }
            if let group = targetWindow.tabGroup, group.windows.contains(window) {
                continue
            }
            window.tabGroup?.removeWindow(window)
            targetWindow.addTabbedWindow(window, ordered: .above)
        }
        targetWindow.makeKeyAndOrderFront(nil)
        NotificationCenter.default.post(name: .textchumDocumentsChanged, object: nil)
    }

    /// One "Gather Into" destination per tab group, the asker's own
    /// group first as "This Window".
    private func windowTargets(asking host: ObjectIdentifier) -> [WindowTarget] {
        let hostGroupKey = editors.first { ObjectIdentifier($0) == host }
            .flatMap { editor -> ObjectIdentifier? in
                guard let window = editor.window else { return nil }
                return window.tabGroup.map(ObjectIdentifier.init) ?? ObjectIdentifier(window)
            }
        var seen = Set<ObjectIdentifier>()
        var targets: [WindowTarget] = []
        for editor in editors {
            guard let window = editor.window else { continue }
            let groupKey =
                window.tabGroup.map(ObjectIdentifier.init) ?? ObjectIdentifier(window)
            guard !seen.contains(groupKey) else { continue }
            seen.insert(groupKey)
            let tabCount = window.tabGroup?.windows.count ?? 1
            let extra = tabCount > 1 ? " (+\(tabCount - 1))" : ""
            let title: String
            if groupKey == hostGroupKey {
                title = "This Window"
            } else {
                title = (window.tabGroup?.selectedWindow ?? window).title + extra
            }
            let target = WindowTarget(id: ObjectIdentifier(editor), title: title)
            if groupKey == hostGroupKey {
                targets.insert(target, at: 0)
            } else {
                targets.append(target)
            }
        }
        return targets
    }

    /// The pool configuration: server entries plus workspace behavior.
    private var combinedLSPConfiguration: String {
        "{\"lsp\":\(config?.lspJSON ?? "{}"),\"workspace\":\(config?.workspaceJSON ?? "{}")}"
    }

    private var sidebarConfiguration: SidebarConfiguration {
        SidebarConfiguration(
            treeState: fileTreeState,
            resolveProjectRoot: { [weak self] path in
                CoreWorkspace.projectRoot(
                    forPath: path, settingsJSON: self?.config?.workspaceJSON ?? "{}")
            },
            workspaceSettingsJSON: { [weak self] in self?.config?.workspaceJSON ?? "{}" },
            selectDocument: { [weak self] id in
                guard let editor = self?.editors.first(where: { ObjectIdentifier($0) == id })
                else { return }
                editor.window?.makeKeyAndOrderFront(nil)
            },
            openFile: { [weak self] path in
                guard let self else { return }
                // Focus an existing window for the file rather than
                // opening it twice.
                if let existing = self.editors.first(where: { $0.coreDocument.path == path }) {
                    existing.window?.makeKeyAndOrderFront(nil)
                } else {
                    self.open(path: path)
                }
            },
            splitGroup: { [weak self] ids in
                self?.splitIntoNewWindow(documentIDs: ids)
            },
            mergeGroup: { [weak self] ids, target in
                self?.mergeAsTabs(documentIDs: ids, into: target)
            },
            windowTargets: { [weak self] host in
                self?.windowTargets(asking: host) ?? []
            }
        )
    }

    // MARK: Quick search

    private let quickFinder = QuickFinderPanel()
    private let commandPalette = CommandPalettePanel()

    /// ⇧⌘P: the fuzzy-searchable menu.
    @objc func showCommandPalette(_ sender: Any?) {
        commandPalette.show(over: NSApp.keyWindow)
    }

    /// ⌥⌘T: buffer rows show project-relative paths while on. Applied to
    /// every window's sidebar so switching tabs keeps a consistent view;
    /// deliberately session-only.
    @objc func togglePathDisplay(_ sender: Any?) {
        let active =
            editors.first { $0.window == NSApp.keyWindow }?.sidebarModel
            ?? editors.first?.sidebarModel
        let newValue = !(active?.showFullPaths ?? false)
        for editor in editors {
            editor.sidebarModel.showFullPaths = newValue
        }
    }

    /// The search scope for the key window: its project, else its file's
    /// directory, else home — always shown editable in the panel.
    var currentScope: String {
        let keyEditor = editors.first { $0.window?.isKeyWindow == true } ?? editors.first
        if let root = keyEditor?.projectRoot { return root }
        if let path = keyEditor?.coreDocument.path {
            return (path as NSString).deletingLastPathComponent
        }
        return NSHomeDirectory()
    }

    private func showQuickFinder(mode: QuickFinderPanel.Mode) {
        quickFinder.onOpen = { [weak self] path, line in
            guard let self else { return }
            if line > 0 {
                self.openLocation(path: path, line: line - 1, character: 0)
            } else {
                // A whole-file open is a jump too.
                self.recordJumpOrigin()
                if let existing = self.editors.first(where: {
                    $0.coreDocument.path == path
                }) {
                    existing.window?.makeKeyAndOrderFront(nil)
                } else {
                    self.open(path: path)
                }
            }
        }
        quickFinder.show(mode: mode, scope: currentScope, over: NSApp.keyWindow)
    }

    @objc func openQuickly(_ sender: Any?) {
        showQuickFinder(mode: .files)
    }

    @objc func findInProject(_ sender: Any?) {
        showQuickFinder(mode: .grep)
    }

    // MARK: Recent files

    /// The Open Recent submenu; rebuilt from NSDocumentController's
    /// persisted list each time the menu opens.
    private var openRecentMenu: NSMenu?

    func noteRecent(path: String) {
        NSDocumentController.shared.noteNewRecentDocumentURL(URL(fileURLWithPath: path))
    }

    @objc private func openRecentDocument(_ sender: NSMenuItem) {
        guard let path = sender.representedObject as? String else { return }
        // Focus if already open, exactly like the navigator does.
        if let existing = editors.first(where: { $0.coreDocument.path == path }) {
            existing.window?.makeKeyAndOrderFront(nil)
        } else {
            open(path: path)
        }
    }

    @objc private func clearRecentDocuments(_ sender: Any?) {
        NSDocumentController.shared.clearRecentDocuments(nil)
    }

    /// Rebuilds Open Recent from the persisted list every time it opens.
    func menuNeedsUpdate(_ menu: NSMenu) {
        guard menu === openRecentMenu else { return }
        menu.removeAllItems()
        let recents = NSDocumentController.shared.recentDocumentURLs
        for url in recents {
            let item = NSMenuItem(
                title: url.lastPathComponent,
                action: #selector(openRecentDocument(_:)),
                keyEquivalent: ""
            )
            item.target = self
            item.representedObject = url.path
            item.toolTip = url.path
            menu.addItem(item)
        }
        if recents.isEmpty {
            let empty = NSMenuItem(title: "No Recent Files", action: nil, keyEquivalent: "")
            empty.isEnabled = false
            menu.addItem(empty)
        } else {
            menu.addItem(.separator())
            let clear = NSMenuItem(
                title: "Clear Menu",
                action: #selector(clearRecentDocuments(_:)),
                keyEquivalent: ""
            )
            clear.target = self
            menu.addItem(clear)
        }
    }

    // MARK: Settings

    @objc func showSettings(_ sender: Any?) {
        guard let settingsModel else { return }
        if settingsWindowController == nil {
            settingsWindowController = SettingsWindowController(model: settingsModel)
            settingsWindowController?.window?.center()
        }
        settingsWindowController?.showWindow(nil)
        settingsWindowController?.window?.makeKeyAndOrderFront(nil)
    }

    // MARK: Document actions

    private var currentSettings: EditorSettings? {
        settingsModel?.currentSettings
    }

    @objc func newDocument(_ sender: Any?) {
        makeUntitled(language: nil)
    }

    /// File → New with Format: an untitled document already speaking a
    /// language, so highlighting works before the first save. The menu
    /// item's represented object carries the language name.
    @objc func newDocumentWithFormat(_ sender: Any?) {
        let language = (sender as? NSMenuItem)?.representedObject as? String
        makeUntitled(language: language)
    }

    private func makeUntitled(language: String?) {
        // The folder of whatever was frontmost is the best guess for
        // where this new file belongs — Save As starts there.
        let suggestedDirectory = editors
            .first { $0.window?.isKeyWindow == true }?
            .coreDocument.path
            .map { URL(fileURLWithPath: $0).deletingLastPathComponent() }
            ?? editors.first?.coreDocument.path
            .map { URL(fileURLWithPath: $0).deletingLastPathComponent() }
        let document = CoreDocument()
        if let language {
            _ = document.setLanguage(language)
        }
        let editor = EditorWindowController(
            document: document,
            settings: currentSettings,
            sidebar: sidebarConfiguration,
            lspApp: coreApp,
            openLocation: { [weak self] path, line, character in
                self?.openLocation(path: path, line: line, character: character)
            })
        editor.suggestedSaveDirectory = suggestedDirectory
        show(editor: editor)
    }

    @objc func openDocument(_ sender: Any?) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        guard panel.runModal() == .OK else { return }
        for url in panel.urls {
            open(path: url.path)
        }
    }

    /// Untitled windows that were never touched: pathless, clean, empty.
    /// Opening a real file replaces them rather than leaving them behind.
    private func closeUntouchedUntitledWindows() {
        let untouched = editors.filter {
            $0.coreDocument.path == nil && !$0.coreDocument.isDirty
                && $0.coreDocument.lengthInBytes == 0
        }
        for editor in untouched {
            editor.window?.close()
        }
    }

    /// Opens `path` in a new editor (or fronts an existing one), alerting
    /// on failure. `target` overrides the configured tab/window choice;
    /// `revealLine` puts the caret on a one-based line.
    private func open(path: String, target: CoreOpenTarget? = nil, revealLine: Int? = nil) {
        // Absolute, standardized paths throughout: relative paths (e.g.
        // from the command line) would corrupt project-root resolution
        // and defeat open-file deduplication.
        let path = URL(fileURLWithPath: path).standardizedFileURL.path
        if let existing = editors.first(where: { $0.coreDocument.path == path }) {
            existing.window?.makeKeyAndOrderFront(nil)
            if let revealLine {
                existing.reveal(line: revealLine - 1, character: 0)
            }
            return
        }
        do {
            let document = try CoreDocument(contentsOf: path)
            closeUntouchedUntitledWindows()
            noteRecent(path: path)
            let editor = EditorWindowController(
                document: document,
                settings: currentSettings,
                sidebar: sidebarConfiguration,
                lspApp: coreApp,
                openLocation: { [weak self] path, line, character in
                    self?.openLocation(path: path, line: line, character: character)
                }
            )
            show(editor: editor, placeAsConfigured: true, target: target)
            if let revealLine {
                editor.reveal(line: revealLine - 1, character: 0)
            }
        } catch {
            let alert = NSAlert()
            alert.alertStyle = .warning
            alert.messageText = "Could not open “\((path as NSString).lastPathComponent)”."
            alert.informativeText = "\(error)"
            alert.runModal()
        }
    }

    /// Attaches `editor` per the requested (or configured) open target: as
    /// a tab of the key editor window's group, or as its own window.
    private func place(editor: EditorWindowController, target: CoreOpenTarget?) {
        guard (target ?? config?.openTarget) == .tab,
            let newWindow = editor.window,
            let anchor = editors.first(where: { $0 !== editor && $0.window?.isKeyWindow == true })
                ?? editors.first(where: { $0 !== editor && $0.window != nil })
        else { return }
        anchor.window?.addTabbedWindow(newWindow, ordered: .above)
    }

    private func show(
        editor: EditorWindowController,
        placeAsConfigured: Bool = false,
        target: CoreOpenTarget? = nil
    ) {
        if placeAsConfigured {
            place(editor: editor, target: target)
        }
        editors.append(editor)
        if let window = editor.window {
            NotificationCenter.default.addObserver(
                forName: NSWindow.willCloseNotification,
                object: window,
                queue: .main
            ) { [weak self] notification in
                guard let closing = notification.object as? NSWindow else { return }
                MainActor.assumeIsolated {
                    if let editor = self?.editors.first(where: { $0.window === closing }) {
                        self?.releaseChumWait(for: editor)
                    }
                    self?.editors.removeAll { $0.window === closing }
                }
                DispatchQueue.main.async {
                    MainActor.assumeIsolated {
                        self?.rebuildSidebar()
                        self?.saveSession()
                    }
                }
            }
        }
        editor.showWindow(nil)
        // No direct rebuild here: the controller publishes its state (via
        // updateChrome) and the deferred notification handler rebuilds —
        // rebuilding synchronously mid-presentation trips NSTableView.
    }

    // MARK: Menu

    /// Programmatic main menu: the app has no nib. Undo/redo use dedicated
    /// selectors handled by the editor window controller, since document
    /// history lives in the core rather than in an `NSUndoManager`.
    private func makeMainMenu() -> NSMenu {
        let mainMenu = NSMenu()

        let appMenu = NSMenu()
        appMenu.addItem(
            withTitle: "About Textchum",
            action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
            keyEquivalent: ""
        )
        appMenu.addItem(.separator())
        appMenu.addItem(
            withTitle: "Settings…",
            action: #selector(showSettings(_:)),
            keyEquivalent: ","
        )
        appMenu.addItem(
            withTitle: "Install chum Command…",
            action: #selector(installCommandLineTool(_:)),
            keyEquivalent: ""
        )
        appMenu.addItem(
            withTitle: "Open Themes Folder",
            action: #selector(openThemesFolder(_:)),
            keyEquivalent: ""
        )
        appMenu.addItem(.separator())
        appMenu.addItem(
            withTitle: "Quit Textchum",
            action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q"
        )
        let appMenuItem = NSMenuItem()
        appMenuItem.submenu = appMenu
        mainMenu.addItem(appMenuItem)

        let fileMenu = NSMenu(title: "File")
        fileMenu.addItem(
            withTitle: "New", action: #selector(newDocument(_:)), keyEquivalent: "n")
        let formatItem = NSMenuItem(title: "New with Format", action: nil, keyEquivalent: "")
        let formatMenu = NSMenu(title: "New with Format")
        for language in CoreLanguages.all {
            let item = NSMenuItem(
                title: language.name.capitalized,
                action: #selector(newDocumentWithFormat(_:)),
                keyEquivalent: ""
            )
            item.representedObject = language.name
            formatMenu.addItem(item)
        }
        formatItem.submenu = formatMenu
        fileMenu.addItem(formatItem)
        fileMenu.addItem(
            withTitle: "Open…", action: #selector(openDocument(_:)), keyEquivalent: "o")
        fileMenu.addItem(
            withTitle: "Open Quickly…", action: #selector(openQuickly(_:)), keyEquivalent: "t")
        let openRecentItem = NSMenuItem(title: "Open Recent", action: nil, keyEquivalent: "")
        let openRecent = NSMenu(title: "Open Recent")
        openRecent.delegate = self
        openRecentItem.submenu = openRecent
        self.openRecentMenu = openRecent
        fileMenu.addItem(openRecentItem)
        fileMenu.addItem(.separator())
        fileMenu.addItem(
            withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        fileMenu.addItem(
            withTitle: "Save",
            action: #selector(EditorWindowController.saveDocument(_:)),
            keyEquivalent: "s"
        )
        let saveAs = NSMenuItem(
            title: "Save As…",
            action: #selector(EditorWindowController.saveDocumentAs(_:)),
            keyEquivalent: "s"
        )
        saveAs.keyEquivalentModifierMask = [.command, .shift]
        fileMenu.addItem(saveAs)
        let revert = NSMenuItem(
            title: "Revert to Saved",
            action: #selector(EditorWindowController.revertToSaved(_:)),
            keyEquivalent: "r"
        )
        revert.keyEquivalentModifierMask = [.command, .option]
        fileMenu.addItem(revert)
        fileMenu.addItem(.separator())
        // The front tab's location in every useful spelling; also on the
        // context menus of buffer-list and file-tree rows.
        let copyPathItem = NSMenuItem(title: "Copy Path", action: nil, keyEquivalent: "")
        let copyPath = NSMenu(title: "Copy Path")
        copyPath.addItem(
            withTitle: "File Name",
            action: #selector(EditorWindowController.copyFileName(_:)), keyEquivalent: "")
        copyPath.addItem(
            withTitle: "Relative Path",
            action: #selector(EditorWindowController.copyRelativePath(_:)), keyEquivalent: "")
        copyPath.addItem(
            withTitle: "Absolute Path",
            action: #selector(EditorWindowController.copyAbsolutePath(_:)), keyEquivalent: "")
        copyPath.addItem(
            withTitle: "Forge URL",
            action: #selector(EditorWindowController.copyForgeURL(_:)), keyEquivalent: "")
        copyPathItem.submenu = copyPath
        fileMenu.addItem(copyPathItem)
        let fileMenuItem = NSMenuItem()
        fileMenuItem.submenu = fileMenu
        mainMenu.addItem(fileMenuItem)

        let editMenu = NSMenu(title: "Edit")
        let undo = NSMenuItem(
            title: "Undo",
            action: #selector(EditorWindowController.performUndo(_:)),
            keyEquivalent: "z"
        )
        editMenu.addItem(undo)
        let redo = NSMenuItem(
            title: "Redo",
            action: #selector(EditorWindowController.performRedo(_:)),
            keyEquivalent: "Z"
        )
        editMenu.addItem(redo)
        editMenu.addItem(.separator())
        editMenu.addItem(
            withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        editMenu.addItem(
            withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        editMenu.addItem(
            withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        editMenu.addItem(
            withTitle: "Select All",
            action: #selector(NSText.selectAll(_:)),
            keyEquivalent: "a"
        )
        editMenu.addItem(.separator())
        let jump = NSMenuItem(
            title: "Jump to Definition",
            action: #selector(EditorWindowController.jumpToDefinition(_:)),
            keyEquivalent: "j"
        )
        jump.keyEquivalentModifierMask = [.command, .control]
        editMenu.addItem(jump)
        let backItem = NSMenuItem(
            title: "Go Back",
            action: #selector(goBack(_:)),
            keyEquivalent: String(UnicodeScalar(NSLeftArrowFunctionKey)!)
        )
        backItem.keyEquivalentModifierMask = [.control, .command]
        editMenu.addItem(backItem)
        let forwardItem = NSMenuItem(
            title: "Go Forward",
            action: #selector(goForward(_:)),
            keyEquivalent: String(UnicodeScalar(NSRightArrowFunctionKey)!)
        )
        forwardItem.keyEquivalentModifierMask = [.control, .command]
        editMenu.addItem(forwardItem)
        let references = NSMenuItem(
            title: "Find References",
            action: #selector(EditorWindowController.findReferences(_:)),
            keyEquivalent: "r"
        )
        references.keyEquivalentModifierMask = [.command, .shift]
        editMenu.addItem(references)
        let rename = NSMenuItem(
            title: "Rename Symbol…",
            action: #selector(EditorWindowController.renameSymbol(_:)),
            keyEquivalent: "r"
        )
        rename.keyEquivalentModifierMask = [.command, .control]
        editMenu.addItem(rename)
        let format = NSMenuItem(
            title: "Format Document",
            action: #selector(EditorWindowController.formatDocument(_:)),
            keyEquivalent: "f"
        )
        format.keyEquivalentModifierMask = [.command, .option, .shift]
        editMenu.addItem(format)
        let blockStart = NSMenuItem(
            title: "Go to Block Start",
            action: #selector(EditorWindowController.goToBlockStart(_:)),
            keyEquivalent: String(UnicodeScalar(NSUpArrowFunctionKey)!)
        )
        blockStart.keyEquivalentModifierMask = [.control, .option]
        editMenu.addItem(blockStart)
        let blockEnd = NSMenuItem(
            title: "Go to Block End",
            action: #selector(EditorWindowController.goToBlockEnd(_:)),
            keyEquivalent: String(UnicodeScalar(NSDownArrowFunctionKey)!)
        )
        blockEnd.keyEquivalentModifierMask = [.control, .option]
        editMenu.addItem(blockEnd)
        let complete = NSMenuItem(
            title: "Complete",
            action: #selector(EditorWindowController.triggerCompletion(_:)),
            keyEquivalent: " "
        )
        complete.keyEquivalentModifierMask = [.control]
        editMenu.addItem(complete)
        editMenu.addItem(.separator())

        // Find submenu, driving the text view's native find bar.
        func finderItem(
            _ title: String,
            _ action: NSTextFinder.Action,
            _ key: String,
            _ modifiers: NSEvent.ModifierFlags = [.command]
        ) -> NSMenuItem {
            let item = NSMenuItem(
                title: title,
                action: #selector(NSResponder.performTextFinderAction(_:)),
                keyEquivalent: key
            )
            item.tag = action.rawValue
            item.keyEquivalentModifierMask = modifiers
            return item
        }
        let findMenu = NSMenu(title: "Find")
        findMenu.addItem(finderItem("Find…", .showFindInterface, "f"))
        findMenu.addItem(
            finderItem("Find and Replace…", .showReplaceInterface, "f", [.command, .option]))
        findMenu.addItem(finderItem("Find Next", .nextMatch, "g"))
        findMenu.addItem(finderItem("Find Previous", .previousMatch, "g", [.command, .shift]))
        findMenu.addItem(finderItem("Use Selection for Find", .setSearchString, "e"))
        findMenu.addItem(.separator())
        let findInProject = NSMenuItem(
            title: "Find in Project…",
            action: #selector(findInProject(_:)),
            keyEquivalent: "f"
        )
        findInProject.keyEquivalentModifierMask = [.command, .shift]
        findMenu.addItem(findInProject)
        let findMenuItem = NSMenuItem(title: "Find", action: nil, keyEquivalent: "")
        findMenuItem.submenu = findMenu
        editMenu.addItem(findMenuItem)

        let editMenuItem = NSMenuItem()
        editMenuItem.submenu = editMenu
        mainMenu.addItem(editMenuItem)

        let viewMenu = NSMenu(title: "View")
        let toggleNavigator = NSMenuItem(
            title: "Toggle Navigator",
            action: #selector(NSSplitViewController.toggleSidebar(_:)),
            keyEquivalent: "0"
        )
        viewMenu.addItem(toggleNavigator)
        let togglePreview = NSMenuItem(
            title: "Toggle Markdown Preview",
            action: #selector(EditorWindowController.togglePreview(_:)),
            keyEquivalent: "p"
        )
        togglePreview.keyEquivalentModifierMask = [.command, .option]
        viewMenu.addItem(togglePreview)
        let lineNumbersItem = NSMenuItem(
            title: "Toggle Line Numbers",
            action: #selector(toggleLineNumbers(_:)),
            keyEquivalent: "l"
        )
        lineNumbersItem.keyEquivalentModifierMask = [.command, .shift]
        viewMenu.addItem(lineNumbersItem)
        let pathDisplayItem = NSMenuItem(
            title: "Toggle Path Display",
            action: #selector(togglePathDisplay(_:)),
            keyEquivalent: "t"
        )
        pathDisplayItem.keyEquivalentModifierMask = [.command, .option]
        viewMenu.addItem(pathDisplayItem)
        let outlineItem = NSMenuItem(
            title: "Document Outline…",
            action: #selector(EditorWindowController.showDocumentOutline(_:)),
            keyEquivalent: "o"
        )
        outlineItem.keyEquivalentModifierMask = [.command, .shift]
        viewMenu.addItem(outlineItem)
        let redrawItem = NSMenuItem(
            title: "Redraw",
            action: #selector(EditorWindowController.redrawDocument(_:)),
            keyEquivalent: "l"
        )
        redrawItem.keyEquivalentModifierMask = [.command, .option]
        viewMenu.addItem(redrawItem)
        viewMenu.addItem(.separator())
        let paletteItem = NSMenuItem(
            title: "Command Palette…",
            action: #selector(showCommandPalette(_:)),
            keyEquivalent: "p"
        )
        paletteItem.keyEquivalentModifierMask = [.command, .shift]
        viewMenu.addItem(paletteItem)
        let viewMenuItem = NSMenuItem()
        viewMenuItem.submenu = viewMenu
        mainMenu.addItem(viewMenuItem)

        let windowMenu = NSMenu(title: "Window")
        windowMenu.addItem(
            withTitle: "Minimize",
            action: #selector(NSWindow.performMiniaturize(_:)),
            keyEquivalent: "m"
        )
        let windowMenuItem = NSMenuItem()
        windowMenuItem.submenu = windowMenu
        mainMenu.addItem(windowMenuItem)
        NSApp.windowsMenu = windowMenu

        return mainMenu
    }
}
