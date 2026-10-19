import CTextchum
import Foundation

/// Refactorings that touch several files, undone and redone as one: see
/// `textchum_core::refactoring`.
public enum CoreRefactoring {
    /// Starts one called `label`; its id tags the edit groups it makes.
    public static func begin(label: String) -> UInt64 {
        label.withCString { tc_refactoring_begin($0, UInt(strlen($0))) }
    }

    public static func noteOpenDocument(_ id: UInt64) {
        tc_refactoring_note_open_document(id)
    }

    /// Writes `text` to `path`, a file not open, keeping what it held;
    /// nil when written, otherwise why not.
    public static func writeClosed(_ id: UInt64, path: String, text: String) -> String? {
        let answer = path.withCString { pathPointer in
            text.withCString { textPointer in
                tc_refactoring_write_closed(
                    id, pathPointer, UInt(strlen(pathPointer)), textPointer, UInt(strlen(textPointer)))
            }
        }
        guard let answer else { return nil }
        defer { tc_string_free(answer) }
        return String(cString: answer)
    }

    /// Makes the files a workspace edit creates, before its text edits;
    /// what was left undone, in words.
    public static func createFiles(_ id: UInt64, editJSON: String) -> [String] {
        guard let answer = editJSON.withCString({ tc_refactoring_create_files(id, $0, UInt(strlen($0))) })
        else { return [] }
        defer { tc_string_free(answer) }
        return (try? JSONDecoder().decode([String].self, from: Data(String(cString: answer).utf8))) ?? []
    }

    /// How many files it touched; 0 for one not remembered.
    public static func filesTouched(_ id: UInt64) -> Int {
        Int(tc_refactoring_files_touched(id))
    }

    /// Undoes (or redoes) it in the files not open, after `documents`
    /// open ones, and says what was done.
    public static func finish(_ id: UInt64, undo: Bool, documents: Int) -> String {
        guard let answer = tc_refactoring_finish(id, undo, UInt(documents)) else { return "" }
        defer { tc_string_free(answer) }
        return String(cString: answer)
    }
}
