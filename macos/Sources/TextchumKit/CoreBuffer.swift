import CTextchum
import Foundation

/// A text document owned by the core.
///
/// This class is the shell's *only* way to hold document text. Views may
/// cache a copy for display, but every mutation must go through here first;
/// the core is the source of truth and the cache is reconciled afterwards.
///
/// Not thread-safe: use from a single thread (in the app, the main thread).
public final class CoreBuffer {
    private let handle: OpaquePointer

    /// Creates an empty buffer.
    public init() {
        // The only failure mode is allocation failure, which is fatal anyway.
        self.handle = tc_buffer_new()!
    }

    deinit {
        tc_buffer_free(handle)
    }

    /// Length of the document in UTF-8 bytes.
    public var lengthInBytes: Int {
        Int(tc_buffer_len_bytes(handle))
    }

    /// Length of the document in UTF-16 code units — the unit `NSString`,
    /// `NSRange`, and AppKit text views count in.
    public var lengthInUTF16: Int {
        Int(tc_buffer_len_utf16(handle))
    }

    /// The full document contents.
    public var text: String {
        guard let cString = tc_buffer_text(handle) else { return "" }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }

    /// Inserts `text` at a UTF-8 byte offset.
    public func insert(_ text: String, atByteOffset offset: Int) throws {
        let accepted = withUTF8Pointer(text) { pointer, length in
            tc_buffer_insert(handle, UInt(offset), pointer, length)
        }
        guard accepted else {
            throw CoreRejectedOperation(operation: "insert at byte \(offset)")
        }
    }

    /// Deletes the UTF-8 byte range `start..<end`.
    public func deleteBytes(from start: Int, to end: Int) throws {
        guard tc_buffer_delete(handle, UInt(start), UInt(end)) else {
            throw CoreRejectedOperation(operation: "delete bytes \(start)..<\(end)")
        }
    }

    /// Replaces an `NSRange` of UTF-16 code units with `text`.
    ///
    /// This maps one-to-one onto the ranges AppKit reports for text edits,
    /// so a view delegate can forward changes verbatim.
    public func replace(utf16Range range: NSRange, with text: String) throws {
        let accepted = withUTF8Pointer(text) { pointer, length in
            tc_buffer_replace_utf16(
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

    /// Runs `body` with a `(pointer, length)` view of the string's UTF-8.
    private func withUTF8Pointer<R>(
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
