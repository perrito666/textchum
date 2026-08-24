import AppKit
import TextchumKit

/// Application lifecycle: the main menu, the core instance, and the set of
/// open editor windows.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var coreApp: CoreApp?
    /// Strong references to open editors; windows do not retain their
    /// controllers. Entries are removed as their windows close.
    private var editors: [EditorWindowController] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = Self.makeMainMenu()

        // The core's event channel (diagnostics and more will arrive here);
        // ping once on launch so a broken channel is caught immediately.
        let coreApp = CoreApp { event in
            if case let .pong(sequence) = event {
                NSLog("core \(Core.version) event channel verified (pong \(sequence))")
            }
        }
        coreApp.ping(sequence: 1)
        self.coreApp = coreApp

        newDocument(nil)
        NSApp.activate(ignoringOtherApps: true)
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

    // MARK: Document actions

    @objc func newDocument(_ sender: Any?) {
        show(editor: EditorWindowController(document: CoreDocument()))
    }

    @objc func openDocument(_ sender: Any?) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        guard panel.runModal() == .OK else { return }
        for url in panel.urls {
            do {
                let document = try CoreDocument(contentsOf: url.path)
                show(editor: EditorWindowController(document: document))
            } catch {
                let alert = NSAlert()
                alert.alertStyle = .warning
                alert.messageText = "Could not open “\(url.lastPathComponent)”."
                alert.informativeText = "\(error)"
                alert.runModal()
            }
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
            }
        }
        editor.showWindow(nil)
    }

    // MARK: Menu

    /// Programmatic main menu: the app has no nib. Undo/redo use dedicated
    /// selectors handled by the editor window controller, since document
    /// history lives in the core rather than in an `NSUndoManager`.
    private static func makeMainMenu() -> NSMenu {
        let mainMenu = NSMenu()

        let appMenu = NSMenu()
        appMenu.addItem(
            withTitle: "About Textchum",
            action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
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
        fileMenu.addItem(
            withTitle: "Open…", action: #selector(openDocument(_:)), keyEquivalent: "o")
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
        let editMenuItem = NSMenuItem()
        editMenuItem.submenu = editMenu
        mainMenu.addItem(editMenuItem)

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
