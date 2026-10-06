import AppKit

/// Rename Symbol's field, laid over the name where it stands.
///
/// A dialog put the name out of sight behind it and showed nothing of
/// what would change. Here the name becomes a field in its own place,
/// prefilled and selected, in the editor's font; the symbol's other
/// uses are marked in the text meanwhile. Return hands the new name
/// on, Escape or the keyboard going elsewhere leaves everything as it
/// was. The field is only the asking: the renaming is the server's
/// workspace edit, applied the way it always was.
@MainActor
final class RenameField: NSTextField, NSTextFieldDelegate {
    /// Called once with the new name; not called when the name is
    /// unchanged or empty, which is a cancel.
    var onCommit: ((String) -> Void)?
    var onCancel: (() -> Void)?
    private var finished = false
    /// The width the name had: the field never shrinks below it.
    private var minimumWidth: CGFloat = 0

    /// Lays the field over `rect`, `name`'s rectangle in `view`, and
    /// gives it the keyboard with the name selected.
    static func begin(over rect: NSRect, in view: NSView, name: String, font: NSFont) -> RenameField {
        let field = RenameField(frame: .zero)
        field.font = font
        field.stringValue = name
        field.isBezeled = false
        field.isBordered = false
        field.drawsBackground = true
        field.backgroundColor = .textBackgroundColor
        field.focusRingType = .none
        field.delegate = field
        field.wantsLayer = true
        field.layer?.borderColor = NSColor.controlAccentColor.cgColor
        field.layer?.borderWidth = 1
        field.layer?.cornerRadius = 3
        // Two points of air on each side of the glyphs, and the field's
        // own inset undone, so the letters sit where the text's did.
        field.minimumWidth = rect.width + 8
        field.frame = NSRect(
            x: rect.minX - 4, y: rect.minY - 2, width: field.minimumWidth, height: rect.height + 4)
        view.addSubview(field)
        view.window?.makeFirstResponder(field)
        field.currentEditor()?.selectAll(nil)
        return field
    }

    /// Hands the name on, when it is a new one, and goes away.
    func commit() {
        guard !finished else { return }
        let name = stringValue.trimmingCharacters(in: .whitespaces)
        finished = true
        let onCommit = self.onCommit
        let onCancel = self.onCancel
        leave()
        if name.isEmpty {
            onCancel?()
        } else {
            onCommit?(name)
        }
    }

    func cancel() {
        guard !finished else { return }
        finished = true
        let onCancel = self.onCancel
        leave()
        onCancel?()
    }

    /// Goes away, and gives the keyboard back to the text: taking the
    /// field out ends its editing, which leaves the window itself with
    /// the keyboard and nothing to type into.
    private func leave() {
        let window = self.window
        let next = superview
        removeFromSuperview()
        if let next {
            window?.makeFirstResponder(next)
        }
    }

    /// Grows with the name as it is typed, never below the old one.
    func controlTextDidChange(_ notification: Notification) {
        let needed = (stringValue as NSString).size(withAttributes: [.font: font as Any]).width + 10
        var frame = self.frame
        frame.size.width = max(minimumWidth, ceil(needed))
        self.frame = frame
    }

    func control(
        _ control: NSControl, textView: NSTextView, doCommandBy selector: Selector
    ) -> Bool {
        switch selector {
        case #selector(NSResponder.insertNewline(_:)):
            commit()
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            cancel()
            return true
        default:
            return false
        }
    }

    /// The keyboard went elsewhere — a click in the text, another
    /// window — and that is the user walking away from the question.
    func controlTextDidEndEditing(_ notification: Notification) {
        cancel()
    }
}
