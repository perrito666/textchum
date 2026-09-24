import AppKit
import TextchumKit

/// The thin bar under the editor that answers the questions a look at
/// the text cannot: where the caret is, how wide a tab is and whether
/// indents are tabs or spaces, and what language the file is being
/// treated as.
///
/// The parts that name a per-file choice — indentation and language —
/// are clickable and open File Properties, where that choice is made.
///
/// Its right end is the notification area: the last thing the editor
/// had to say, as much of it as fits, and a click on it lists the
/// session's notices. What used to be a dialog to dismiss is a line
/// here instead.
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
    /// The last word, at the right end. Truncated to what the bar has
    /// room for; the whole of it is the tooltip, and the list is a
    /// click away.
    private let notice = makeButton()
    private var shown = Info()
    private var noticePopover: NSPopover?

    /// The session's notices, shared by every window's bar. The
    /// workbench hands it over; without one the bar still shows the
    /// last word and lists nothing.
    var log: CoreNotices?

    /// What the bar says right now, for the smoke test.
    var shownInfo: Info { shown }
    /// Opens File Properties for the focused document.
    var onProperties: (() -> Void)?

    static let height: CGFloat = 24

    override init(frame: NSRect) {
        super.init(frame: frame)
        translatesAutoresizingMaskIntoConstraints = false
        let font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        for label in [position, encoding, branch] {
            label.font = font
            label.textColor = .secondaryLabelColor
        }
        branch.toolTip = t("The git branch checked out in this file's project")
        for button in [indent, language, notice] {
            button.font = font
            button.target = self
        }
        indent.action = #selector(openProperties(_:))
        language.action = #selector(openProperties(_:))
        notice.action = #selector(showNotices(_:))
        notice.alignment = .right
        notice.lineBreakMode = .byTruncatingTail
        notice.toolTip = t("What the editor had to say — click for the whole list")
        // The row keeps its natural width; the notice takes what is
        // left and gives it up first when the window narrows.
        notice.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        notice.setContentHuggingPriority(.defaultLow, for: .horizontal)
        notice.translatesAutoresizingMaskIntoConstraints = false
        let row = NSStackView(views: [position, indent, language, encoding, branch])
        row.orientation = .horizontal
        row.spacing = 14
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        addSubview(notice)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: Self.height),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            row.centerYAnchor.constraint(equalTo: centerYAnchor),
            notice.leadingAnchor.constraint(greaterThanOrEqualTo: row.trailingAnchor, constant: 14),
            notice.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            notice.centerYAnchor.constraint(equalTo: centerYAnchor),
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

    /// Puts `text` at the right end, where it stays until the next
    /// thing worth saying. One line of it; the rest is a click away.
    func notice(_ text: String) {
        let firstLine =
            text.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: true)
            .first.map(String.init) ?? text
        notice.title = firstLine
        notice.toolTip = text
    }

    /// For the smoke test: the last word, if anything has been said.
    var noticeText: String? { notice.title.isEmpty ? nil : notice.title }

    /// The session's notices as the list shows them: newest first, each
    /// with the time it was said.
    func historyText() -> String {
        guard let log, log.count > 0 else { return t("Nothing to report yet.") }
        let clock = DateFormatter()
        clock.dateStyle = .none
        clock.timeStyle = .short
        return log.entries
            .map { "\(clock.string(from: $0.at))  \($0.text)" }
            .joined(separator: "\n\n")
    }

    @objc private func showNotices(_ sender: Any?) {
        if let popover = noticePopover, popover.isShown {
            popover.close()
            return
        }
        let popover = NSPopover()
        popover.behavior = .transient
        let controller = NSViewController()
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 460, height: 260))
        scroll.hasVerticalScroller = true
        scroll.borderType = .noBorder
        let text = NSTextView(frame: scroll.bounds)
        text.isEditable = false
        text.isSelectable = true
        text.drawsBackground = false
        text.textContainerInset = NSSize(width: 10, height: 10)
        text.font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        text.textColor = .labelColor
        text.string = historyText()
        text.autoresizingMask = [.width]
        text.isVerticallyResizable = true
        text.textContainer?.widthTracksTextView = true
        scroll.documentView = text
        controller.view = scroll
        popover.contentViewController = controller
        popover.contentSize = scroll.frame.size
        noticePopover = popover
        popover.show(relativeTo: notice.bounds, of: notice, preferredEdge: .maxY)
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        bounds.fill()
        NSColor.separatorColor.withAlphaComponent(0.5).setFill()
        NSRect(x: 0, y: bounds.maxY - 1, width: bounds.width, height: 1).fill()
    }
}
