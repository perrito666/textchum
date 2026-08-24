import AppKit
import TextchumKit

/// Application lifecycle: builds the main menu, starts the core, and opens
/// an editor window.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var coreApp: CoreApp?
    private var editorWindowController: EditorWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = Self.makeMainMenu()

        let windowController = EditorWindowController()
        editorWindowController = windowController

        // Prove the async event path on every launch: ping the core and
        // reflect the reply in the window subtitle when it comes back.
        let coreApp = CoreApp { [weak windowController] event in
            if case let .pong(sequence) = event {
                windowController?.coreDidRespond(toPing: sequence)
            }
        }
        coreApp.ping(sequence: 1)
        self.coreApp = coreApp

        windowController.showWindow(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    /// Programmatic main menu: the app has no nib. Only what the current
    /// feature set needs — app menu for quitting, Edit menu so the standard
    /// editing key equivalents (⌘X/C/V/A/Z) reach the text view.
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

        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(
            withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        editMenu.addItem(
            withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "Z")
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

        return mainMenu
    }
}
