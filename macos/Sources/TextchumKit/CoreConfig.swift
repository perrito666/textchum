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

    /// Where File → New places the fresh document; tabs by default.
    public var newFileTarget: CoreOpenTarget {
        get {
            tc_config_new_file_target(handle) == UInt32(TC_OPEN_IN_WINDOW) ? .window : .tab
        }
        set {
            tc_config_set_new_file_target(
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

    /// Whether the editor shows a line-number gutter (default true).
    public var lineNumbers: Bool {
        get { tc_config_line_numbers(handle) }
        set { tc_config_set_line_numbers(handle, newValue) }
    }

    /// Whether hover documentation pops up on mouse rest.
    public var hoverDocs: Bool {
        get { tc_config_hover_docs(handle) }
        set { tc_config_set_hover_docs(handle, newValue) }
    }

    /// The chosen theme name (the default theme's name when unset).
    public var theme: String {
        get {
            guard let cString = tc_config_theme(handle) else { return "Textchum" }
            defer { tc_string_free(cString) }
            return String(cString: cString)
        }
        set {
            var name = newValue
            name.withUTF8 { bytes in
                tc_config_set_theme(
                    handle,
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count)
                )
            }
        }
    }

    /// Keyboard-shortcut overrides as JSON: `{action: "modifiers+key"}`.
    public var keysJSON: String {
        guard let cString = tc_config_keys_json(handle) else { return "{}" }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }

    /// The workspace-behavior section: manifest-project and
    /// recursive-config flags, defaults plus per-root entries.
    public var workspaceJSON: String {
        guard let cString = tc_config_workspace_json(handle) else { return "{}" }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }

    /// Sets (nil removes) a workspace flag — "manifest_projects" or
    /// "recursive_config" — for a project root, or the defaults when
    /// `root` is nil.
    public func setWorkspaceFlag(root: String?, key: String, value: Bool?) {
        var root = root ?? ""
        var key = key
        root.withUTF8 { rootBytes in
            key.withUTF8 { keyBytes in
                tc_config_set_workspace_flag(
                    handle,
                    rootBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(rootBytes.count),
                    keyBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(keyBytes.count),
                    value != nil,
                    value ?? false
                )
            }
        }
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

    /// The save-preprocessor section, serialized:
    /// `{"defaults": {lang: [cmd, ...]}, "projects": {root: {lang: [...]}}}`.
    public var preprocessorsJSON: String {
        guard let cString = tc_config_preprocessors_json(handle) else { return "{}" }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }

    /// Sets (nil or blank removes) the save-preprocessor chain for a
    /// language — newline-separated command lines — scoped to a project
    /// root, or the defaults when `root` is nil.
    public func setPreprocessorEntry(root: String?, language: String, commands: String?) {
        var root = root ?? ""
        var language = language
        var commands = commands ?? ""
        root.withUTF8 { rootBytes in
            language.withUTF8 { languageBytes in
                commands.withUTF8 { commandBytes in
                    tc_config_set_preprocessor_entry(
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

    /// The resolved preprocessor chain for a language under a project
    /// root: the root's entry when it has one, the defaults otherwise.
    public func preprocessorCommands(root: String?, language: String) -> [String] {
        var root = root ?? ""
        var language = language
        let joined: String? = root.withUTF8 { rootBytes in
            language.withUTF8 { languageBytes in
                guard
                    let cString = tc_config_preprocessor_commands(
                        handle,
                        rootBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(rootBytes.count),
                        languageBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(languageBytes.count)
                    )
                else { return nil }
                defer { tc_string_free(cString) }
                return String(cString: cString)
            }
        }
        guard let joined, !joined.isEmpty else { return [] }
        return joined.split(separator: "\n").map(String.init)
    }

    /// The prose spell-check language: a spelling identifier like
    /// "en_US", "auto" for automatic detection, or nil when off.
    public var spellLanguage: String? {
        get {
            guard let cString = tc_config_spell_language(handle) else { return nil }
            defer { tc_string_free(cString) }
            let value = String(cString: cString)
            return value.isEmpty ? nil : value
        }
        set {
            var language = newValue ?? ""
            language.withUTF8 { bytes in
                tc_config_set_spell_language(
                    handle,
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count)
                )
            }
        }
    }

    /// Re-reads the file, replacing in-memory state — for following
    /// external edits while running. Returns a warning to show once when
    /// the file existed but was unusable.
    @discardableResult
    public func reload() -> String? {
        guard let cString = tc_config_reload(handle) else { return nil }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }

    /// Per-project editor overrides for a root, as a JSON object with
    /// any of `font_family`, `font_size`, `tab_width` (`{}` when none).
    public func editorOverridesJSON(root: String) -> String {
        var root = root
        return root.withUTF8 { bytes in
            guard
                let cString = tc_config_editor_overrides(
                    handle,
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count)
                )
            else { return "{}" }
            defer { tc_string_free(cString) }
            return String(cString: cString)
        }
    }

    /// Sets (nil removes) one per-project editor override; `valueJSON`
    /// is a JSON value — `13.5`, `"Menlo"`.
    public func setEditorOverride(root: String, key: String, valueJSON: String?) {
        var root = root
        var key = key
        var value = valueJSON ?? ""
        root.withUTF8 { rootBytes in
            key.withUTF8 { keyBytes in
                value.withUTF8 { valueBytes in
                    tc_config_set_editor_override(
                        handle,
                        rootBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(rootBytes.count),
                        keyBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(keyBytes.count),
                        valueBytes.baseAddress.map {
                            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                        },
                        UInt(valueBytes.count)
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
