import AppKit
import SwiftUI

/// A text field with a list attached: the known values are offered, and
/// anything else can still be typed.
///
/// Language names appear in several places in Settings and were plain
/// text fields, so choosing one meant knowing how it is spelled here.
/// The set is open — a language the build does not know can be
/// configured, and will be once a grammar or a server arrives for it —
/// so a closed picker would be wrong. `NSComboBox` is both.
struct EditableCombo: NSViewRepresentable {
    @Binding var text: String
    var placeholder: String
    var options: [String]

    func makeNSView(context: Context) -> NSComboBox {
        let combo = NSComboBox()
        combo.usesDataSource = false
        combo.completes = true
        combo.hasVerticalScroller = true
        combo.numberOfVisibleItems = 12
        combo.placeholderString = placeholder
        combo.delegate = context.coordinator
        combo.target = context.coordinator
        combo.action = #selector(Coordinator.changed(_:))
        return combo
    }

    func updateNSView(_ combo: NSComboBox, context: Context) {
        context.coordinator.text = $text
        if combo.objectValues as? [String] != options {
            combo.removeAllItems()
            combo.addItems(withObjectValues: options)
        }
        if combo.stringValue != text {
            combo.stringValue = text
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    final class Coordinator: NSObject, NSComboBoxDelegate {
        var text: Binding<String>

        init(text: Binding<String>) {
            self.text = text
        }

        /// Typing and picking arrive through different paths; both end
        /// up here so the binding sees either.
        @objc func changed(_ sender: NSComboBox) {
            text.wrappedValue = sender.stringValue
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let combo = notification.object as? NSComboBox else { return }
            text.wrappedValue = combo.stringValue
        }

        func comboBoxSelectionDidChange(_ notification: Notification) {
            guard let combo = notification.object as? NSComboBox else { return }
            // The selection is not in stringValue yet at this point.
            let index = combo.indexOfSelectedItem
            guard index >= 0, let value = combo.itemObjectValue(at: index) as? String
            else { return }
            text.wrappedValue = value
        }
    }
}
