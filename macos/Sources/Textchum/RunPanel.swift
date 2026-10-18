import AppKit
import TextchumKit

/// The run panel, under the editor: a command's output as it comes, in
/// its colours, with the places it names underlined to open with a
/// click and walked with Next and Previous Output Place. While the
/// command runs, a line can be sent to its input — which is what a
/// command that has gone quiet is often waiting for, and the panel says
/// so. The output is the core's (`textchum_core::output`); this draws it.
@MainActor
final class RunPanel: NSView, NSTextViewDelegate, NSTextFieldDelegate {
    /// The run on show, and the core that runs it.
    private(set) var runID: UInt64?
    weak var core: CoreApp?
    /// Opens a place the output names: path, zero-based line and column.
    var onOpenPlace: ((String, Int, Int) -> Void)?
    /// The panel's height when shown; zero when not.
    var heightConstraint: NSLayoutConstraint?
    static let shownHeight: CGFloat = 220

    private(set) var command = ""
    private let title = NSTextField(labelWithString: "")
    private let status = NSTextField(labelWithString: "")
    private let stopButton = NSButton(title: t("Stop"), target: nil, action: nil)
    private let closeButton = NSButton(title: t("Close"), target: nil, action: nil)
    private let scroll = NSScrollView()
    private let text = NSTextView()
    private let input = NSTextField()
    /// Each output line's length in UTF-16 units, so a changed line can
    /// be found in the text without measuring the whole of it.
    private var lineLengths: [Int] = []
    /// The places each output line names: (start, end, path, line, column).
    private var places: [[(Int, Int, String, Int, Int)]] = []
    /// Where Next Output Place stepped last: an output line and a start.
    private var stepped: (line: Int, character: Int)?

    var isShown: Bool { (heightConstraint?.constant ?? 0) > 0 }
    /// What the panel shows, for the smoke test.
    var shownText: String { text.string }
    var statusText: String { status.stringValue }

    override init(frame: NSRect) {
        super.init(frame: frame)
        translatesAutoresizingMaskIntoConstraints = false
        let small = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        title.font = NSFont.boldSystemFont(ofSize: NSFont.smallSystemFontSize)
        title.lineBreakMode = .byTruncatingTail
        title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        status.font = small
        status.textColor = .secondaryLabelColor
        status.lineBreakMode = .byTruncatingTail
        status.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        for button in [stopButton, closeButton] {
            button.bezelStyle = .inline
            button.font = small
            button.target = self
        }
        stopButton.action = #selector(stop(_:))
        closeButton.action = #selector(close(_:))
        let header = NSStackView(views: [title, status, NSView(), stopButton, closeButton])
        header.orientation = .horizontal
        header.spacing = 8
        header.edgeInsets = NSEdgeInsets(top: 4, left: 10, bottom: 4, right: 10)

        text.isEditable = false
        text.isSelectable = true
        text.drawsBackground = false
        text.delegate = self
        text.textContainerInset = NSSize(width: 6, height: 4)
        text.isAutomaticLinkDetectionEnabled = false
        text.linkTextAttributes = [.underlineStyle: NSUnderlineStyle.single.rawValue, .cursor: NSCursor.pointingHand]
        text.autoresizingMask = [.width]
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.borderType = .noBorder

        input.placeholderString = t("A line for its input — ⏎ sends it")
        input.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        input.delegate = self
        input.target = self
        input.action = #selector(send(_:))

        let stack = NSStackView(views: [header, scroll, input])
        stack.orientation = .vertical
        stack.spacing = 0
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        let separator = NSBox()
        separator.boxType = .separator
        separator.translatesAutoresizingMaskIntoConstraints = false
        addSubview(separator)
        NSLayoutConstraint.activate([
            separator.topAnchor.constraint(equalTo: topAnchor),
            separator.leadingAnchor.constraint(equalTo: leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: separator.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
            header.leadingAnchor.constraint(equalTo: stack.leadingAnchor),
            header.trailingAnchor.constraint(equalTo: stack.trailingAnchor),
            input.leadingAnchor.constraint(equalTo: stack.leadingAnchor, constant: 8),
            input.trailingAnchor.constraint(equalTo: stack.trailingAnchor, constant: -8),
        ])
        clipsToBounds = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("RunPanel is built in code") }

    /// Shows a run that just started, in place of the one before, which
    /// is let go of.
    func begin(id: UInt64, command: String) {
        if let previous = runID, previous != id {
            core?.runForget(id: previous)
        }
        runID = id
        self.command = command
        title.stringValue = command
        status.stringValue = t("running")
        text.string = ""
        lineLengths = []
        places = []
        stepped = nil
        stopButton.isEnabled = true
        input.isHidden = false
        input.stringValue = ""
        heightConstraint?.constant = Self.shownHeight
    }

    /// What the core said about the run on show.
    func handle(status news: String) {
        guard let id = runID, let core else { return }
        switch news {
        case "quiet":
            status.stringValue = t("no output for a while — it may be waiting for input")
        default:
            if let update = core.runRead(id: id) {
                apply(update)
                if !update.running {
                    status.stringValue = t("finished")
                    stopButton.isEnabled = false
                    input.isHidden = true
                } else if news == "output" {
                    status.stringValue = t("running")
                }
            }
        }
    }

    private func apply(_ update: CoreApp.RunUpdate) {
        guard let storage = text.textStorage else { return }
        let atBottom = scroll.contentView.bounds.maxY >= text.frame.height - 4
        let first = update.reset ? 0 : min(update.first, lineLengths.count)
        let offset = lineLengths.prefix(first).reduce(0, +)
        let replaced = NSRange(location: offset, length: storage.length - offset)
        let rendered = NSMutableAttributedString()
        lineLengths.removeSubrange(first...)
        places.removeSubrange(min(first, places.count)...)
        for (index, line) in update.lines.enumerated() {
            let piece = render(line, outputLine: first + index)
            let isLast = index == update.lines.count - 1
            if !isLast { piece.append(NSAttributedString(string: "\n", attributes: Self.plain)) }
            lineLengths.append(piece.length)
            rendered.append(piece)
        }
        storage.replaceCharacters(in: replaced, with: rendered)
        if atBottom {
            text.scrollToEndOfDocument(nil)
        }
    }

    private static var plain: [NSAttributedString.Key: Any] {
        [.font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular), .foregroundColor: NSColor.labelColor]
    }

