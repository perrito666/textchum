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
        /// The git branch of the file's project, when it has one.
        var branch: String?
    }

    private let position = NSTextField(labelWithString: "")
    private let indent = makeButton()
    private let language = makeButton()
    private let encoding = NSTextField(labelWithString: "")
    private let branch = NSTextField(labelWithString: "")
    /// A word in passing — a save that went ahead without its
    /// preprocessors — shown for a while at the end of the bar and
    /// then gone, without a dialog to dismiss.
    private let notice = NSTextField(labelWithString: "")
    private var noticeTimer: Timer?
    private var shown = Info()

    /// What the bar says right now, for the smoke test.
    var shownInfo: Info { shown }
    /// Opens File Properties for the focused document.
    var onProperties: (() -> Void)?

    static let height: CGFloat = 24

    override init(frame: NSRect) {
        super.init(frame: frame)
        translatesAutoresizingMaskIntoConstraints = false
        let font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        for label in [position, encoding, branch, notice] {
            label.font = font
            label.textColor = .secondaryLabelColor
        }
        notice.textColor = .systemOrange
        notice.lineBreakMode = .byTruncatingTail
        notice.isHidden = true
        branch.toolTip = t("The git branch checked out in this file's project")
        for button in [indent, language] {
            button.font = font
            button.target = self
            button.action = #selector(openProperties(_:))
        }
        let row = NSStackView(views: [position, indent, language, encoding, branch, notice])
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
        branch.stringValue = info.branch.map { "⎇ \($0)" } ?? ""
        branch.isHidden = info.branch == nil
        indent.toolTip = t("How this file is indented — click to change it")
        language.toolTip = t("What this file is treated as — click to change it")
    }

    @objc private func openProperties(_ sender: Any?) {
        onProperties?()
    }

    /// Says `text` for a while. Long enough to be read, not so long
    /// that it reads as the state of things.
    func notice(_ text: String) {
        notice.stringValue = text
        notice.toolTip = text
        notice.isHidden = false
        noticeTimer?.invalidate()
        noticeTimer = Timer.scheduledTimer(withTimeInterval: 8, repeats: false) { [weak self] _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    self?.notice.isHidden = true
                    self?.notice.stringValue = ""
                }
            }
        }
    }

    /// For the smoke test: what the bar is saying in passing, if anything.
    var noticeText: String? { notice.isHidden ? nil : notice.stringValue }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        bounds.fill()
        NSColor.separatorColor.withAlphaComponent(0.5).setFill()
        NSRect(x: 0, y: bounds.maxY - 1, width: bounds.width, height: 1).fill()
    }
}
