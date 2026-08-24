import Foundation

/// LSP text-edit plumbing, shared by rename (a workspace-wide edit
/// across files) and formatting (a batch of edits in one document).
/// Positions are LSP-style: zero-based lines, UTF-16 columns.
enum LSPEdits {
    struct TextEdit {
        let startLine: Int
        let startCharacter: Int
        let endLine: Int
        let endCharacter: Int
        let newText: String
    }

    /// A `TextEdit[]` result (formatting).
    static func textEdits(fromResultJSON json: String) -> [TextEdit] {
        guard let data = json.data(using: .utf8),
            let array = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]]
        else { return [] }
        return edits(fromArray: array)
    }

    /// A `WorkspaceEdit` result (rename), flattened to edits per absolute
    /// path. Reads both spellings: `changes` (uri → edits) and
    /// `documentChanges` (documents with edits; rename never needs the
    /// create/delete/rename operations, which are skipped).
    static func workspaceEdits(fromResultJSON json: String) -> [String: [TextEdit]] {
        guard let data = json.data(using: .utf8),
            let result = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        else { return [:] }
        var byPath: [String: [TextEdit]] = [:]
        for (uri, rawEdits) in result["changes"] as? [String: [[String: Any]]] ?? [:] {
            guard let path = path(fromURI: uri) else { continue }
            byPath[path, default: []] += edits(fromArray: rawEdits)
        }
        for change in result["documentChanges"] as? [[String: Any]] ?? [] {
            guard
                let document = change["textDocument"] as? [String: Any],
                let uri = document["uri"] as? String,
                let path = path(fromURI: uri),
                let rawEdits = change["edits"] as? [[String: Any]]
            else { continue }
            byPath[path, default: []] += edits(fromArray: rawEdits)
        }
        return byPath
    }

    private static func edits(fromArray array: [[String: Any]]) -> [TextEdit] {
        array.compactMap { raw in
            guard
                let range = raw["range"] as? [String: Any],
                let start = range["start"] as? [String: Any],
                let end = range["end"] as? [String: Any],
                let startLine = start["line"] as? Int,
                let startCharacter = start["character"] as? Int,
                let endLine = end["line"] as? Int,
                let endCharacter = end["character"] as? Int,
                let newText = raw["newText"] as? String
            else { return nil }
            return TextEdit(
                startLine: startLine, startCharacter: startCharacter,
                endLine: endLine, endCharacter: endCharacter, newText: newText)
        }
    }

    static func path(fromURI uri: String) -> String? {
        guard uri.hasPrefix("file://"), let url = URL(string: uri) else { return nil }
        return url.path
    }

    /// The UTF-16 index of an LSP position, clamped into the text.
    static func index(ofLine line: Int, character: Int, in text: NSString) -> Int {
        var index = 0
        var current = 0
        while current < line && index < text.length {
            index = NSMaxRange(text.lineRange(for: NSRange(location: index, length: 0)))
            current += 1
        }
        return min(index + max(0, character), text.length)
    }

    static func nsRange(of edit: TextEdit, in text: NSString) -> NSRange {
        let start = index(ofLine: edit.startLine, character: edit.startCharacter, in: text)
        let end = max(start, index(ofLine: edit.endLine, character: edit.endCharacter, in: text))
        return NSRange(location: start, length: end - start)
    }

    /// Edits sorted so applying front-to-back never shifts a later range:
    /// bottom-most first.
    static func bottomUp(_ edits: [TextEdit]) -> [TextEdit] {
        edits.sorted {
            ($0.startLine, $0.startCharacter) > ($1.startLine, $1.startCharacter)
        }
    }

    /// `contents` with the edits applied (bottom-up, so ranges stay
    /// valid). Used for files that are not open in any window.
    static func applied(to contents: String, edits: [TextEdit]) -> String {
        let mutable = NSMutableString(string: contents)
        for edit in bottomUp(edits) {
            mutable.replaceCharacters(
                in: nsRange(of: edit, in: mutable), with: edit.newText)
        }
        return mutable as String
    }
}
