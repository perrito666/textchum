import AppKit
import TextchumKit

/// Find and replace, docked above the text: a pattern, a replacement,
/// three switches — regular expression in Vim's dialect, match case,
/// whole word — and the buttons. The document does the finding; the
/// bar only says what was typed and shows how many were found.
@MainActor
final class FindReplaceBar: NSView {
    /// What the bar asks the document to do.
    enum Request {
        case changed
        case next
        case previous
        case replace
        case replaceAll
        case close
    }

    var onRequest: ((Request) -> Void)?

    let patternField = NSTextField()
    let replacementField = NSTextField()
    private let regexSwitch = NSButton(checkboxWithTitle: ".*", target: nil, action: nil)
    private let caseSwitch = NSButton(checkboxWithTitle: "Aa", target: nil, action: nil)
    private let wordSwitch = NSButton(checkboxWithTitle: "\\b", target: nil, action: nil)
    private let status = NSTextField(labelWithString: "")

    static let height: CGFloat = 34

    var pattern: String { patternField.stringValue }
    var replacement: String { replacementField.stringValue }
    var options: CoreFind.Options {
        CoreFind.Options(
            regex: regexSwitch.state == .on, caseSensitive: caseSwitch.state == .on,
            wholeWord: wordSwitch.state == .on)
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        let font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        patternField.placeholderString = t("Find")
        replacementField.placeholderString = t("Replace")
        for field in [patternField, replacementField] {
            field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            field.delegate = self
            field.setContentHuggingPriority(.defaultLow, for: .horizontal)
        }
        regexSwitch.toolTip = t("Regular expression, in Vim's dialect")
        caseSwitch.toolTip = t("Match case")
        wordSwitch.toolTip = t("Whole word")
        for toggle in [regexSwitch, caseSwitch, wordSwitch] {
            toggle.font = font
            toggle.target = self
            toggle.action = #selector(optionChanged(_:))
            toggle.setContentHuggingPriority(.required, for: .horizontal)
            toggle.setContentCompressionResistancePriority(.required, for: .horizontal)
        }
        let previous = NSButton(title: "‹", target: self, action: #selector(previousPressed))
        let next = NSButton(title: "›", target: self, action: #selector(nextPressed))
        let replace = NSButton(title: t("Replace"), target: self, action: #selector(replacePressed))
        let all = NSButton(title: t("All"), target: self, action: #selector(replaceAllPressed))
        let done = NSButton(title: t("Done"), target: self, action: #selector(donePressed))
        for button in [previous, next, replace, all, done] {
            button.font = font
            button.bezelStyle = .rounded
            button.controlSize = .small
            button.setContentHuggingPriority(.required, for: .horizontal)
            button.setContentCompressionResistancePriority(.required, for: .horizontal)
        }
        status.font = font
        status.textColor = .secondaryLabelColor
        status.lineBreakMode = .byTruncatingTail
        status.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let row = NSStackView(views: [
            patternField, replacementField, regexSwitch, caseSwitch, wordSwitch,
            previous, next, replace, all, status, done,
        ])
        row.orientation = .horizontal
        row.spacing = 6
        row.edgeInsets = NSEdgeInsets(top: 4, left: 8, bottom: 4, right: 8)
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor),
            row.trailingAnchor.constraint(equalTo: trailingAnchor),
            row.topAnchor.constraint(equalTo: topAnchor),
            row.bottomAnchor.constraint(equalTo: bottomAnchor),
            patternField.widthAnchor.constraint(greaterThanOrEqualToConstant: 140),
            replacementField.widthAnchor.constraint(greaterThanOrEqualToConstant: 120),
            patternField.widthAnchor.constraint(equalTo: replacementField.widthAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("FindReplaceBar is built in code") }

    /// Drawn, not a layer colour: the window colour follows the
    /// appearance, and a colour taken once at build stays light.
    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        bounds.fill()
        NSColor.separatorColor.withAlphaComponent(0.5).setFill()
        NSRect(x: 0, y: 0, width: bounds.width, height: 1).fill()
    }

    /// Puts the bar to use: the pattern to start from, and the keyboard.
    func begin(with pattern: String?) {
        if let pattern, !pattern.isEmpty { patternField.stringValue = pattern }
        window?.makeFirstResponder(patternField)
        patternField.currentEditor()?.selectAll(nil)
    }

    /// "3 of 12", "No matches", or what is wrong with the pattern.
    func show(found count: Int, current: Int?, problem: String?) {
        if let problem {
            status.stringValue = problem
            status.textColor = .systemRed
            return
        }
        status.textColor = .secondaryLabelColor
        if count == 0 {
            status.stringValue = pattern.isEmpty ? "" : t("No matches")
        } else if let current {
            status.stringValue = t("{} of {}", String(current + 1), String(count))
        } else {
            status.stringValue = t("{} matches", String(count))
        }
    }

    /// For the smoke test: the switches, set the way a click would.
    func setOptions(_ options: CoreFind.Options) {
        regexSwitch.state = options.regex ? .on : .off
        caseSwitch.state = options.caseSensitive ? .on : .off
        wordSwitch.state = options.wholeWord ? .on : .off
    }

    @objc private func optionChanged(_ sender: Any?) { onRequest?(.changed) }
    @objc private func previousPressed() { onRequest?(.previous) }
    @objc private func nextPressed() { onRequest?(.next) }
    @objc private func replacePressed() { onRequest?(.replace) }
    @objc private func replaceAllPressed() { onRequest?(.replaceAll) }
    @objc private func donePressed() { onRequest?(.close) }
}

extension FindReplaceBar: NSTextFieldDelegate {
    func controlTextDidChange(_ notification: Notification) {
        if notification.object as AnyObject? === patternField { onRequest?(.changed) }
    }

    /// ⏎ in the pattern goes to the next match, ⇧⏎ to the previous; ⏎ in
    /// the replacement replaces the current one and moves on; ⎋ closes.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.insertNewline(_:)):
            if control === replacementField {
                onRequest?(.replace)
            } else if NSEvent.modifierFlags.contains(.shift) {
                onRequest?(.previous)
            } else {
                onRequest?(.next)
            }
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            onRequest?(.close)
            return true
        default:
            return false
        }
    }
}
