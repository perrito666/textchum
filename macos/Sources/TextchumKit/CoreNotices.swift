import CTextchum
import Foundation

/// What the editor has had to say this session — a language server it
/// could not find, a grammar it could not load, a save that went ahead
/// without its preprocessors — kept by the core so the last word can
/// stay on the status bar and the rest can be read back. Both shells
/// keep one and draw it their own way.
///
/// Not thread-safe: use from the main thread.
public final class CoreNotices {
    private let handle: OpaquePointer

    /// One thing said, and when.
    public struct Entry: Equatable {
        public let at: Date
        public let text: String
    }

    public init() {
        handle = tc_notices_new()!
    }

    deinit {
        tc_notices_free(handle)
    }

    /// Says `text`. Said again straight after itself, it is not
    /// repeated; its time moves instead.
    public func push(_ text: String) {
        text.withCString { pointer in
            tc_notices_push(handle, pointer, UInt(strlen(pointer)))
        }
    }

    /// The most recent notice, or nil when nothing has been said.
    public var latest: String? {
        guard let raw = tc_notices_latest(handle) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    public var count: Int {
        Int(tc_notices_count(handle))
    }

    /// Everything said, newest first.
    public var entries: [Entry] {
        guard let raw = tc_notices_json(handle) else { return [] }
        defer { tc_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        let items = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] ?? []
        return items.compactMap { item in
            guard let text = item["text"] as? String else { return nil }
            let milliseconds = (item["at"] as? NSNumber)?.doubleValue ?? 0
            return Entry(at: Date(timeIntervalSince1970: milliseconds / 1000), text: text)
        }
    }
}
