import AppKit
import SwiftUI

/// A path input with filesystem completion and a Browse… button — used
/// everywhere the settings ask for a project root.
struct PathPicker: View {
    @Binding var text: String
    var placeholder: String

    var body: some View {
        HStack(spacing: 4) {
            CompletingPathField(text: $text, placeholder: placeholder)
            Button {
                let panel = NSOpenPanel()
                panel.canChooseDirectories = true
                panel.canChooseFiles = false
                panel.allowsMultipleSelection = false
                if !text.isEmpty {
                    panel.directoryURL = URL(
                        fileURLWithPath: (text as NSString).expandingTildeInPath)
                }
                if panel.runModal() == .OK, let url = panel.url {
                    text = url.path
                }
            } label: {
                Image(systemName: "folder")
            }
            .help("Choose a folder")
        }
    }
}

/// An `NSTextField` whose typing completes directory paths: the last path
/// component autocompletes against the entries of its parent directory.
private struct CompletingPathField: NSViewRepresentable {
    @Binding var text: String
    var placeholder: String

    func makeNSView(context: Context) -> NSTextField {
        let field = NSTextField()
        field.placeholderString = placeholder
        field.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        field.delegate = context.coordinator
        field.lineBreakMode = .byTruncatingHead
        return field
    }

    func updateNSView(_ field: NSTextField, context: Context) {
        if field.stringValue != text {
            field.stringValue = text
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        private let text: Binding<String>
        /// True while the field editor is inserting a completion, so the
        /// resulting change notification does not re-trigger completion.
        private var completing = false
        private var previousLength = 0

        init(text: Binding<String>) {
            self.text = text
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            text.wrappedValue = field.stringValue
            let length = field.stringValue.count
            defer { previousLength = length }
            // Complete only while typing forward, never on deletion.
            guard !completing, length > previousLength,
                field.stringValue.hasPrefix("/") || field.stringValue.hasPrefix("~"),
                let editor = field.currentEditor() as? NSTextView
            else { return }
            completing = true
            editor.complete(nil)
            completing = false
        }

        func control(
            _ control: NSControl,
            textView: NSTextView,
            completions words: [String],
            forPartialWordRange charRange: NSRange,
            indexOfSelectedItem index: UnsafeMutablePointer<Int>
        ) -> [String] {
            // The partial word is the last path component ("/" is a word
            // boundary), so completions are directory names in the parent.
            let full = (textView.string as NSString)
            let typed = full.substring(to: NSMaxRange(charRange))
            let expanded = (typed as NSString).expandingTildeInPath
            let parent = (expanded as NSString).deletingLastPathComponent
            let prefix = ((expanded as NSString).lastPathComponent).lowercased()
            guard
                let entries = try? FileManager.default.contentsOfDirectory(
                    at: URL(fileURLWithPath: parent),
                    includingPropertiesForKeys: [.isDirectoryKey],
                    options: [.skipsHiddenFiles])
            else { return [] }
            index.pointee = -1
            return entries
                .filter {
                    (try? $0.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true
                        && (prefix.isEmpty
                            || $0.lastPathComponent.lowercased().hasPrefix(prefix))
                }
                .map(\.lastPathComponent)
                .sorted()
        }
    }
}
