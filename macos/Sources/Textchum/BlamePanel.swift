import AppKit
import TextchumKit

/// Blame Line: what git knows about the line under the caret.
///
/// A panel rather than an alert, because the answer is text people want
/// to read and copy — a commit hash to paste into `git show`, a message
/// body to quote. An alert's label is neither selectable nor scrollable.
@MainActor
final class BlamePanel: NSObject {
    static let shared = BlamePanel()

    private var panel: NSPanel?
    private let text = NSTextView()
    private var commit = ""

    /// Shows what git said about `line` of `path`.
    func show(_ blame: CoreBlame.Line, file: String, over window: NSWindow?) {
        let panel = self.panel ?? makePanel()
        self.panel = panel
        commit = blame.commit
        panel.title = "\((file as NSString).lastPathComponent):\(blame.line)"

        let body = NSMutableAttributedString()
        let heading: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 11, weight: .semibold),
            .foregroundColor: NSColor.secondaryLabelColor,
        ]
        let value: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.labelColor,
        ]
        let monospaced: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: NSColor.labelColor,
        ]
        func row(_ label: String, _ content: String, mono: Bool = false) {
            guard !content.isEmpty else { return }
            body.append(NSAttributedString(string: label + "\n", attributes: heading))
            body.append(
                NSAttributedString(string: content + "\n\n", attributes: mono ? monospaced : value))
        }

        if blame.uncommitted {
            body.append(
                NSAttributedString(
                    string: "This line is not committed yet.\n\n"
                        + "It was typed since the last commit, so git has nobody to name.",
                    attributes: value))
        } else {
            row("Commit", blame.abbreviated, mono: true)
            row(
                "Author",
                blame.authorMail.isEmpty
                    ? blame.author : "\(blame.author) <\(blame.authorMail)>")
            row("Written", blame.authorDate)
            row("Committed by", blame.committer)
            row("Committed", blame.committerDate)
            row("Named at the time", blame.renamedFrom, mono: true)
            row("Subject", blame.summary)
            row("Message", blame.body)
        }

        text.textStorage?.setAttributedString(body)
        panel.setContentSize(NSSize(width: 480, height: 360))
        if let window {
            panel.setFrameOrigin(
                NSPoint(
                    x: window.frame.midX - panel.frame.width / 2,
                    y: window.frame.midY - panel.frame.height / 2))
        } else {
            panel.center()
        }
        panel.makeKeyAndOrderFront(nil)
    }

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 480, height: 360),
            styleMask: [.titled, .closable, .resizable, .utilityWindow],
            backing: .buffered,
            defer: false
        )
        panel.isFloatingPanel = true
        panel.contentMinSize = NSSize(width: 360, height: 240)

        text.isEditable = false
        text.isSelectable = true
        text.drawsBackground = false
        text.textContainerInset = NSSize(width: 14, height: 12)
        let scroll = NSScrollView()
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false

        // Copying the hash is most of what the answer is for.
        let copy = NSButton(
            title: "Copy Commit", target: self, action: #selector(copyCommit))
        copy.translatesAutoresizingMaskIntoConstraints = false

        let content = NSView()
        content.addSubview(scroll)
        content.addSubview(copy)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: content.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: copy.topAnchor, constant: -10),
            copy.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -14),
            copy.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -12),
        ])
        panel.contentView = content
        return panel
    }

    @objc private func copyCommit() {
        guard !commit.isEmpty else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(commit, forType: .string)
    }
}
