import CTextchum
import Foundation

/// The application's configuration, backed by a JSON file the core owns.
///
/// The shell decides where the file lives (platform convention) and hands
/// the path in; the core does the parsing, clamping, merging, and atomic
/// writing. Two guarantees matter to callers:
///
/// * Loading always succeeds — defaults cover a missing, broken, or
///   partially valid file. When the file existed but was unusable,
///   ``loadWarning`` carries a message to show the user once; the broken
///   file stays untouched on disk and is backed up to `<name>.bak` before
///   the first save would overwrite it.
/// * Saving preserves JSON keys this version does not recognize, so hand
///   edits and future versions' settings survive the settings UI.
///
/// Not thread-safe: use from the main thread.
public final class CoreConfig {
    private let handle: OpaquePointer

    /// Human-readable warning from load time, if the file existed but could
    /// not be used. Surface it to the user once.
    public let loadWarning: String?

    /// Loads the configuration at `path` (which need not exist yet).
    public init(path: String) {
        var warning: UnsafeMutablePointer<CChar>?
        var path = path
        self.handle = path.withUTF8 { bytes in
            let pointer = bytes.baseAddress.map {
                UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
            }
            return tc_config_load(pointer, UInt(bytes.count), &warning)
        }!
        if let warning {
            self.loadWarning = String(cString: warning)
            tc_string_free(warning)
        } else {
            self.loadWarning = nil
        }
    }

    deinit {
        tc_config_free(handle)
    }

    /// Editor font family; nil means "use the platform monospaced font".
    public var fontFamily: String? {
        get {
            guard let cString = tc_config_font_family(handle) else { return nil }
            defer { tc_string_free(cString) }
            return String(cString: cString)
        }
        set {
            var family = newValue ?? ""
            family.withUTF8 { bytes in
                let pointer = bytes.baseAddress.map {
                    UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                }
                tc_config_set_font_family(handle, pointer, UInt(bytes.count))
            }
        }
    }

    /// Editor font size in points. The core clamps to its valid range.
    public var fontSize: Double {
        get { tc_config_font_size(handle) }
        set { tc_config_set_font_size(handle, newValue) }
    }

    /// Tab width in columns. The core clamps to its valid range.
    public var tabWidth: Int {
        get { Int(tc_config_tab_width(handle)) }
        set { tc_config_set_tab_width(handle, UInt32(max(0, newValue))) }
    }

    /// Writes the configuration back to its file (atomic, pretty-printed,
    /// unknown keys preserved).
    public func save() throws {
        var error: UnsafeMutablePointer<CChar>?
        guard tc_config_save(handle, &error) else {
            let message: String
            if let error {
                message = String(cString: error)
                tc_string_free(error)
            } else {
                message = "unknown error saving configuration"
            }
            throw CoreIOError(message: message)
        }
    }
}
