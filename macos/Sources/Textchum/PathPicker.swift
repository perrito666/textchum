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
        private let completion = PathCompletion()

        init(text: Binding<String>) {
            self.text = text
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            text.wrappedValue = field.stringValue
            completion.textDidChange(in: field)
        }

        func control(
            _ control: NSControl,
            textView: NSTextView,
            completions words: [String],
            forPartialWordRange charRange: NSRange,
            indexOfSelectedItem index: UnsafeMutablePointer<Int>
        ) -> [String] {
            index.pointee = -1
            return PathCompletion.completions(
                in: textView.string, forPartialWordRange: charRange)
        }
    }
}

/// Filesystem completion for a text field that expects a folder: the
/// last component of what is typed completes against the folders of its
/// parent. One of these per field, fed from the field's delegate —
/// every input that asks for a path completes the same way.
@MainActor
final class PathCompletion {
    /// True while the field editor is inserting a completion, so the
    /// resulting change notification does not re-trigger completion.
    private var completing = false
    private var previousLength = 0

    /// A SwiftUI coordinator is made off the main actor's books, and
    /// there is nothing here to protect until a field calls in.
    nonisolated init() {}

    /// Call from `controlTextDidChange`. Offers completions only while
    /// typing forward, never on deletion, and only for a path that says
    /// where it starts.
    func textDidChange(in field: NSTextField) {
        let length = field.stringValue.count
        defer { previousLength = length }
        guard !completing, length > previousLength,
            field.stringValue.hasPrefix("/") || field.stringValue.hasPrefix("~"),
            let editor = field.currentEditor() as? NSTextView
        else { return }
        completing = true
        editor.complete(nil)
        completing = false
    }

    /// What the field editor should offer for `range`, the partial word
    /// it is about to replace in `text`.
    ///
    /// The folders come from the last path component, but the answer is
    /// phrased for the range, because the two are not the same thing:
    /// the field editor breaks words at "-", "." and spaces, so in
    /// `/src/my-pr` it replaces only `pr`, and just past a slash it
    /// replaces the folder before it, slash included. Answering with
    /// whole names made `my-my-project` out of the first and dropped a
    /// level out of the second.
    nonisolated static func completions(
        in text: String, forPartialWordRange range: NSRange
    ) -> [String] {
        let typed = (text as NSString).substring(to: NSMaxRange(range)) as NSString
        let slash = typed.range(of: "/", options: .backwards)
        guard slash.location != NSNotFound else { return [] }
        let componentStart = NSMaxRange(slash)
        let component = typed.substring(from: componentStart) as NSString
        let parent = (typed.substring(to: componentStart) as NSString).expandingTildeInPath
        return directories(in: parent, startingWith: component as String).compactMap { name in
            if range.location <= componentStart {
                // The range reaches back past the component: what it
                // covers before the name goes back in with the name.
                let before = NSRange(
                    location: range.location, length: componentStart - range.location)
                return typed.substring(with: before) + name
            }
            // The start of the component stays as typed, so it has to
            // be the name's own start, case and all.
            let kept = range.location - componentStart
            let name = name as NSString
            guard name.length >= kept, name.substring(to: kept) == component.substring(to: kept)
            else { return nil }
            return name.substring(from: kept)
        }
    }

    /// The folders of `parent` whose names start with `prefix`, whatever
    /// its case. Hidden ones are offered once the dot has been typed.
    nonisolated static func directories(in parent: String, startingWith prefix: String) -> [String] {
        let prefix = prefix.lowercased()
        let options: FileManager.DirectoryEnumerationOptions =
            prefix.hasPrefix(".") ? [] : [.skipsHiddenFiles]
        guard
            let entries = try? FileManager.default.contentsOfDirectory(
                at: URL(fileURLWithPath: parent, isDirectory: true),
                includingPropertiesForKeys: [.isDirectoryKey],
                options: options)
        else { return [] }
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
