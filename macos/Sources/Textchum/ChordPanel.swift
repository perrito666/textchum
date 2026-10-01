import AppKit
import TextchumKit

/// The chord window: commands by one key each.
///
/// Two modifier keys pressed together open it; it lists what can be
/// done, a key beside each, in groups short enough to read. The next
/// key runs a command or opens a group, Escape closes it. It is the
/// answer to shortcuts of three and four keys, which have to be both
/// remembered and reached: here the screen does the remembering.
///
/// The panel never becomes key. The window the user was typing in keeps
/// the keyboard, and the app's event monitor hands this the keys while
/// it is up, so a command it runs acts on the document in front.
@MainActor
final class ChordPanel {
    private var panel: NSPanel?
    private let label = NSTextField(labelWithString: "")
    /// The list on show: the menu, or the group last opened.
    private var entries: [CoreChords.Entry] = []
    /// The groups opened to get here, for the heading.
    private var trail: [String] = []
    /// The window it was opened over, to stay centred on as it resizes.
    private weak var anchor: NSWindow?

    /// Runs the action with this name; set by the app.
    var onRun: ((String) -> Void)?

    var isShown: Bool { panel?.isVisible == true }

    /// The lines on show, for the smoke test.
    var shownText: String { label.stringValue }

    func show(over window: NSWindow?) {
        entries = CoreChords.menu
        trail = []
        anchor = window
        render()
        let panel = self.panel ?? makePanel()
        self.panel = panel
        place(panel, over: window)
        panel.orderFront(nil)
    }

    func close() {
        panel?.orderOut(nil)
    }

    /// Takes a key typed while the window is up. A group opens, an
    /// action runs and closes the window, and a key that is neither is
    /// refused with the window left up: a slip should not cost the
    /// place in the menu.
    @discardableResult
    func press(_ key: Character) -> Bool {
        guard let entry = entries.first(where: { $0.key == String(key) }) else {
            NSSound.beep()
            return false
        }
        if let items = entry.items {
            entries = items
            trail.append(entry.label)
            render()
            if let panel { place(panel, over: anchor) }
        } else if let action = entry.action {
            close()
            onRun?(action)
        }
        return true
    }

    /// The list as text: a heading, then the entries in columns, a key
    /// and its name each, a group marked with a plus. With it, where
    /// the keys are, so they can be told from the names at a glance.
    static func text(
        for entries: [CoreChords.Entry], trail: [String]
    ) -> (text: String, keys: [NSRange]) {
        let heading = (trail.isEmpty ? t("Commands") : trail.joined(separator: " ▸ "))
        let cells = entries.map { entry in
            "\(entry.key)  \(entry.items == nil ? "" : "+")\(entry.label)"
        }
        // Down the columns, so a list reads top to bottom.
        let rows = max(1, Int((Double(cells.count) / 3).rounded(.up)))
        let columns = stride(from: 0, to: cells.count, by: rows).map {
            Array(cells[$0..<min($0 + rows, cells.count)])
        }
        let widths = columns.map { ($0.map(\.count).max() ?? 0) + 4 }
        var text = heading + "\n\n"
        var keys: [NSRange] = []
        for row in 0..<rows {
            var line = ""
            for (index, column) in columns.enumerated() where row < column.count {
                let cell = column[row]
                keys.append(NSRange(location: (text + line).utf16.count, length: 1))
                line += cell + String(repeating: " ", count: widths[index] - cell.count)
            }
            text += line.trimmingCharacters(in: .whitespaces) + "\n"
        }
        text += "\n" + t("esc closes")
        return (text, keys)
    }

    private func render() {
        let (text, keys) = Self.text(for: entries, trail: trail)
        let font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        let styled = NSMutableAttributedString(
            string: text, attributes: [.font: font, .foregroundColor: NSColor.labelColor])
        let bold = NSFont.monospacedSystemFont(ofSize: 12, weight: .bold)
        let whole = text as NSString
        styled.addAttribute(.font, value: bold, range: whole.lineRange(for: NSRange(location: 0, length: 0)))
        for key in keys {
            styled.addAttributes(
                [.font: bold, .foregroundColor: NSColor.systemYellow], range: key)
        }
        styled.addAttribute(
            .foregroundColor, value: NSColor.secondaryLabelColor,
            range: whole.lineRange(for: NSRange(location: whole.length, length: 0)))
        label.attributedStringValue = styled
        label.sizeToFit()
        guard let panel else { return }
        let size = NSSize(width: label.frame.width + 36, height: label.frame.height + 28)
        panel.setContentSize(size)
        label.setFrameOrigin(NSPoint(x: 18, y: 14))
    }

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 320, height: 160),
            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        panel.isFloatingPanel = true
        panel.level = .floating
        panel.hasShadow = true
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hidesOnDeactivate = true
        // The backdrop is dark whatever the theme; the text is drawn
        // for a dark window so it stays readable on it.
        panel.appearance = NSAppearance(named: .darkAqua)
        let backdrop = NSVisualEffectView()
        backdrop.material = .hudWindow
        backdrop.state = .active
        backdrop.wantsLayer = true
        backdrop.layer?.cornerRadius = 10
        backdrop.layer?.masksToBounds = true
        backdrop.autoresizingMask = [.width, .height]
        // The text is set in a font where every character is as wide
        // as the next: its columns are padded with spaces.
        label.maximumNumberOfLines = 0
        backdrop.addSubview(label)
        panel.contentView = backdrop
        self.panel = panel
        render()
        return panel
    }

    /// Centred along the bottom of the window being worked in, clear of
    /// its status bar: near where the eyes are, and over none of the
    /// line being edited.
    private func place(_ panel: NSPanel, over window: NSWindow?) {
        guard let frame = window?.frame ?? NSScreen.main?.visibleFrame else { return }
        var origin = NSPoint(
            x: frame.midX - panel.frame.width / 2, y: frame.minY + StatusBar.height + 16)
        if let screen = (window?.screen ?? NSScreen.main)?.visibleFrame {
            origin.x = min(max(origin.x, screen.minX + 8), screen.maxX - panel.frame.width - 8)
            origin.y = max(origin.y, screen.minY + 8)
        }
        panel.setFrameOrigin(origin)
    }
}
