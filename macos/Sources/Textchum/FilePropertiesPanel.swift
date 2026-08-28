import AppKit
import TextchumKit

/// File properties (⌘I): what this document is, when its name does not
/// say. A `.txt` holding SQL, a dotfile with no extension, an `.inc`
/// that is really C — the filename decides the language everywhere else
/// in the editor, and this is where it can be told otherwise.
///
/// Indentation lives here too. Tab width and tabs-versus-spaces are set
/// for a project or for everything; one file that disagrees with its
/// project had nowhere to say so.
///
/// A choice is remembered for the path, so reopening the file finds it
/// the way it was left.
@MainActor
final class FilePropertiesPanel: NSObject {
    static let shared = FilePropertiesPanel()

    /// What the panel shows and what it hands back.
    struct Properties {
        /// nil = follow the filename.
        var language: String?
        /// nil = follow the project or the global setting.
        var tabWidth: UInt32?
        /// nil = follow the setting; true = spaces, false = tabs.
        var spaces: Bool?
    }

    private var panel: NSPanel?
    private let languagePopUp = NSPopUpButton()
    private let tabWidthField = NSTextField()
    private let indentPopUp = NSPopUpButton()
    private let factsLabel = NSTextField(labelWithString: "")
    private var onChange: ((Properties) -> Void)?
    private var languages: [String] = []

    /// Shows the panel for a document. `detected` is the language the
    /// filename implies, shown as the default choice so the difference
    /// between "no opinion" and "same as detected" stays visible.
    func show(
        over window: NSWindow?,
        title: String,
        facts: String,
        detected: String?,
        properties: Properties,
        onChange: @escaping (Properties) -> Void
    ) {
        self.onChange = onChange
        let panel = self.panel ?? makePanel()
        self.panel = panel
        panel.title = title
        factsLabel.stringValue = facts

        languages = CoreLanguages.all.map(\.name).sorted()
        languagePopUp.removeAllItems()
        let followTitle =
            detected.map { "Automatic (\($0))" } ?? "Automatic (plain text)"
        languagePopUp.addItem(withTitle: followTitle)
        languagePopUp.menu?.addItem(.separator())
        for language in languages {
            languagePopUp.addItem(withTitle: language)
        }
        if let chosen = properties.language, let index = languages.firstIndex(of: chosen) {
            // Two items before the list: the automatic row and the
            // separator.
            languagePopUp.selectItem(at: index + 2)
        } else {
            languagePopUp.selectItem(at: 0)
        }

        tabWidthField.stringValue = properties.tabWidth.map(String.init) ?? ""
        indentPopUp.selectItem(at: properties.spaces.map { $0 ? 1 : 2 } ?? 0)

        if let window {
            panel.setFrameOrigin(
                NSPoint(
                    x: window.frame.midX - panel.frame.width / 2,
                    y: window.frame.midY - panel.frame.height / 2
                ))
        }
        panel.makeKeyAndOrderFront(nil)
    }

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 380, height: 210),
            styleMask: [.titled, .closable, .utilityWindow],
            backing: .buffered,
            defer: false
        )
        panel.isFloatingPanel = true
        panel.hidesOnDeactivate = false

        let grid = NSGridView(views: [
            [NSTextField(labelWithString: "Language:"), languagePopUp],
            [NSTextField(labelWithString: "Tab width:"), tabWidthField],
            [NSTextField(labelWithString: "Indent with:"), indentPopUp],
        ])
        grid.column(at: 0).xPlacement = .trailing
        grid.rowSpacing = 10
        grid.columnSpacing = 10

        languagePopUp.target = self
        languagePopUp.action = #selector(changed)
        tabWidthField.placeholderString = "follow the project"
        tabWidthField.target = self
        tabWidthField.action = #selector(changed)
        indentPopUp.removeAllItems()
        indentPopUp.addItems(withTitles: ["Automatic", "Spaces", "Tabs"])
        indentPopUp.target = self
        indentPopUp.action = #selector(changed)

        factsLabel.font = .systemFont(ofSize: 11)
        factsLabel.textColor = .secondaryLabelColor
        factsLabel.lineBreakMode = .byTruncatingMiddle

        let stack = NSStackView(views: [grid, factsLabel])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 14
        stack.edgeInsets = NSEdgeInsets(top: 18, left: 20, bottom: 18, right: 20)
        panel.contentView = stack
        return panel
    }

    @objc private func changed() {
        var properties = Properties()
        let index = languagePopUp.indexOfSelectedItem
        if index >= 2, index - 2 < languages.count {
            properties.language = languages[index - 2]
        }
        // Blank means "no opinion"; a number out of range is not a
        // tab width anyone meant, so it is treated the same way.
        if let width = UInt32(tabWidthField.stringValue.trimmingCharacters(in: .whitespaces)),
            (1...16).contains(width)
        {
            properties.tabWidth = width
        }
        switch indentPopUp.indexOfSelectedItem {
        case 1: properties.spaces = true
        case 2: properties.spaces = false
        default: properties.spaces = nil
        }
        onChange?(properties)
    }
}
