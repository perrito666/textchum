import AppKit
import TextchumKit

/// The thin bar under the editor that answers the questions a look at
/// the text cannot: where the caret is, how wide a tab is and whether
/// indents are tabs or spaces, and what language the file is being
/// treated as.
///
/// The parts that name a per-file choice — indentation and language —
/// are clickable and open File Properties, where that choice is made.
final class StatusBar: NSView {
    /// What one refresh says. Assembled by the workbench from the
    /// focused document; the bar only draws it.
    struct Info: Equatable {
        var line = 1
        var column = 1
        var tabWidth = 4
        var usesTabs = false
        var language: String?
        var encoding = ""
    }

    private let position = NSTextField(labelWithString: "")
    private let indent = makeButton()
    private let language = makeButton()
    private let encoding = NSTextField(labelWithString: "")
    private var shown = Info()
    /// Opens File Properties for the focused document.
    var onProperties: (() -> Void)?

    static let height: CGFloat = 24

    override init(frame: NSRect) {
        super.init(frame: frame)
        translatesAutoresizingMaskIntoConstraints = false
        let font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        for label in [position, encoding] {
            label.font = font
            label.textColor = .secondaryLabelColor
        }
        for button in [indent, language] {
            button.font = font
            button.target = self
            button.action = #selector(openProperties(_:))
        }
        let row = NSStackView(views: [position, indent, language, encoding])
        row.orientation = .horizontal
        row.spacing = 14
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: Self.height),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            row.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -12),
            row.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("StatusBar is created in code")
    }

    private static func makeButton() -> NSButton {
        let button = NSButton(title: "", target: nil, action: nil)
        button.isBordered = false
        button.contentTintColor = .secondaryLabelColor
        button.setButtonType(.momentaryChange)
        return button
    }

    func show(_ info: Info) {
        guard info != shown else { return }
        shown = info
        position.stringValue = t("Ln {}, Col {}", info.line, info.column)
        indent.title =
            info.usesTabs
            ? t("Tabs: {}", info.tabWidth)
            : t("Spaces: {}", info.tabWidth)
        language.title = info.language ?? t("Plain Text")
        encoding.stringValue = info.encoding
        indent.toolTip = t("How this file is indented — click to change it")
        language.toolTip = t("What this file is treated as — click to change it")
    }

    @objc private func openProperties(_ sender: Any?) {
        onProperties?()
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        bounds.fill()
        NSColor.separatorColor.withAlphaComponent(0.5).setFill()
        NSRect(x: 0, y: bounds.maxY - 1, width: bounds.width, height: 1).fill()
    }
}