    private func render(_ line: CoreApp.RunUpdate.Line, outputLine: Int) -> NSMutableAttributedString {
        let piece = NSMutableAttributedString(string: line.text, attributes: Self.plain)
        let characters = Array(line.text)
        // Characters to UTF-16 offsets, as the core counts in characters.
        func utf16(_ character: Int) -> Int {
            String(characters.prefix(max(0, min(character, characters.count)))).utf16.count
        }
        for span in line.spans {
            guard span.count == 4, let start = span[0].number, let end = span[1].number else { continue }
            let range = NSRange(location: utf16(start), length: utf16(end) - utf16(start))
            if let hex = span[2].text, let colour = Self.colour(hex) {
                piece.addAttribute(.foregroundColor, value: colour, range: range)
            }
            if span[3].flag {
                piece.addAttribute(.font, value: NSFont.monospacedSystemFont(ofSize: 11, weight: .bold), range: range)
            }
        }
        var named: [(Int, Int, String, Int, Int)] = []
        for (index, place) in line.places.enumerated() {
            guard place.count == 5, let start = place[0].number, let end = place[1].number,
                let path = place[2].text, let line = place[3].number, let column = place[4].number
            else { continue }
            named.append((start, end, path, line, column))
            let range = NSRange(location: utf16(start), length: utf16(end) - utf16(start))
            piece.addAttribute(.link, value: "textchum-place:\(outputLine):\(index)", range: range)
        }
        places.append(named)
        return piece
    }

    private static func colour(_ hex: String) -> NSColor? {
        guard hex.hasPrefix("#"), let value = UInt32(hex.dropFirst(), radix: 16) else { return nil }
        return NSColor(
            srgbRed: CGFloat(value >> 16 & 0xff) / 255, green: CGFloat(value >> 8 & 0xff) / 255,
            blue: CGFloat(value & 0xff) / 255, alpha: 1)
    }

    /// A place clicked in the output opens.
    func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
        guard let link = link as? String, link.hasPrefix("textchum-place:") else { return false }
        let parts = link.dropFirst("textchum-place:".count).split(separator: ":").compactMap { Int($0) }
        guard parts.count == 2, places.indices.contains(parts[0]), places[parts[0]].indices.contains(parts[1])
        else { return true }
        let place = places[parts[0]][parts[1]]
        stepped = (parts[0], place.0)
        onOpenPlace?(place.2, place.3, place.4)
        return true
    }

    /// Next Output Place (F4) and Previous (⇧F4): the place after or
    /// before the last one gone to, round the end, marked in the output
    /// and opened. False when the output names none.
    @discardableResult
    func step(forward: Bool) -> Bool {
        guard let id = runID, let core,
            let place = core.runNeighbouringPlace(id: id, from: stepped, forward: forward)
        else { return false }
        stepped = (place.outputLine, place.start)
        // The place marked in the output: the core counts characters, the
        // text view UTF-16 units.
        let all = text.string as NSString
        let offset = lineLengths.prefix(place.outputLine).reduce(0, +)
        if lineLengths.indices.contains(place.outputLine), offset <= all.length {
            let lineRange = NSRange(location: offset, length: min(lineLengths[place.outputLine], all.length - offset))
            let characters = Array(all.substring(with: lineRange))
            let before = String(characters.prefix(place.start)).utf16.count
            let length = String(characters.dropFirst(place.start).prefix(place.end - place.start)).utf16.count
            let range = NSRange(location: offset + before, length: length)
            text.setSelectedRange(range)
            text.scrollRangeToVisible(range)
        }
        onOpenPlace?(place.path, place.line, place.column)
        return true
    }

    @objc private func send(_ sender: Any?) {
        guard let id = runID, !input.stringValue.isEmpty else { return }
        if core?.runWrite(id: id, line: input.stringValue) == true {
            input.stringValue = ""
            status.stringValue = t("running")
        }
    }

    @objc func stop(_ sender: Any?) {
        guard let id = runID else { return }
        core?.runStop(id: id)
    }

    @objc private func close(_ sender: Any?) {
        hide()
    }

    /// Puts the panel away; a run still going keeps going, and the panel
    /// comes back when it is shown again.
    func hide() {
        heightConstraint?.constant = 0
        window?.makeFirstResponder(nil)
    }

    func show() {
        guard runID != nil else { return }
        heightConstraint?.constant = Self.shownHeight
    }
}
