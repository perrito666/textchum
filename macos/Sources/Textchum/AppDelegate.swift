import AppKit
import TextchumKit

/// Application lifecycle: the main menu, the core instance, configuration,
/// and the set of open editor windows.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private var coreApp: CoreApp?
    private(set) var config: CoreConfig?
    private var settingsModel: SettingsModel?
    private var settingsWindowController: SettingsWindowController?
    /// Strong references to open editors; windows do not retain their
    /// controllers. Entries are removed as their windows close.
    private var editors: [DocumentController] = []

    /// `~/Library/Application Support/Textchum/config.json` — GUI-managed,
    /// hand-editable JSON. `--config <path>` points at another file and
    /// `--data-dir <path>` moves the whole profile; see `AppPaths`.
    private static var configPath: String { AppPaths.configPath }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let config = CoreConfig(path: Self.configPath)
        self.config = config
        // The interface language first: every label below is drawn in
        // it, and the menu bar is built before anything else.
        SessionStore.useProfile(ofConfigAt: Self.configPath)
        CoreI18n.use(
            config.interfaceLanguage,
            catalogueDirectory: SessionStore.directory
                .appendingPathComponent("translations", isDirectory: true).path)

        let mainMenu = makeMainMenu()
        NSApp.mainMenu = mainMenu
        registerMenuActions(in: mainMenu)
        // Grammars the build does not carry, named in the
        // configuration. One that cannot be opened costs that language
        // and nothing else, so the rest of the launch carries on.
        let grammarProblems = config.loadGrammars()
        for problem in grammarProblems {
            NSLog("languages: \(problem)")
        }
        self.grammarProblems = grammarProblems
        // The session belongs to the configuration's profile: a scratch
        // --config run must never write over the real session.
        SessionStore.useProfile(ofConfigAt: Self.configPath)
        applyKeyOverrides()
        let settingsModel = SettingsModel(config: config)
        settingsModel.onChange = { [weak self] in
            guard let self, let model = self.settingsModel else { return }
            // The change was ours; the config watcher must not treat the
            // save's echo as an external edit.
            self.lastOwnConfigSave = Date()
            self.applyKeyOverrides()
            // Choosing a profile moves the shortcuts; the fields have to
            // say so, or picking one looks like it did nothing.
            self.refreshShortcutCatalog()
            self.applyAppearanceChoice()
            self.applyThemeChoice()
            self.applyIconPack()
            self.coreApp?.lspConfigure(json: self.combinedLSPConfiguration)
            for editor in self.editors {
                editor.refreshProjectRoot()
            }
            for editor in self.editors {
                editor.apply(settings: model.currentSettings(forRoot: editor.projectRoot))
            }
        }
        self.settingsModel = settingsModel
        applyAppearanceChoice()
        applyThemeChoice()
        applyIconPack()
        startWatchingConfig()
        installCommandClickMonitor()

        NotificationCenter.default.addObserver(
            forName: .textchumDocumentsChanged, object: nil, queue: .main
        ) { [weak self] notification in
            let changedEditor = notification.object as? DocumentController
            // Deferred a runloop turn: the notification can fire while
            // AppKit is mid-layout (e.g. from a window-title update), and
            // rebuilding the list reentrantly trips NSTableView.
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    self?.rebuildSidebar()
                    if self?.isTerminating != true {
                        self?.saveSession()
                    }
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
        CoreWorkspace.setLSPLogPath(AppPaths.logFile.path)

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
        for flag in AppPaths.valueFlags {
            if let at = arguments.firstIndex(of: flag) {
                flagValueIndexes.insert(at + 1)
            }
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
                self.announceGrammarProblems()
                // Records for roots that are gone, and those past their
                // keep window, on a thread of their own.
                ProjectState.sweepAtLaunch()
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
                    if allArguments[flagIndex + 1] == "typefiles" {
                        self?.showQuickFinder(mode: .files)
                        let typedScope =
                            scope == "-" ? (self?.currentScope ?? "") : scope
                        self?.quickFinder.debugType(scope: typedScope, query: query)
                        return
                    }
                    if allArguments[flagIndex + 1] == "newformat" {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                            MainActor.assumeIsolated {
                                self?.newDocumentWithFormatPicker(nil)
                            }
                        }
                        return
                    }
                    if allArguments[flagIndex + 1] == "newplacement" {
                        // Two fresh documents; with the tab default they
                        // must share one tab group.
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                            MainActor.assumeIsolated {
                                guard let self else { return }
                                self.newDocument(nil)
                                self.newDocument(nil)
                                let tabs =
                                    self.editors.first?.window?.tabbedWindows?.count ?? 0
                                NSLog("debug newplacement tabs=\(tabs)")
                            }
                        }
                        return
                    }
                    if allArguments[flagIndex + 1] == "about" {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                            MainActor.assumeIsolated {
                                self?.showAbout(nil)
                            }
                        }
                        return
                    }
                    if allArguments[flagIndex + 1] == "scrollto" {
                        // scope = fraction of the document to scroll to,
                        // so viewport-scoped colouring is verifiable
                        // deep inside a large file.
                        let fraction = Double(scope) ?? 0.5
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                            MainActor.assumeIsolated {
                                self?.editors.first?.debugScroll(toFraction: fraction)
                            }
                        }
                        return
                    }
                    if allArguments[flagIndex + 1] == "quitafter" {
                        // Quit through the real path, so the session
                        // written at shutdown is the one under test.
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                            MainActor.assumeIsolated { NSApp.terminate(nil) }
                        }
                        return
                    }
                    if allArguments[flagIndex + 1] == "hoverat" {
                        // scope=line, query=character: park the caret and
                        // ask the server, so the balloon holds real content.
                        let line = Int(scope) ?? 0
                        let character = Int(query) ?? 0
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                            MainActor.assumeIsolated {
                                guard let editor = self?.editors.first else { return }
                                editor.reveal(line: line, character: character)
                                editor.showHoverAtCaret(nil)
                            }
                        }
                        return
                    }
                    if allArguments[flagIndex + 1] == "status" {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                            MainActor.assumeIsolated {
                                self?.showServerStatus(nil)
                            }
                        }
                        return
                    }
                    if allArguments[flagIndex + 1] == "preprocess" {
                        // Exercise the save-preprocessor chain end to
                        // end on the named document: save through the
                        // same path a user's ⌘S takes. The path-suffix
                        // argument picks the window, so an untitled or
                        // restored one can never soak up the save.
                        let suffix = scope
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                            MainActor.assumeIsolated {
                                let target = self?.editors.first {
                                    $0.coreDocument.path?.hasSuffix(suffix) == true
                                }
                                _ = target?.saveInteractively()
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
                    if allArguments[flagIndex + 1] == "properties" {
                        // scope = a language to choose, or "-" to just
                        // open the panel and photograph it.
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                            MainActor.assumeIsolated {
                                guard let editor = self?.editors.first else { return }
                                editor.showFileProperties(nil)
                                guard scope != "-", let path = editor.coreDocument.path
                                else { return }
                                // The same two steps the panel takes,
                                // so this exercises saving as well as
                                // applying.
                                self?.setFileOverride(
                                    path: path, .init(language: scope))
                                editor.applyFileProperties(.init(language: scope))
                            }
                        }
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
                        // The pool needs a moment to spawn and shake
                        // hands before a completion can be answered.
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                            MainActor.assumeIsolated {
                                guard let editor = self?.editors.first else { return }
                                editor.reveal(line: line, character: character)
                                editor.triggerCompletion(nil)
                            }
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
            alert.messageText = t("Settings file could not be read")
            alert.informativeText = warning
            alert.runModal()
        }
    }

    /// Files opened from Finder (double-click, Open With, drag to icon)
    /// and `textchum://` URLs from the `chum` command.
    /// Says once what the configured grammars could not do.
    private func announceGrammarProblems() {
        guard !grammarProblems.isEmpty else { return }
        let problems = grammarProblems
        grammarProblems = []
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText =
            problems.count == 1
            ? "A configured grammar could not be loaded"
            : "\(problems.count) configured grammars could not be loaded"
        alert.informativeText = problems.joined(separator: "\n")
        alert.runModal()
    }

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

    private func releaseChumWait(for editor: DocumentController) {
        if let sentinel = chumWaitSentinels.removeValue(forKey: ObjectIdentifier(editor)) {
            try? FileManager.default.removeItem(atPath: sentinel)
        }
    }

    /// True from the moment quitting begins. Windows close one by one
    /// during termination, and each close would otherwise schedule a
    /// session save over an emptying list — the authoritative save
    /// already happened at the top of `applicationShouldTerminate`.
    /// What the configured grammars could not do, said once when the
    /// first window is up: a grammar that fails silently looks like a
    /// language with no colour, which says nothing about why.
    private var grammarProblems: [String] = []
    private var isTerminating = false

    func applicationWillTerminate(_ notification: Notification) {
        isTerminating = true
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
        case #selector(toggleHoverDocs(_:)):
            menuItem.state = (settingsModel?.hoverDocs ?? true) ? .on : .off
            return true
        default:
            return true
        }
    }

    // MARK: Session

    /// Writes the current session: open files with their positions.
    /// Called eagerly on document changes and window closes, and at quit
    /// (which captures the freshest caret positions).
    private func saveSession() {
        for workbench in Workbench.all {
            workbench.recordLayouts()
        }
        var state = SessionState()
        for editor in editors {
            guard let path = editor.coreDocument.path else { continue }
            let position = editor.sessionPosition
            state.windows.append(
                SessionState.Window(
                    path: path, caret: position.caret, scroll: position.scroll))
        }
        // Which window held what, and what each pane was showing: a
        // session that comes back as one window of everything is not
        // the session that was left.
        state.layout = Workbench.all.map { workbench in
            SessionState.Layout(
                tabs: workbench.documents.compactMap { $0.coreDocument.path },
                columns: workbench.columns.compactMap { column in
                    guard let file = column.document?.coreDocument.path else { return nil }
                    return SessionState.ColumnLayout(
                        file: file,
                        views: column.views.count,
                        dividers: column.dividerFractions)
                }
            )
        }
        state.frontmost =
            Workbench.all.first { $0.window?.isKeyWindow == true }?
            .focusedDocument?.coreDocument.path
            ?? state.windows.last?.path
        state.sidebarSplit = fileTreeState.splitFraction
        SessionStore.save(state)
    }

    // MARK: Reopening closed tabs

    /// Documents that were closed, newest last, with where the caret and
    /// scroll were. Only saved ones: an untitled buffer has nothing to
    /// reopen from, and reopening it empty would be a lie.
    private var closedDocuments: [(path: String, caret: Int, scroll: Double)] = []

    /// Deep enough to undo a run of mistaken closes, shallow enough that
    /// the list stays a list of recent mistakes rather than a second
    /// history — the session file is already the history.
    private static let closedDocumentMemory = 20

    /// Remembers a closing window so ⇧⌘T can bring it back, and hands
    /// the document to the store's cache: what comes back is the file
    /// as it was, unsaved text and all, rather than what is on disk.
    private func noteClosedEditor(_ editor: DocumentController) {
        DocumentStore.shared.close(editor.openDocument.id)
        guard let path = editor.coreDocument.path else { return }
        let position = editor.sessionPosition
        closedDocuments.removeAll { $0.path == path }
        closedDocuments.append((path, position.caret, position.scroll))
        if closedDocuments.count > Self.closedDocumentMemory {
            closedDocuments.removeFirst(closedDocuments.count - Self.closedDocumentMemory)
        }
    }

    @objc func reopenClosedDocument(_ sender: Any?) {
        guard let closed = closedDocuments.popLast() else {
            NSSound.beep()
            return
        }
        guard FileManager.default.fileExists(atPath: closed.path) else {
            // Gone from disk since it was closed. Drop it and try the one
            // before it rather than beeping at a file the user cannot see.
            reopenClosedDocument(sender)
            return
        }
        open(path: closed.path)
        editors.first { $0.coreDocument.path == closed.path }?
            .restoreSessionPosition(caret: closed.caret, scroll: closed.scroll)
    }

    /// Reopens the saved session's files and positions.
    private func restoreSession() {
        guard let state = SessionStore.load() else { return }
        if let split = state.sidebarSplit {
            fileTreeState.splitFraction = min(0.85, max(0.15, split))
        }
        var frontmostEditor: DocumentController?
        // Each saved window comes back as a window, with its files as
        // tabs; a session from before windows were recorded comes back
        // as one window of everything.
        let layout =
            state.layout?.filter { group in
                group.tabs.contains { FileManager.default.fileExists(atPath: $0) }
            }
            ?? [SessionState.Layout(tabs: state.windows.map(\.path), panes: [])]
        for group in layout {
            var workbench: Workbench?
            for path in group.tabs where FileManager.default.fileExists(atPath: path) {
                open(path: path, target: workbench == nil ? .window : .tab)
                guard let editor = editors.first(where: { $0.coreDocument.path == path })
                else { continue }
                workbench = editor.workbench
                if let saved = state.windows.first(where: { $0.path == path }) {
                    editor.restoreSessionPosition(caret: saved.caret, scroll: saved.scroll)
                }
                if path == state.frontmost {
                    frontmostEditor = editor
                }
            }
            // The columns come back showing what they were showing,
            // each with the views it had. A session written before
            // windows held columns names one file per pane.
            let saved =
                group.columns
                ?? group.panes.map { SessionState.ColumnLayout(file: $0) }
            if let workbench, saved.count > 1 {
                for (index, column) in saved.enumerated() {
                    if index > 0 { workbench.newColumn() }
                    guard
                        let editor = workbench.documents.first(where: {
                            $0.coreDocument.path == column.file
                        })
                    else { continue }
                    workbench.show(editor, inColumn: index)
                    workbench.restore(
                        column: index, views: column.views, dividers: column.dividers)
                }
                workbench.focus(column: 0)
            } else if let workbench, let only = saved.first {
                workbench.restore(column: 0, views: only.views, dividers: only.dividers)
            }
        }
        frontmostEditor?.workbench?.window?.makeKeyAndOrderFront(nil)
        if let frontmostEditor, let workbench = frontmostEditor.workbench {
            workbench.showInFocusedPane(ObjectIdentifier(frontmostEditor))
        }
    }

    /// Quitting reviews every dirty window through the same save/discard
    /// flow as closing it by hand, then records the session with the
    /// freshest positions.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        saveSession()
        isTerminating = true
        for editor in editors {
            if !editor.mayClose() {
                // The user changed their mind; windows stay open and
                // ordinary saves resume.
                isTerminating = false
                return .terminateCancel
            }
        }
        // Files that outlive their windows are settled here, which is
        // the last place they can be.
        if !settleFilesPutAside() {
            isTerminating = false
            return .terminateCancel
        }
        return .terminateNow
    }

    /// Asks about the files with changes that were never saved and
    /// no window left to ask through — the ones set to outlive their
    /// windows. Answers whether quitting may go ahead.
    private func settleFilesPutAside() -> Bool {
        let unsaved = DocumentStore.shared.unsaved
        if unsaved.isEmpty { return true }
        let names = unsaved.map { document in
            (document.path ?? document.core.path)
                .map { ($0 as NSString).lastPathComponent } ?? n_("Untitled")
        }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText =
            names.count == 1
            ? t("Do you want to save the changes made to {}?", names[0])
            : tn(
                "Do you want to save the changes made to {} file?",
                "Do you want to save the changes made to {} files?", names.count)
        alert.informativeText = names.joined(separator: ", ")
        alert.addButton(withTitle: t("Save All"))
        alert.addButton(withTitle: t("Cancel"))
        alert.addButton(withTitle: t("Don’t Save"))
        switch alert.runModal() {
        case .alertFirstButtonReturn:
            var homeless = 0
            for document in unsaved {
                // Nowhere to write it: quitting now would be the
                // discard nobody asked for.
                guard document.core.path != nil else {
                    homeless += 1
                    continue
                }
                try? document.core.save()
            }
            return homeless == 0
        case .alertThirdButtonReturn:
            return true
        default:
            return false
        }
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
            noteServerStatus(server: server, root: root, status: status, message: message)
            if status == "not-found", !reportedMissingServers.contains(server) {
                reportedMissingServers.insert(server)
                let alert = NSAlert()
                alert.alertStyle = .informational
                alert.messageText = t("No language server for this project")
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
                alert.messageText = t("Language server failed to start")
                alert.informativeText =
                    "\(server) exited during startup"
                    + (message.isEmpty || message == "during initialize"
                        ? "" : " (\(message))")
                    + ". Its own error output is in \(AppPaths.logFileForDisplay)."
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

    @objc func toggleHoverDocs(_ sender: Any?) {
        settingsModel?.hoverDocs.toggle()
    }

    // MARK: Configurable key shortcuts

    // MARK: About

    /// The standard About panel, with content worth reading: the real
    /// build version (git-described for local builds, the tag for
    /// releases), the author with their site, the repository, and the
    /// license — all clickable.
    /// ⌘1…⌘9: the tab by its place on the bar.
    @objc func selectTabByNumber(_ sender: Any?) {
        guard let item = sender as? NSMenuItem,
            let workbench = Workbench.all.first(where: { $0.window?.isKeyWindow == true })
        else { return }
        workbench.selectTab(number: item.tag)
    }

    @objc func showAbout(_ sender: Any?) {
        let version =
            Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
            ?? "development build"
        let credits = NSMutableAttributedString()
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        paragraph.paragraphSpacing = 4
        let base: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 11),
            .paragraphStyle: paragraph,
            .foregroundColor: NSColor.labelColor,
        ]
        func line(_ text: String, link: String? = nil, prefix: String = "", suffix: String = "\n")
        {
            if !prefix.isEmpty {
                credits.append(NSAttributedString(string: prefix, attributes: base))
            }
            var attributes = base
            if let link { attributes[.link] = link }
            credits.append(NSAttributedString(string: text, attributes: attributes))
            credits.append(NSAttributedString(string: suffix, attributes: base))
        }
        line("A text editor in the spirit of TextMate:")
        line("native, fast, and focused on editing.")
        line("")
        line("Horacio Duran", link: "https://perri.to", prefix: "By ", suffix: " · ")
        line("perri.to", link: "https://perri.to")
        line(
            "github.com/perrito666/textchum",
            link: "https://github.com/perrito666/textchum", prefix: "Source: ")
        line(
            "MIT license",
            link: "https://github.com/perrito666/textchum/blob/main/LICENSE")
        line(
            "Juan Diaz", link: "https://github.com/nueces",
            prefix: "With contributions from ")
        NSApp.orderFrontStandardAboutPanel(options: [
            .applicationName: "Textchum",
            .applicationVersion: version,
            .version: "core " + Core.version,
            .credits: credits,
        ])
    }

    // MARK: Server status

    /// The last hundred server status transitions, oldest first.
    private var serverStatusLog: [(at: Date, server: String, root: String, line: String)] = []
    private var statusPanel: NSPanel?
    private var statusRefreshTimer: Timer?

    private func noteServerStatus(server: String, root: String, status: String, message: String)
    {
        let line = message.isEmpty ? status : "\(status) — \(message)"
        serverStatusLog.append((Date(), server, root, line))
        if serverStatusLog.count > 100 {
            serverStatusLog.removeFirst(serverStatusLog.count - 100)
        }
        if statusPanel?.isVisible == true {
            refreshStatusPanel()
        }
    }

    /// View → Language Server Status: what runs where, and the recent
    /// transitions — the at-a-glance answer to "is my server alive?".
    @objc func showServerStatus(_ sender: Any?) {
        if statusPanel == nil {
            let panel = NSPanel(
                contentRect: NSRect(x: 0, y: 0, width: 560, height: 360),
                styleMask: [.titled, .closable, .resizable, .utilityWindow],
                backing: .buffered,
                defer: false
            )
            panel.title = "Language Server Status"
            panel.isReleasedWhenClosed = false
            let scroll = NSTextView.scrollableTextView()
            let text = scroll.documentView as! NSTextView
            text.isEditable = false
            text.font = .monospacedSystemFont(ofSize: 11.5, weight: .regular)
            text.textContainerInset = NSSize(width: 8, height: 8)
            panel.contentView = scroll
            panel.center()
            statusPanel = panel
        }
        refreshStatusPanel()
        statusPanel?.makeKeyAndOrderFront(nil)
        statusRefreshTimer?.invalidate()
        statusRefreshTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) {
            [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    guard let self else { return }
                    if self.statusPanel?.isVisible == true {
                        self.refreshStatusPanel()
                    } else {
                        self.statusRefreshTimer?.invalidate()
                        self.statusRefreshTimer = nil
                    }
                }
            }
        }
    }

    private func refreshStatusPanel() {
        guard let scroll = statusPanel?.contentView as? NSScrollView,
            let text = scroll.documentView as? NSTextView
        else { return }
        var lines: [String] = []
        let running = coreApp?.lspRunning() ?? []
        lines.append("Running instances (\(running.count)):")
        if running.isEmpty {
            lines.append("  none — servers start when a matching document opens")
        }
        for instance in running.sorted(by: { $0.root < $1.root }) {
            lines.append("  \(instance.server)  \(abbreviate(instance.root))")
        }
        lines.append("")
        lines.append("Recent transitions:")
        if serverStatusLog.isEmpty {
            lines.append("  none this session")
        }
        let clock = DateFormatter()
        clock.dateFormat = "HH:mm:ss"
        for entry in serverStatusLog.suffix(30).reversed() {
            lines.append(
                "  \(clock.string(from: entry.at))  \(entry.server) [\(abbreviate(entry.root))]: \(entry.line)"
            )
        }
        lines.append("")
        lines.append("Full trail: \(AppPaths.logFileForDisplay)")
        text.string = lines.joined(separator: "\n")
    }

    private func abbreviate(_ path: String) -> String {
        (path as NSString).abbreviatingWithTildeInPath
    }

    // MARK: Configuration file watching

    /// When the app itself last wrote config.json; the watcher ignores
    /// the echo of our own saves.
    private var lastOwnConfigSave = Date.distantPast
    private var configWatcher: DispatchSourceFileSystemObject?

    /// Follows external edits to config.json while running: the file is
    /// reloaded wholesale (hand edits and unknown keys intact) and the
    /// same pipeline a Settings change runs re-applies everything.
    private func startWatchingConfig() {
        configWatcher?.cancel()
        configWatcher = nil
        let descriptor = Darwin.open(Self.configPath, O_EVTONLY)
        guard descriptor >= 0 else {
            // Not created yet; try again once something exists.
            DispatchQueue.main.asyncAfter(deadline: .now() + 5) { [weak self] in
                MainActor.assumeIsolated { self?.startWatchingConfig() }
            }
            return
        }
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .extend, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            self?.configDidChangeOnDisk()
        }
        source.setCancelHandler {
            _ = Darwin.close(descriptor)
        }
        source.resume()
        configWatcher = source
    }

    private func configDidChangeOnDisk() {
        // Atomic saves replace the file; rewatch the path first.
        startWatchingConfig()
        guard Date().timeIntervalSince(lastOwnConfigSave) > 2 else { return }
        guard let config else { return }
        if let warning = config.reload() {
            NSLog("config reload: \(warning)")
        }
        // Same pipeline as a Settings change — plus re-publishing the
        // Settings window's own fields.
        settingsModel?.reloadFromConfig()
        applyKeyOverrides()
        applyAppearanceChoice()
        applyThemeChoice()
        applyIconPack()
        coreApp?.lspConfigure(json: combinedLSPConfiguration)
        for editor in editors {
            editor.refreshProjectRoot()
        }
        if let model = settingsModel {
            for editor in editors {
                editor.apply(settings: model.currentSettings(forRoot: editor.projectRoot))
            }
        }
    }

    // MARK: What a document has been told it is

    func fileOverride(path: String) -> CoreConfig.FileOverride {
        config?.fileOverride(path: path) ?? CoreConfig.FileOverride()
    }

    /// Records a document's own settings and writes them out. Saved
    /// straight away: the panel has no OK button, so a change is made
    /// the moment it is chosen.
    func setFileOverride(path: String, _ entry: CoreConfig.FileOverride) {
        guard let config else { return }
        config.setFileOverride(path: path, entry)
        lastOwnConfigSave = Date()
        try? config.save()
    }

    /// Adds a word to the personal dictionary from an editor's spelling
    /// menu, then re-applies settings so every open window stops
    /// flagging it. The list is a setting like any other, so it goes
    /// through the settings model rather than straight to the file —
    /// otherwise the Settings window would show a stale list.
    func addSpellWord(_ word: String) {
        guard let config, let model = settingsModel else { return }
        guard config.addSpellWord(word) else { return }
        lastOwnConfigSave = Date()
        try? config.save()
        model.reloadFromConfig()
        for editor in editors {
            editor.apply(settings: model.currentSettings(forRoot: editor.projectRoot))
        }
    }

    // MARK: Command-click navigation

    /// ⌘-click jumps to the definition under the pointer — the caret
    /// moves to the click first, so the jump stack records where the
    /// mouse actually was.
    private var commandClickMonitor: Any?

    private func installCommandClickMonitor() {
        commandClickMonitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) {
            [weak self] event in
            guard event.modifierFlags.contains(.command),
                !event.modifierFlags.contains(.shift),
                let self,
                let editor = self.editors.first(where: { $0.window == event.window }),
                let textView = editor.editorTextView,
                let contentView = event.window?.contentView,
                textView.isDescendant(of: contentView)
            else { return event }
            let point = textView.convert(event.locationInWindow, from: nil)
            guard textView.bounds.contains(point) else { return event }
            let index = textView.characterIndexForInsertion(at: point)
            guard index >= 0, index <= (textView.string as NSString).length else {
                return event
            }
            textView.setSelectedRange(NSRange(location: index, length: 0))
            self.recordJumpOrigin()
            editor.jumpToDefinition(nil)
            return nil
        }
    }

    /// Menu items by their stable action name, for `keys` overrides.
    private var menuActions: [String: NSMenuItem] = [:]

    /// What each of those items came with. A profile names the
    /// shortcuts it moves and nothing else, so leaving one — or
    /// dropping an override — has to give the original back, and the
    /// only place it exists is here.
    private var defaultShortcuts: [String: (String, NSEvent.ModifierFlags)] = [:]

    /// Indexes every overridable menu item by a stable name.
    private func registerMenuActions(in menu: NSMenu) {
        let bySelector: [Selector: String] = [
            #selector(newDocument(_:)): "new",
            #selector(newDocumentWithFormatPicker(_:)): "newWithFormat",
            #selector(openDocument(_:)): "open",
            #selector(openQuickly(_:)): "openQuickly",
            #selector(DocumentController.saveDocument(_:)): "save",
            #selector(DocumentController.saveDocumentAs(_:)): "saveAs",
            #selector(DocumentController.revertToSaved(_:)): "revertToSaved",
            #selector(reopenClosedDocument(_:)): "reopenClosed",
            #selector(DocumentController.showFileProperties(_:)): "fileProperties",
            #selector(DocumentController.performUndo(_:)): "undo",
            #selector(DocumentController.performRedo(_:)): "redo",
            #selector(DocumentController.jumpToDefinition(_:)): "jumpToDefinition",
            #selector(goBack(_:)): "goBack",
            #selector(goForward(_:)): "goForward",
            #selector(DocumentController.findReferences(_:)): "findReferences",
            #selector(DocumentController.showCodeActions(_:)): "codeActions",
            #selector(DocumentController.newColumn(_:)): "newColumn",
            #selector(DocumentController.closeColumn(_:)): "closeColumn",
            #selector(DocumentController.addView(_:)): "secondView",
            #selector(DocumentController.closeView(_:)): "closeView",
            #selector(DocumentController.focusOtherSide(_:)): "nextPane",
            #selector(DocumentController.closeTab(_:)): "close",
            #selector(DocumentController.toggleFold(_:)): "fold",
            #selector(DocumentController.foldAll(_:)): "foldAll",
            #selector(DocumentController.unfoldAll(_:)): "unfoldAll",
            #selector(DocumentController.selectNextTab(_:)): "nextTab",
            #selector(DocumentController.selectPreviousTab(_:)): "previousTab",
            #selector(DocumentController.showInEveryPane(_:)): "sameFileEveryColumn",
            #selector(DocumentController.moveTabToNewWindow(_:)): "moveTabToNewWindow",
            #selector(DocumentController.renameSymbol(_:)): "renameSymbol",
            #selector(DocumentController.formatDocument(_:)): "formatDocument",
            #selector(DocumentController.runPreprocessors(_:)): "runPreprocessors",
            #selector(DocumentController.blameLine(_:)): "blameLine",
            #selector(DocumentController.showDiagnosticAtCaret(_:)): "showDiagnostic",
            #selector(DocumentController.showDiagnosticList(_:)): "diagnosticList",
            #selector(DocumentController.goToLine(_:)): "goToLine",
            #selector(DocumentController.goToBlockStart(_:)): "goToBlockStart",
            #selector(DocumentController.goToBlockEnd(_:)): "goToBlockEnd",
            #selector(DocumentController.triggerCompletion(_:)): "complete",
            #selector(findInProject(_:)): "findInProject",
            #selector(NSSplitViewController.toggleSidebar(_:)): "toggleNavigator",
            #selector(DocumentController.togglePreview(_:)): "togglePreview",
            #selector(toggleLineNumbers(_:)): "toggleLineNumbers",
            #selector(toggleHoverDocs(_:)): "toggleHover",
            #selector(DocumentController.showHoverAtCaret(_:)): "showHover",
            #selector(togglePathDisplay(_:)): "togglePathDisplay",
            #selector(DocumentController.redrawDocument(_:)): "redraw",
            #selector(DocumentController.showDocumentOutline(_:)): "documentOutline",
            #selector(DocumentController.revealInTree(_:)): "revealInTree",
            #selector(showCommandPalette(_:)): "commandPalette",
            #selector(showServerStatus(_:)): "serverStatus",
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
            if let name = menuActions.first(where: { $0.value === item })?.key,
                defaultShortcuts[name] == nil
            {
                defaultShortcuts[name] = (item.keyEquivalent, item.keyEquivalentModifierMask)
            }
        }
    }

    /// Applies the shortcuts that are in force: the chosen profile's,
    /// with the `keys` overrides on top.
    ///
    /// Every registered item goes back to what it came with first —
    /// otherwise leaving a profile, or removing one override, would
    /// leave its shortcut behind with nothing naming it. Unknown
    /// actions and unparseable shortcuts are logged, never fatal.
    private func applyKeyOverrides() {
        guard let config else { return }
        for (action, item) in menuActions {
            guard let (key, modifiers) = defaultShortcuts[action] else { continue }
            item.keyEquivalent = key
            item.keyEquivalentModifierMask = modifiers
        }
        let overrides = config.effectiveKeys
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
                // Function keys: profiles from other editors lean on
                // them (F12 for a definition, F2 for a rename).
                if token.hasPrefix("f"), let number = Int(token.dropFirst()),
                    (1...20).contains(number),
                    let scalar = UnicodeScalar(NSF1FunctionKey + number - 1)
                {
                    key = String(scalar)
                    continue
                }
                // Punctuation by name, the way a keymap from another
                // editor spells it: `cmd+period`, not `cmd+.`.
                let named: [String: String] = [
                    "period": ".", "comma": ",", "semicolon": ";", "slash": "/",
                    "backslash": "\\", "bracketleft": "[", "bracketright": "]",
                    "grave": "`", "minus": "-", "equal": "=", "apostrophe": "'",
                ]
                if let punctuation = named[token] {
                    key = punctuation
                    continue
                }
                guard token.count == 1 else { return nil }
                key = token
            }
        }
        guard let key else { return nil }
        return (key, modifiers)
    }

    /// A menu item's shortcut as a spec the configuration can hold —
    /// the inverse of `parseShortcut`. Empty when the item has none.
    static func shortcutSpec(key: String, modifiers: NSEvent.ModifierFlags) -> String {
        guard !key.isEmpty else { return "" }
        var parts: [String] = []
        if modifiers.contains(.command) { parts.append("cmd") }
        if modifiers.contains(.control) { parts.append("ctrl") }
        if modifiers.contains(.option) { parts.append("alt") }
        if modifiers.contains(.shift) { parts.append("shift") }
        let named: [String: String] = [
            String(UnicodeScalar(NSUpArrowFunctionKey)!): "up",
            String(UnicodeScalar(NSDownArrowFunctionKey)!): "down",
            String(UnicodeScalar(NSLeftArrowFunctionKey)!): "left",
            String(UnicodeScalar(NSRightArrowFunctionKey)!): "right",
            "\r": "return",
            "\u{1b}": "escape",
            " ": "space",
            "\t": "tab",
            "\u{8}": "delete",
        ]
        if let scalar = key.unicodeScalars.first,
            (NSF1FunctionKey...NSF1FunctionKey + 19).contains(Int(scalar.value))
        {
            parts.append("f\(Int(scalar.value) - NSF1FunctionKey + 1)")
        } else {
            parts.append(named[key] ?? key.lowercased())
        }
        return parts.joined(separator: "+")
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
    /// The icon pack currently loaded, so a configuration reload that
    /// did not touch it does not reload it.
    private var appliedIconPack: String?
    private var warnedBrokenIconPack = false
    /// The theme name currently applied, to skip redundant recolors.
    private var appliedTheme: String?

    /// Chooses `name` as the theme and puts it on, the way the
    /// Settings picker does. The configuration watcher ignores this
    /// process's own writes, so the applying is done here rather than
    /// waited for.
    func selectTheme(named name: String) {
        guard let config else { return }
        config.theme = name
        do {
            try config.save()
        } catch {
            NSLog("could not save the theme choice: \(error)")
        }
        applyThemeChoice()
        applyIconPack()
    }

    /// Loads the configured file-icon pack, or clears it. A pack that
    /// cannot be read is reported once and the tree keeps the desktop's
    /// icons — the same escape hatch a broken theme gets, and for the
    /// same reason: a pack someone moved should not stop the editor.
    private func applyIconPack() {
        let chosen = config?.iconPack
        guard chosen != appliedIconPack else { return }
        appliedIconPack = chosen
        guard let chosen else {
            CoreIcons.clear()
            refreshFileIcons()
            return
        }
        do {
            _ = try CoreIcons.load(at: chosen)
        } catch {
            CoreIcons.clear()
            if !warnedBrokenIconPack {
                warnedBrokenIconPack = true
                let alert = NSAlert()
                alert.alertStyle = .warning
                alert.messageText = t("The icon pack could not be used")
                alert.informativeText =
                    "\(error) — the file tree keeps the system's icons."
                alert.runModal()
            }
        }
        refreshFileIcons()
    }

    /// Redraws every navigator, so a pack put on or taken off shows
    /// without reopening a window. The rows read the pack as they draw,
    /// so telling the model it changed is the whole of it.
    private func refreshFileIcons() {
        for editor in editors {
            editor.workbench?.sidebarModel.objectWillChange.send()
        }
    }

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
        for workbench in Workbench.all {
            let entries = workbench.documents.map { peer in
                (
                    document: SidebarDocument(
                        id: ObjectIdentifier(peer),
                        title: peer.chromeTitle,
                        path: peer.coreDocument.path,
                        isDirty: peer.coreDocument.isDirty
                    ),
                    projectRoot: peer.projectRoot
                )
            }
            workbench.sidebarModel.rebuild(entries: entries)
            workbench.refreshTabs()
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

    /// The format picker's list. The app owns this one; each editor
    /// window owns its own for the lists that are about a document.
    private let formatPicker = ListPanel()

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

    /// "Split into New Window": gathers the group's files into a
    /// window of their own.
    private func splitIntoNewWindow(documentIDs: [ObjectIdentifier]) {
        let members = editors.filter { documentIDs.contains(ObjectIdentifier($0)) }
        guard !members.isEmpty else { return }
        let workbench = makeWorkbench()
        for member in members {
            member.workbench?.detach(member)
            workbench.add(member)
        }
        workbench.showWindow(nil)
        workbench.window?.makeKeyAndOrderFront(nil)
        NotificationCenter.default.post(name: .textchumDocumentsChanged, object: nil)
    }

    /// "Gather Into …": moves the group's files into the chosen window
    /// as tabs. A file already there is left where it is.
    private func mergeAsTabs(documentIDs: [ObjectIdentifier], into target: ObjectIdentifier) {
        guard let workbench = Workbench.all.first(where: { ObjectIdentifier($0) == target })
        else { return }
        let members = editors.filter { documentIDs.contains(ObjectIdentifier($0)) }
        for member in members where member.workbench !== workbench {
            member.workbench?.detach(member)
            workbench.add(member)
        }
        workbench.window?.makeKeyAndOrderFront(nil)
        NotificationCenter.default.post(name: .textchumDocumentsChanged, object: nil)
    }

    /// One "Gather Into" destination per window, the asking window
    /// first as "This Window".
    private func windowTargets(asking host: ObjectIdentifier) -> [WindowTarget] {
        var targets: [WindowTarget] = []
        for workbench in Workbench.all {
            let extra = workbench.documents.count > 1 ? " (+\(workbench.documents.count - 1))" : ""
            let isHost = ObjectIdentifier(workbench) == host
            let title =
                isHost
                ? t("This Window")
                : (workbench.focusedDocument?.chromeTitle ?? "Window") + extra
            let target = WindowTarget(id: ObjectIdentifier(workbench), title: title)
            if isHost {
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
            preprocessorCommands: { [weak self] root, language in
                self?.config?.preprocessorCommands(root: root, language: language) ?? []
            },
            hiddenGlobs: { [weak self] root in
                self?.config?.hiddenGlobs(root: root) ?? [".*"]
            },
            revealInTree: { [weak self] path in
                guard let self else { return }
                let root = CoreWorkspace.projectRoot(
                    forPath: path, settingsJSON: self.config?.workspaceJSON ?? "{}")
                guard let root else { return }
                self.fileTreeState.reveal(path: path, under: root)
            },
            followEnabled: { [weak self] in self?.config?.followFile ?? true },
            selectDocument: { [weak self] id in
                guard let editor = self?.editors.first(where: { ObjectIdentifier($0) == id })
                else { return }
                editor.window?.makeKeyAndOrderFront(nil)
            },
            showProperties: { [weak self] id in
                guard let editor = self?.editors.first(where: { ObjectIdentifier($0) == id })
                else { return }
                editor.window?.makeKeyAndOrderFront(nil)
                editor.showFileProperties(nil)
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
            Workbench.all.first { $0.window == NSApp.keyWindow }?.sidebarModel
            ?? Workbench.all.first?.sidebarModel
        let newValue = !(active?.showFullPaths ?? false)
        for workbench in Workbench.all {
            workbench.sidebarModel.showFullPaths = newValue
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
        // Any open document's project beats the home directory: an
        // untitled window in front used to send the finder walking
        // every file the user owns, which is slow and ranks nonsense.
        if let root = editors.compactMap(\.projectRoot).first { return root }
        if let path = editors.compactMap(\.coreDocument.path).first {
            return (path as NSString).deletingLastPathComponent
        }
        return QuickFinderPanel.lastScope ?? NSHomeDirectory()
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
            let empty = NSMenuItem(title: t("No Recent Files"), action: nil, keyEquivalent: "")
            empty.isEnabled = false
            menu.addItem(empty)
        } else {
            menu.addItem(.separator())
            let clear = NSMenuItem(
                title: t("Clear Menu"),
                action: #selector(clearRecentDocuments(_:)),
                keyEquivalent: ""
            )
            clear.target = self
            menu.addItem(clear)
        }
    }

    // MARK: Settings

    /// Puts the shortcuts in force into the Keyboard tab. Only the app
    /// knows which actions exist and what they are called on screen,
    /// and the menu items carry whatever the profile last set.
    func refreshShortcutCatalog() {
        guard let settingsModel else { return }
        settingsModel.shortcutCatalog = menuActions
            .map { action, item in
                SettingsModel.Shortcut(
                    action: action,
                    title: item.title,
                    spec: Self.shortcutSpec(
                        key: item.keyEquivalent,
                        modifiers: item.keyEquivalentModifierMask))
            }
            .sorted { $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending }
    }

    @objc func showSettings(_ sender: Any?) {
        guard let settingsModel else { return }
        // The Projects tab offers the open projects to add; only the
        // app knows what they are, and only now does it matter.
        settingsModel.openProjectRoots = Array(
            Set(editors.compactMap(\.projectRoot))
        ).sorted()
        refreshShortcutCatalog()
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
        let editor = DocumentController(
            document: document,
            settings: currentSettings,
            sidebar: sidebarConfiguration,
            lspApp: coreApp,
            openLocation: { [weak self] path, line, character in
                self?.openLocation(path: path, line: line, character: character)
            })
        editor.suggestedSaveDirectory = suggestedDirectory
        // Fresh documents follow their own placement setting: a tab of
        // the frontmost group by default, a window when configured so.
        show(editor: editor, placeAsConfigured: true, target: config?.newFileTarget)
    }

    /// File → New with Format… (⇧⌘N): the language list as a
    /// fuzzy-filterable panel, for keyboards. ⏎ creates the untitled
    /// document already speaking the selection.
    @objc func newDocumentWithFormatPicker(_ sender: Any?) {
        let languages = CoreLanguages.all.map { language in
            OutlineSymbol(
                name: language.name,
                kind: language.fileExtension.isEmpty ? "" : ".\(language.fileExtension)",
                line: 0,
                character: 0,
                depth: 0
            )
        }
        formatPicker.show(
            rows: languages.map { .item($0.name) }, over: NSApp.keyWindow,
            title: t("New with Format"), placeholder: t("language…")
        ) { [weak self] index in
            guard languages.indices.contains(index) else { return }
            self?.makeUntitled(language: languages[index].name)
        }
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
            editor.workbench?.closeTab(ObjectIdentifier(editor))
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
            // A file closed a moment ago is still in the store, whole:
            // opening it again is taking the closing back, so what was
            // typed and never saved comes back with it.
            let document = try DocumentStore.shared.reclaim(path: path)?.core
                ?? CoreDocument(contentsOf: path)
            closeUntouchedUntitledWindows()
            noteRecent(path: path)
            let editor = DocumentController(
                document: document,
                settings: currentSettings,
                sidebar: sidebarConfiguration,
                lspApp: coreApp,
                openLocation: { [weak self] path, line, character in
                    self?.openLocation(path: path, line: line, character: character)
                }
            )
            show(editor: editor, placeAsConfigured: true, target: target)
            // The window was built with the global settings; its project
            // root is known now, so any per-root overrides apply.
            if let model = settingsModel, editor.projectRoot != nil {
                editor.apply(settings: model.currentSettings(forRoot: editor.projectRoot))
            }
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
    /// A new window, wired to the application's navigator.
    func makeWorkbench() -> Workbench {
        let workbench = Workbench(sidebar: sidebarConfiguration)
        workbench.onDocumentClosed = { [weak self] document in
            MainActor.assumeIsolated {
                self?.releaseChumWait(for: document)
                self?.noteClosedEditor(document)
                self?.editors.removeAll { $0 === document }
                self?.rebuildSidebar()
                if self?.isTerminating != true {
                    self?.saveSession()
                }
            }
        }
        return workbench
    }

    /// The window a newly opened file goes to: the front one when files
    /// open as tabs, a new one otherwise.
    private func workbench(for target: CoreOpenTarget?) -> Workbench {
        let wanted = target ?? config?.openTarget ?? .tab
        if wanted == .tab {
            if let key = Workbench.all.first(where: { $0.window?.isKeyWindow == true }) {
                return key
            }
            if let any = Workbench.all.last {
                return any
            }
        }
        return makeWorkbench()
    }

    private func show(
        editor: DocumentController,
        placeAsConfigured: Bool = false,
        target: CoreOpenTarget? = nil
    ) {
        let workbench = placeAsConfigured ? self.workbench(for: target) : self.workbench(for: .tab)
        // A document told what it is stays told: reopening a .txt that
        // holds SQL should not find it plain text again.
        if let path = editor.coreDocument.path {
            let stored = fileOverride(path: path)
            if !stored.isEmpty {
                editor.applyFileProperties(
                    .init(
                        language: stored.language,
                        tabWidth: stored.tabWidth,
                        spaces: stored.spaces
                    ))
            }
        }
        editors.append(editor)
        workbench.add(editor)
        workbench.showWindow(nil)
        workbench.window?.makeKeyAndOrderFront(nil)
        workbench.focus(column: workbench.focusedColumn, view: workbench.focusedView)
        // No direct rebuild here: the controller publishes its state (via
        // updateChrome) and the deferred notification handler rebuilds —
        // rebuilding synchronously mid-presentation trips NSTableView.
    }

    // MARK: Menu

    /// Programmatic main menu: the app has no nib. Undo/redo use dedicated
    /// selectors handled by the editor window controller, since document
    /// history lives in the core rather than in an `NSUndoManager`.
    /// Edit ▸ Transform: what to do to the selection, or to the whole
    /// document when nothing is selected.
    ///
    /// AppKit has a Transformations submenu of its own, but it appears
    /// only in the context menu the editor no longer uses, and it knows
    /// nothing about lines. These go through the core, so both shells
    /// agree on what sorting and joining mean.
    private func makeTransformMenuItem() -> NSMenuItem {
        let menu = NSMenu(title: t("Transform"))
        let groups: [[(String, String)]] = [
            [
                (n_("Upper Case"), "upper"),
                (n_("Lower Case"), "lower"),
                (n_("Title Case"), "title"),
                (n_("Invert Case"), "invert"),
            ],
            [
                (n_("Sort Lines"), "sort"),
                (n_("Sort Lines Reversed"), "sort-reversed"),
                (n_("Remove Duplicate Lines"), "dedupe"),
                (n_("Join Lines"), "join"),
                (n_("Trim Trailing Whitespace"), "trim"),
            ],
            [
                (n_("Convert to Unix Line Endings (LF)"), "lf"),
                (n_("Convert to Windows Line Endings (CRLF)"), "crlf"),
            ],
        ]
        for (at, group) in groups.enumerated() {
            if at > 0 { menu.addItem(.separator()) }
            for (title, kind) in group {
                let item = NSMenuItem(
                    title: t(title),
                    action: #selector(DocumentController.transformText(_:)),
                    keyEquivalent: "")
                item.representedObject = kind
                menu.addItem(item)
            }
        }
        let item = NSMenuItem(title: t("Transform"), action: nil, keyEquivalent: "")
        item.submenu = menu
        return item
    }

    private func makeMainMenu() -> NSMenu {
        let mainMenu = NSMenu()

        let appMenu = NSMenu()
        appMenu.addItem(
            withTitle: t("About Textchum"),
            action: #selector(showAbout(_:)),
            keyEquivalent: ""
        )
        appMenu.addItem(.separator())
        appMenu.addItem(
            withTitle: t("Settings…"),
            action: #selector(showSettings(_:)),
            keyEquivalent: ","
        )
        appMenu.addItem(
            withTitle: t("Install chum Command…"),
            action: #selector(installCommandLineTool(_:)),
            keyEquivalent: ""
        )
        appMenu.addItem(
            withTitle: t("Open Themes Folder"),
            action: #selector(openThemesFolder(_:)),
            keyEquivalent: ""
        )
        let importThemeItem = NSMenuItem(title: t("Import Theme"), action: nil, keyEquivalent: "")
        let importThemeMenu = NSMenu(title: t("Import Theme"))
        importThemeMenu.addItem(
            withTitle: t("From VS Code…"),
            action: #selector(importVSCodeTheme(_:)),
            keyEquivalent: ""
        )
        importThemeMenu.addItem(
            withTitle: t("From TextMate…"),
            action: #selector(importTextMateTheme(_:)),
            keyEquivalent: ""
        )
        importThemeItem.submenu = importThemeMenu
        appMenu.addItem(importThemeItem)
        appMenu.addItem(.separator())
        appMenu.addItem(
            withTitle: t("Quit Textchum"),
            action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q"
        )
        let appMenuItem = NSMenuItem()
        appMenuItem.submenu = appMenu
        mainMenu.addItem(appMenuItem)

        let fileMenu = NSMenu(title: t("File"))
        fileMenu.addItem(
            withTitle: t("New"), action: #selector(newDocument(_:)), keyEquivalent: "n")
        let formatPickerItem = NSMenuItem(
            title: t("New with Format…"),
            action: #selector(newDocumentWithFormatPicker(_:)),
            keyEquivalent: "n"
        )
        formatPickerItem.keyEquivalentModifierMask = [.command, .shift]
        fileMenu.addItem(formatPickerItem)
        let formatItem = NSMenuItem(title: t("New with Format"), action: nil, keyEquivalent: "")
        let formatMenu = NSMenu(title: t("New with Format"))
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
            withTitle: t("Open…"), action: #selector(openDocument(_:)), keyEquivalent: "o")
        fileMenu.addItem(
            withTitle: t("Open Quickly…"), action: #selector(openQuickly(_:)), keyEquivalent: "t")
        let openRecentItem = NSMenuItem(title: t("Open Recent"), action: nil, keyEquivalent: "")
        let openRecent = NSMenu(title: t("Open Recent"))
        openRecent.delegate = self
        openRecentItem.submenu = openRecent
        self.openRecentMenu = openRecent
        fileMenu.addItem(openRecentItem)
        fileMenu.addItem(.separator())
        fileMenu.addItem(
            withTitle: t("Close Tab"),
            action: #selector(DocumentController.closeTab(_:)),
            keyEquivalent: "w")
        let closeWindowItem = NSMenuItem(
            title: t("Close Window"),
            action: #selector(NSWindow.performClose(_:)),
            keyEquivalent: "w")
        closeWindowItem.keyEquivalentModifierMask = [.command, .shift]
        fileMenu.addItem(closeWindowItem)
        let properties = NSMenuItem(
            title: t("Get Info"),
            action: #selector(DocumentController.showFileProperties(_:)),
            keyEquivalent: "i"
        )
        fileMenu.addItem(properties)
        let reopen = NSMenuItem(
            title: t("Reopen Closed Tab"),
            action: #selector(reopenClosedDocument(_:)),
            keyEquivalent: "t"
        )
        reopen.keyEquivalentModifierMask = [.command, .shift]
        reopen.target = self
        fileMenu.addItem(reopen)
        fileMenu.addItem(
            withTitle: t("Save"),
            action: #selector(DocumentController.saveDocument(_:)),
            keyEquivalent: "s"
        )
        let saveAs = NSMenuItem(
            title: t("Save As…"),
            action: #selector(DocumentController.saveDocumentAs(_:)),
            keyEquivalent: "s"
        )
        saveAs.keyEquivalentModifierMask = [.command, .shift]
        fileMenu.addItem(saveAs)
        let revert = NSMenuItem(
            title: t("Revert to Saved"),
            action: #selector(DocumentController.revertToSaved(_:)),
            keyEquivalent: "r"
        )
        revert.keyEquivalentModifierMask = [.command, .option]
        fileMenu.addItem(revert)
        fileMenu.addItem(.separator())
        // The front tab's location in every useful spelling; also on the
        // context menus of buffer-list and file-tree rows.
        let copyPathItem = NSMenuItem(title: t("Copy Path"), action: nil, keyEquivalent: "")
        let copyPath = NSMenu(title: t("Copy Path"))
        copyPath.addItem(
            withTitle: t("File Name"),
            action: #selector(DocumentController.copyFileName(_:)), keyEquivalent: "")
        copyPath.addItem(
            withTitle: t("Relative Path"),
            action: #selector(DocumentController.copyRelativePath(_:)), keyEquivalent: "")
        copyPath.addItem(
            withTitle: t("Absolute Path"),
            action: #selector(DocumentController.copyAbsolutePath(_:)), keyEquivalent: "")
        copyPath.addItem(
            withTitle: t("Forge URL"),
            action: #selector(DocumentController.copyForgeURL(_:)), keyEquivalent: "")
        copyPathItem.submenu = copyPath
        fileMenu.addItem(copyPathItem)
        let fileMenuItem = NSMenuItem()
        fileMenuItem.submenu = fileMenu
        mainMenu.addItem(fileMenuItem)

        let editMenu = NSMenu(title: t("Edit"))
        let undo = NSMenuItem(
            title: t("Undo"),
            action: #selector(DocumentController.performUndo(_:)),
            keyEquivalent: "z"
        )
        editMenu.addItem(undo)
        let redo = NSMenuItem(
            title: t("Redo"),
            action: #selector(DocumentController.performRedo(_:)),
            keyEquivalent: "Z"
        )
        editMenu.addItem(redo)
        editMenu.addItem(.separator())
        editMenu.addItem(
            withTitle: t("Cut"), action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        editMenu.addItem(
            withTitle: t("Copy"), action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        editMenu.addItem(
            withTitle: t("Paste"), action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        editMenu.addItem(
            withTitle: t("Select All"),
            action: #selector(NSText.selectAll(_:)),
            keyEquivalent: "a"
        )
        editMenu.addItem(.separator())
        let jump = NSMenuItem(
            title: t("Jump to Definition"),
            action: #selector(DocumentController.jumpToDefinition(_:)),
            keyEquivalent: "j"
        )
        jump.keyEquivalentModifierMask = [.command, .control]
        editMenu.addItem(jump)
        let backItem = NSMenuItem(
            title: t("Go Back"),
            action: #selector(goBack(_:)),
            keyEquivalent: String(UnicodeScalar(NSLeftArrowFunctionKey)!)
        )
        backItem.keyEquivalentModifierMask = [.control, .command]
        editMenu.addItem(backItem)
        let forwardItem = NSMenuItem(
            title: t("Go Forward"),
            action: #selector(goForward(_:)),
            keyEquivalent: String(UnicodeScalar(NSRightArrowFunctionKey)!)
        )
        forwardItem.keyEquivalentModifierMask = [.control, .command]
        editMenu.addItem(forwardItem)
        let references = NSMenuItem(
            title: t("Find References"),
            action: #selector(DocumentController.findReferences(_:)),
            keyEquivalent: "r"
        )
        references.keyEquivalentModifierMask = [.command, .shift]
        editMenu.addItem(references)
        let codeActionsItem = NSMenuItem(
            title: t("Code Actions…"),
            action: #selector(DocumentController.showCodeActions(_:)),
            keyEquivalent: "."
        )
        codeActionsItem.keyEquivalentModifierMask = [.command]
        editMenu.addItem(codeActionsItem)

        let rename = NSMenuItem(
            title: t("Rename Symbol…"),
            action: #selector(DocumentController.renameSymbol(_:)),
            keyEquivalent: "r"
        )
        rename.keyEquivalentModifierMask = [.command, .control]
        editMenu.addItem(rename)
        let format = NSMenuItem(
            title: t("Format Document"),
            action: #selector(DocumentController.formatDocument(_:)),
            keyEquivalent: "f"
        )
        format.keyEquivalentModifierMask = [.command, .option, .shift]
        editMenu.addItem(format)
        let preprocess = NSMenuItem(
            title: t("Run Save Preprocessors"),
            action: #selector(DocumentController.runPreprocessors(_:)),
            keyEquivalent: "f"
        )
        preprocess.keyEquivalentModifierMask = [.command, .option, .control]
        editMenu.addItem(preprocess)
        let blockStart = NSMenuItem(
            title: t("Go to Block Start"),
            action: #selector(DocumentController.goToBlockStart(_:)),
            keyEquivalent: String(UnicodeScalar(NSUpArrowFunctionKey)!)
        )
        blockStart.keyEquivalentModifierMask = [.control, .option]
        editMenu.addItem(blockStart)
        let blockEnd = NSMenuItem(
            title: t("Go to Block End"),
            action: #selector(DocumentController.goToBlockEnd(_:)),
            keyEquivalent: String(UnicodeScalar(NSDownArrowFunctionKey)!)
        )
        blockEnd.keyEquivalentModifierMask = [.control, .option]
        editMenu.addItem(blockEnd)
        let complete = NSMenuItem(
            title: t("Complete"),
            action: #selector(DocumentController.triggerCompletion(_:)),
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
        let findMenu = NSMenu(title: t("Find"))
        findMenu.addItem(finderItem("Find…", .showFindInterface, "f"))
        findMenu.addItem(
            finderItem("Find and Replace…", .showReplaceInterface, "f", [.command, .option]))
        findMenu.addItem(finderItem("Find Next", .nextMatch, "g"))
        findMenu.addItem(finderItem("Find Previous", .previousMatch, "g", [.command, .shift]))
        findMenu.addItem(finderItem("Use Selection for Find", .setSearchString, "e"))
        findMenu.addItem(.separator())
        let findInProject = NSMenuItem(
            title: t("Find in Project…"),
            action: #selector(findInProject(_:)),
            keyEquivalent: "f"
        )
        findInProject.keyEquivalentModifierMask = [.command, .shift]
        findMenu.addItem(findInProject)
        let findMenuItem = NSMenuItem(title: t("Find"), action: nil, keyEquivalent: "")
        findMenuItem.submenu = findMenu
        editMenu.addItem(findMenuItem)

        editMenu.addItem(.separator())
        let foldItem = NSMenuItem(
            title: t("Fold"),
            action: #selector(DocumentController.toggleFold(_:)),
            keyEquivalent: "[")
        foldItem.keyEquivalentModifierMask = [.command]
        editMenu.addItem(foldItem)
        let foldAllItem = NSMenuItem(
            title: t("Fold All"),
            action: #selector(DocumentController.foldAll(_:)),
            keyEquivalent: "[")
        foldAllItem.keyEquivalentModifierMask = [.command, .option]
        editMenu.addItem(foldAllItem)
        let unfoldItem = NSMenuItem(
            title: t("Unfold All"),
            action: #selector(DocumentController.unfoldAll(_:)),
            keyEquivalent: "]")
        unfoldItem.keyEquivalentModifierMask = [.command]
        editMenu.addItem(unfoldItem)
        editMenu.addItem(.separator())
        let columnItem = NSMenuItem(
            title: t("New Column"),
            action: #selector(DocumentController.newColumn(_:)),
            keyEquivalent: "\\")
        columnItem.keyEquivalentModifierMask = [.command]
        editMenu.addItem(columnItem)
        let closeColumnItem = NSMenuItem(
            title: t("Close Column"),
            action: #selector(DocumentController.closeColumn(_:)),
            keyEquivalent: "\\")
        closeColumnItem.keyEquivalentModifierMask = [.command, .shift]
        editMenu.addItem(closeColumnItem)
        let viewItem = NSMenuItem(
            title: t("Second View"),
            action: #selector(DocumentController.addView(_:)),
            keyEquivalent: "\\")
        viewItem.keyEquivalentModifierMask = [.command, .option]
        editMenu.addItem(viewItem)
        let closeViewItem = NSMenuItem(
            title: t("Close View"),
            action: #selector(DocumentController.closeView(_:)),
            keyEquivalent: "\\")
        closeViewItem.keyEquivalentModifierMask = [.command, .option, .shift]
        editMenu.addItem(closeViewItem)
        let nextPaneItem = NSMenuItem(
            title: t("Next Pane"),
            action: #selector(DocumentController.focusOtherSide(_:)),
            keyEquivalent: "`")
        nextPaneItem.keyEquivalentModifierMask = [.command, .option]
        editMenu.addItem(nextPaneItem)
        editMenu.addItem(.separator())
        editMenu.addItem(makeTransformMenuItem())

        let editMenuItem = NSMenuItem()
        editMenuItem.submenu = editMenu
        mainMenu.addItem(editMenuItem)

        let viewMenu = NSMenu(title: t("View"))
        let toggleNavigator = NSMenuItem(
            title: t("Toggle Navigator"),
            action: #selector(NSSplitViewController.toggleSidebar(_:)),
            keyEquivalent: "0"
        )
        viewMenu.addItem(toggleNavigator)
        let togglePreview = NSMenuItem(
            title: t("Toggle Markdown Preview"),
            action: #selector(DocumentController.togglePreview(_:)),
            keyEquivalent: "p"
        )
        togglePreview.keyEquivalentModifierMask = [.command, .option]
        viewMenu.addItem(togglePreview)
        let lineNumbersItem = NSMenuItem(
            title: t("Toggle Line Numbers"),
            action: #selector(toggleLineNumbers(_:)),
            keyEquivalent: "l"
        )
        lineNumbersItem.keyEquivalentModifierMask = [.command, .shift]
        viewMenu.addItem(lineNumbersItem)
        let pathDisplayItem = NSMenuItem(
            title: t("Toggle Path Display"),
            action: #selector(togglePathDisplay(_:)),
            keyEquivalent: "t"
        )
        pathDisplayItem.keyEquivalentModifierMask = [.command, .option]
        viewMenu.addItem(pathDisplayItem)
        let hoverDocsItem = NSMenuItem(
            title: t("Hover Documentation"),
            action: #selector(toggleHoverDocs(_:)),
            keyEquivalent: ""
        )
        viewMenu.addItem(hoverDocsItem)
        let showHoverItem = NSMenuItem(
            title: t("Show Documentation for Symbol"),
            action: #selector(DocumentController.showHoverAtCaret(_:)),
            keyEquivalent: "h"
        )
        showHoverItem.keyEquivalentModifierMask = [.command, .control]
        viewMenu.addItem(showHoverItem)
        let serverStatusItem = NSMenuItem(
            title: t("Language Server Status"),
            action: #selector(showServerStatus(_:)),
            keyEquivalent: ""
        )
        viewMenu.addItem(serverStatusItem)
        let revealItem = NSMenuItem(
            title: t("Reveal in Tree"),
            action: #selector(DocumentController.revealInTree(_:)),
            keyEquivalent: "j"
        )
        revealItem.keyEquivalentModifierMask = [.command, .shift]
        viewMenu.addItem(revealItem)
        let outlineItem = NSMenuItem(
            title: t("Document Outline…"),
            action: #selector(DocumentController.showDocumentOutline(_:)),
            keyEquivalent: "o"
        )
        outlineItem.keyEquivalentModifierMask = [.command, .shift]
        viewMenu.addItem(outlineItem)
        let diagnosticListItem = NSMenuItem(
            title: t("Diagnostics…"),
            action: #selector(DocumentController.showDiagnosticList(_:)),
            keyEquivalent: "e"
        )
        diagnosticListItem.keyEquivalentModifierMask = [.command, .shift]
        viewMenu.addItem(diagnosticListItem)
        let diagnosticItem = NSMenuItem(
            title: t("Show Diagnostic for Line"),
            action: #selector(DocumentController.showDiagnosticAtCaret(_:)),
            keyEquivalent: "e"
        )
        diagnosticItem.keyEquivalentModifierMask = [.command, .control]
        viewMenu.addItem(diagnosticItem)
        let blameItem = NSMenuItem(
            title: t("Blame Line…"),
            action: #selector(DocumentController.blameLine(_:)),
            keyEquivalent: "b"
        )
        blameItem.keyEquivalentModifierMask = [.command, .control]
        viewMenu.addItem(blameItem)
        let goToLineItem = NSMenuItem(
            title: t("Go to Line…"),
            action: #selector(DocumentController.goToLine(_:)),
            keyEquivalent: "l"
        )
        goToLineItem.keyEquivalentModifierMask = [.command]
        viewMenu.addItem(goToLineItem)
        let redrawItem = NSMenuItem(
            title: t("Redraw"),
            action: #selector(DocumentController.redrawDocument(_:)),
            keyEquivalent: "l"
        )
        redrawItem.keyEquivalentModifierMask = [.command, .option]
        viewMenu.addItem(redrawItem)
        viewMenu.addItem(.separator())
        let paletteItem = NSMenuItem(
            title: t("Command Palette…"),
            action: #selector(showCommandPalette(_:)),
            keyEquivalent: "p"
        )
        paletteItem.keyEquivalentModifierMask = [.command, .shift]
        viewMenu.addItem(paletteItem)
        let viewMenuItem = NSMenuItem()
        viewMenuItem.submenu = viewMenu
        mainMenu.addItem(viewMenuItem)

        let windowMenu = NSMenu(title: t("Window"))
        windowMenu.addItem(
            withTitle: t("Minimize"),
            action: #selector(NSWindow.performMiniaturize(_:)),
            keyEquivalent: "m"
        )
        windowMenu.addItem(.separator())
        let nextTab = NSMenuItem(
            title: t("Next Tab"),
            action: #selector(DocumentController.selectNextTab(_:)),
            keyEquivalent: "\t")
        nextTab.keyEquivalentModifierMask = [.control]
        windowMenu.addItem(nextTab)
        let previousTab = NSMenuItem(
            title: t("Previous Tab"),
            action: #selector(DocumentController.selectPreviousTab(_:)),
            keyEquivalent: "\t")
        previousTab.keyEquivalentModifierMask = [.control, .shift]
        windowMenu.addItem(previousTab)
        // ⌘1…⌘9: the tab by its place on the bar, in the pane with the
        // keyboard.
        for number in 1...9 {
            let item = NSMenuItem(
                title: "Tab \(number)",
                action: #selector(selectTabByNumber(_:)),
                keyEquivalent: "\(number)")
            item.tag = number
            item.isAlternate = false
            item.isHidden = number > 1
            windowMenu.addItem(item)
        }
        windowMenu.addItem(.separator())
        windowMenu.addItem(
            withTitle: t("This File in Every Column"),
            action: #selector(DocumentController.showInEveryPane(_:)),
            keyEquivalent: "")
        windowMenu.addItem(
            withTitle: t("Move Tab to New Window"),
            action: #selector(DocumentController.moveTabToNewWindow(_:)),
            keyEquivalent: "")
        windowMenu.addItem(.separator())
        let windowMenuItem = NSMenuItem()
        windowMenuItem.submenu = windowMenu
        mainMenu.addItem(windowMenuItem)
        NSApp.windowsMenu = windowMenu

        return mainMenu
    }
}
