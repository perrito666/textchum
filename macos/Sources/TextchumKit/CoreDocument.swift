import CTextchum
import Foundation

/// A file I/O failure reported by the core, carrying its message.
public struct CoreIOError: Error, CustomStringConvertible {
    public let message: String

    public var description: String { message }
}

/// A text document owned by the core: buffer, undo history, path, encoding.
///
/// Like ``CoreBuffer``, this is the shell's only way to hold document text —
/// views cache a copy for display and every mutation goes through here
/// first. Additionally the document tracks dirty state against its last
/// save and provides undo/redo whose results the view replays verbatim.
///
/// Not thread-safe: use from the main thread.
public final class CoreDocument {
    /// A change the core performed on itself via undo/redo. The shell must
    /// apply the same replacement to its display cache.
    public struct AppliedEdit {
        public let range: NSRange
        public let replacement: String
    }

    private let handle: OpaquePointer

    /// Creates an empty untitled document.
    public init() {
        self.handle = tc_document_new()!
    }

    /// Opens the file at `path`.
    public init(contentsOf path: String) throws {
        var error: UnsafeMutablePointer<CChar>?
        let handle = Self.withUTF8Pointer(path) { pointer, length in
            tc_document_open(pointer, length, &error)
        }
        guard let handle else {
            throw CoreIOError(message: Self.takeString(error) ?? "unknown error opening \(path)")
        }
        self.handle = handle
    }

    deinit {
        tc_document_free(handle)
    }

    /// The full document contents.
    public var text: String {
        guard let cString = tc_document_text(handle) else { return "" }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }

    /// Length in UTF-8 bytes.
    public var lengthInBytes: Int {
        Int(tc_document_len_bytes(handle))
    }

    /// Length in UTF-16 code units.
    public var lengthInUTF16: Int {
        Int(tc_document_len_utf16(handle))
    }

    /// Whether the document differs from its last saved state.
    public var isDirty: Bool {
        tc_document_is_dirty(handle)
    }

    public var canUndo: Bool {
        tc_document_can_undo(handle)
    }

    public var canRedo: Bool {
        tc_document_can_redo(handle)
    }

    /// The document's file path, or nil for untitled documents.
    public var path: String? {
        Self.takeString(tc_document_path(handle))
    }

    /// Human-readable encoding name, e.g. "UTF-8" or "ISO-8859-1".
    public var encodingName: String {
        String(cString: tc_document_encoding_name(handle))
    }

    /// Replaces an `NSRange` of UTF-16 code units with `text`, recording the
    /// edit in the undo history.
    public func replace(utf16Range range: NSRange, with text: String) throws {
        let accepted = Self.withUTF8Pointer(text) { pointer, length in
            tc_document_replace_utf16(
                handle,
                UInt(range.location),
                UInt(range.location + range.length),
                pointer,
                length
            )
        }
        guard accepted else {
            throw CoreRejectedOperation(operation: "replace utf16 range \(range)")
        }
    }

    /// Ends the current undo coalescing run; the next edit starts a fresh
    /// undo step. Call when the caret moves for reasons other than typing.
    public func breakUndoCoalescing() {
        tc_document_break_undo_group(handle)
    }

    /// Undoes the newest edit, returning the change to replay on the display
    /// cache, or nil if there was nothing to undo.
    public func undo() -> AppliedEdit? {
        popHistory(tc_document_undo)
    }

    /// Redoes the most recently undone edit; same contract as ``undo()``.
    public func redo() -> AppliedEdit? {
        popHistory(tc_document_redo)
    }

    /// Saves to the document's path. Untitled documents fail; use
    /// ``save(to:)``.
    public func save() throws {
        var error: UnsafeMutablePointer<CChar>?
        guard tc_document_save(handle, &error) else {
            throw CoreIOError(message: Self.takeString(error) ?? "unknown save error")
        }
    }

    /// Saves to `path` and adopts it as the document's path.
    public func save(to path: String) throws {
        var error: UnsafeMutablePointer<CChar>?
        let saved = Self.withUTF8Pointer(path) { pointer, length in
            tc_document_save_as(handle, pointer, length, &error)
        }
        guard saved else {
            throw CoreIOError(message: Self.takeString(error) ?? "unknown save error")
        }
    }

    private func popHistory(
        _ operation: (OpaquePointer?, UnsafeMutablePointer<TcAppliedEdit>?) -> Bool
    ) -> AppliedEdit? {
        var edit = TcAppliedEdit(start: 0, end: 0, text: nil)
        guard operation(handle, &edit) else { return nil }
        return AppliedEdit(
            range: NSRange(location: Int(edit.start), length: Int(edit.end - edit.start)),
            replacement: Self.takeString(edit.text) ?? ""
        )
    }

    /// Consumes a core-owned C string: copies it into a Swift string and
    /// frees the original. Nil-safe.
    private static func takeString(_ cString: UnsafeMutablePointer<CChar>?) -> String? {
        guard let cString else { return nil }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }

    /// Runs `body` with a `(pointer, length)` view of the string's UTF-8.
    private static func withUTF8Pointer<R>(
        _ text: String,
        _ body: (UnsafePointer<CChar>?, UInt) -> R
    ) -> R {
        var text = text
        return text.withUTF8 { bytes in
            let pointer = bytes.baseAddress.map {
                UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
            }
            return body(pointer, UInt(bytes.count))
        }
    }
}
