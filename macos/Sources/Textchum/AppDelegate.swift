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
    private let sidebarModel = SidebarModel()
    /// Strong references to open editors; windows do not retain their
    /// controllers. Entries are removed as their windows close.
    private var editors: [EditorWindowController] = []

    /// `~/Library/Application Support/Textchum/config.json` — GUI-managed,
    /// hand-editable JSON.
    private static var configPath: String {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Textchum/config.json").path
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = makeMainMenu()

        let config = CoreConfig(path: Self.configPath)
        self.config = config
        let settingsModel = SettingsModel(config: config)
        settingsModel.onChange = { [weak self] in
            guard let self, let model = self.settingsModel else { return }
            self.applyAppearanceChoice()
            for editor in self.editors {
                editor.apply(settings: model.currentSettings)
            }
        }
        self.settingsModel = settingsModel
        applyAppearanceChoice()

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
                    // Save-as gives untitled documents a path; recents
                    // track it (the controller cannot reach this list).
                    if let path = changedEditor?.coreDocument.path {
                        self?.noteRecent(path: path)
                    }
                }
            }
        }

        // The core's event channel; ping once on launch so a broken
        // channel is caught immediately.
        let coreApp = CoreApp { [weak self] event in
            self?.handleCoreEvent(event)
        }
        coreApp.ping(sequence: 1)
        self.coreApp = coreApp

        // Open files given on the command line; otherwise a fresh window.
        let fileArguments = CommandLine.arguments.dropFirst()
            .filter { FileManager.default.fileExists(atPath: $0) }
        if fileArguments.isEmpty {
            newDocument(nil)
        } else {
            for path in fileArguments {
                open(path: path)
            }
        }
        NSApp.activate(ignoringOtherApps: true)

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

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    /// Quitting reviews every dirty window through the same save/discard
    /// flow as closing it by hand.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
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
        }
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

    private func rebuildSidebar() {
        sidebarModel.rebuild(
            entries: editors.map { editor in
                (
                    document: SidebarDocument(
                        id: ObjectIdentifier(editor),
                        title: editor.window?.title ?? "Untitled",
                        path: editor.coreDocument.path,
                        isDirty: editor.coreDocument.isDirty
                    ),
                    projectRoot: editor.projectRoot
                )
            })
    }

    /// Cross-file navigation: front (or open) `path` and put the caret at
    /// an LSP position. Used by go-to-definition.
    private func openLocation(path: String, line: Int, character: Int) {
        if let existing = editors.first(where: { $0.coreDocument.path == path }) {
            existing.window?.makeKeyAndOrderFront(nil)
            existing.reveal(line: line, character: character)
        } else {
            open(path: path)
            editors.first { $0.coreDocument.path == path }?
                .reveal(line: line, character: character)
        }
    }

    private var sidebarConfiguration: SidebarConfiguration {
        SidebarConfiguration(
            model: sidebarModel,
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
            }
        )
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
        show(
            editor: EditorWindowController(
                document: CoreDocument(),
                settings: currentSettings,
                sidebar: sidebarConfiguration,
                lspApp: coreApp,
                openLocation: { [weak self] path, line, character in
                    self?.openLocation(path: path, line: line, character: character)
                }            ))
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

    /// Opens `path` in a new editor window, alerting on failure.
    private func open(path: String) {
        // Absolute, standardized paths throughout: relative paths (e.g.
        // from the command line) would corrupt project-root resolution
        // and defeat open-file deduplication.
        let path = URL(fileURLWithPath: path).standardizedFileURL.path
        do {
            let document = try CoreDocument(contentsOf: path)
            closeUntouchedUntitledWindows()
            noteRecent(path: path)
            show(
                editor: EditorWindowController(
                    document: document,
                    settings: currentSettings,
                    sidebar: sidebarConfiguration,
                lspApp: coreApp,
                openLocation: { [weak self] path, line, character in
                    self?.openLocation(path: path, line: line, character: character)
                }                ))
        } catch {
            let alert = NSAlert()
            alert.alertStyle = .warning
            alert.messageText = "Could not open “\((path as NSString).lastPathComponent)”."
            alert.informativeText = "\(error)"
            alert.runModal()
        }
    }

    private func show(editor: EditorWindowController) {
        editors.append(editor)
        if let window = editor.window {
            NotificationCenter.default.addObserver(
                forName: NSWindow.willCloseNotification,
                object: window,
                queue: .main
            ) { [weak self] notification in
                guard let closing = notification.object as? NSWindow else { return }
                MainActor.assumeIsolated {
                    self?.editors.removeAll { $0.window === closing }
                }
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { self?.rebuildSidebar() }
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
        fileMenu.addItem(
            withTitle: "Open…", action: #selector(openDocument(_:)), keyEquivalent: "o")
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
