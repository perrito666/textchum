import CTextchum
import Foundation

/// The user's appearance choice: follow the system, or force a mode.
public enum CoreAppearance: CaseIterable {
    case system
    case light
    case dark
}

/// Where opening a file puts it: a tab of the current window's group, or
/// a separate window.
public enum CoreOpenTarget: CaseIterable {
    case tab
    case window
}

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

    /// Appearance choice. `system` is the default and keeps the app
    /// following macOS light/dark switches live.
    public var appearance: CoreAppearance {
        get {
            switch tc_config_appearance(handle) {
            case UInt32(TC_APPEARANCE_LIGHT): return .light
            case UInt32(TC_APPEARANCE_DARK): return .dark
            default: return .system
            }
        }
        set {
            let raw: UInt32
            switch newValue {
            case .system: raw = UInt32(TC_APPEARANCE_SYSTEM)
            case .light: raw = UInt32(TC_APPEARANCE_LIGHT)
            case .dark: raw = UInt32(TC_APPEARANCE_DARK)
            }
            tc_config_set_appearance(handle, raw)
        }
    }

    /// Where opened files go; tabs by default.
    public var openTarget: CoreOpenTarget {
        get {
            tc_config_open_target(handle) == UInt32(TC_OPEN_IN_WINDOW) ? .window : .tab
        }
        set {
            tc_config_set_open_target(
                handle,
                newValue == .window ? UInt32(TC_OPEN_IN_WINDOW) : UInt32(TC_OPEN_IN_TAB))
        }
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

    /// The language-server section, serialized for the pool:
    /// `{"defaults": {lang: cmdline}, "projects": {root: {lang: cmdline}}}`.
    public var lspJSON: String {
        guard let cString = tc_config_lsp_json(handle) else { return "{}" }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }

    /// Sets (nil removes) the server command line for a language — scoped
    /// to a project root, or the defaults when `root` is nil.
    public func setLSPEntry(root: String?, language: String, command: String?) {
        var root = root ?? ""
        var language = language
        var command = command ?? ""
        root.withUTF8 { rootBytes in
            language.withUTF8 { languageBytes in
                command.withUTF8 { commandBytes in
                    tc_config_set_lsp_entry(
                        handle,
                        rootBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(rootBytes.count),
                        languageBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(languageBytes.count),
                        commandBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(commandBytes.count)
                    )
                }
            }
        }
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
