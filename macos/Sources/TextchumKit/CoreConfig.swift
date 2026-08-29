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

    /// The chosen keyboard profile; empty means the editor's own
    /// bindings.
    public var keysProfile: String {
        get {
            guard let raw = tc_config_keys_profile(handle) else { return "" }
            defer { tc_string_free(raw) }
            return String(cString: raw)
        }
        set {
            newValue.withCString { pointer in
                tc_config_set_keys_profile(handle, pointer, UInt(strlen(pointer)))
            }
        }
    }

    /// The profiles saved in the configuration, as name to
    /// action-to-shortcut map.
    public var keyProfilesJSON: String {
        guard let raw = tc_config_key_profiles(handle) else { return "{}" }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The profiles that can be chosen: the bundled ones and the saved
    /// ones.
    public var keyProfileChoices: [(id: String, name: String)] {
        guard let raw = tc_config_key_profile_choices(handle) else { return [] }
        defer { tc_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        let parsed = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] ?? []
        return parsed.compactMap { item in
            guard let id = item["id"] as? String, let name = item["name"] as? String
            else { return nil }
            return (id, name)
        }
    }

    /// The bindings that actually apply: the profile's, with the
    /// overrides on top.
    public var effectiveKeys: [String: String] {
        guard let raw = tc_config_effective_keys(handle) else { return [:] }
        defer { tc_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: String] ?? [:]
    }

    /// Saves a profile, or removes it with a nil binding set.
    public func setKeyProfile(name: String, bindingsJSON: String?) {
        name.withCString { namePointer in
            guard let bindingsJSON else {
                tc_config_set_key_profile(handle, namePointer, UInt(strlen(namePointer)), nil, 0)
                return
            }
            bindingsJSON.withCString { bindingsPointer in
                tc_config_set_key_profile(
                    handle, namePointer, UInt(strlen(namePointer)),
                    bindingsPointer, UInt(strlen(bindingsPointer)))
            }
        }
    }

    /// Sets one shortcut override, or removes it with a nil spec.
    public func setKeyBinding(action: String, spec: String?) {
        action.withCString { actionPointer in
            guard let spec, !spec.isEmpty else {
                tc_config_set_key_binding(handle, actionPointer, UInt(strlen(actionPointer)), nil, 0)
                return
            }
            spec.withCString { specPointer in
                tc_config_set_key_binding(
                    handle, actionPointer, UInt(strlen(actionPointer)),
                    specPointer, UInt(strlen(specPointer)))
            }
        }
    }

    /// Forgets every shortcut override.
    public func clearKeyBindings() {
        tc_config_clear_key_bindings(handle)
    }

    /// Every project root the configuration mentions, in any section.
    public var configuredProjects: [String] {
        guard let raw = tc_config_configured_projects(handle) else { return [] }
        defer { tc_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        return (try? JSONSerialization.jsonObject(with: data)) as? [String] ?? []
    }

    /// Removes every trace of a project root: flags, editor overrides,
    /// hidden globs, servers and save commands.
    public func removeProject(root: String) {
        root.withCString { pointer in
            tc_config_remove_project(handle, pointer, UInt(strlen(pointer)))
        }
    }

    /// Copies one project's settings onto another root, taking the
    /// parts asked for. Returns whether anything was copied.
    @discardableResult
    public func copyProject(
        from: String, to: String,
        workspace: Bool = true, servers: Bool = true, preprocessors: Bool = true
    ) -> Bool {
        from.withCString { fromPointer in
            to.withCString { toPointer in
                tc_config_copy_project(
                    handle,
                    fromPointer, UInt(strlen(fromPointer)),
                    toPointer, UInt(strlen(toPointer)),
                    workspace, servers, preprocessors)
            }
        }
    }

    /// Whether selecting a word marks its other occurrences on screen.
    public var markOccurrences: Bool {
        get { tc_config_mark_occurrences(handle) }
        set { tc_config_set_mark_occurrences(handle, newValue) }
    }

    /// Whether those marks tell `Item` from `item`.
    public var occurrencesCaseSensitive: Bool {
        get { tc_config_occurrences_case_sensitive(handle) }
        set { tc_config_set_occurrences_case_sensitive(handle, newValue) }
    }

    /// Whether `item` inside `items` counts as an occurrence.
    public var occurrencesWholeWord: Bool {
        get { tc_config_occurrences_whole_word(handle) }
        set { tc_config_set_occurrences_whole_word(handle, newValue) }
    }

    /// The file-icon pack: a path to a VS Code icon theme JSON, or the
    /// extension folder holding one. Nil means the system's own icons.
    public var iconPack: String? {
        get {
            guard let cString = tc_config_icon_pack(handle) else { return nil }
            defer { tc_string_free(cString) }
            return String(cString: cString)
        }
        set {
            guard var path = newValue else {
                tc_config_set_icon_pack(handle, nil, 0)
                return
            }
            path.withUTF8 { bytes in
                tc_config_set_icon_pack(
                    handle,
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count)
                )
            }
        }
    }

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

    /// The spell setting split into the dictionaries it names. A
    /// bilingual document wants both at once, and the natural way to ask
    /// for that is "en_US, es_ES"; one dictionary is the one-element
    /// case, and "auto" stays a single entry.
    public var spellLanguages: [String] {
        guard let cString = tc_config_spell_languages_json(handle) else { return [] }
        defer { tc_string_free(cString) }
        let json = String(cString: cString)
        return (try? JSONDecoder().decode([String].self, from: Data(json.utf8))) ?? []
    }

    /// Words the spell checker accepts whatever the dictionary says:
    /// project names, acronyms, and the rest of the vocabulary no
    /// dictionary ships with.
    public var spellWords: [String] {
        get {
            guard let cString = tc_config_spell_words_json(handle) else { return [] }
            defer { tc_string_free(cString) }
            let json = String(cString: cString)
            return (try? JSONDecoder().decode([String].self, from: Data(json.utf8))) ?? []
        }
        set {
            guard let data = try? JSONEncoder().encode(newValue),
                  var json = String(data: data, encoding: .utf8)
            else { return }
            json.withUTF8 { bytes in
                tc_config_set_spell_words_json(
                    handle,
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count)
                )
            }
        }
    }

    /// Adds one word to the personal list. Returns whether it was new,
    /// so a caller can skip a re-check that would change nothing.
    @discardableResult
    public func addSpellWord(_ word: String) -> Bool {
        var word = word
        return word.withUTF8 { bytes in
            tc_config_add_spell_word(
                handle,
                bytes.baseAddress.map {
                    UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                },
                UInt(bytes.count)
            )
        }
    }

    /// Seconds of quiet before the editor saves by itself; zero means
    /// off, which is the default.
    public var autosaveSeconds: UInt32 {
        get { tc_config_autosave_seconds(handle) }
        set { tc_config_set_autosave_seconds(handle, newValue) }
    }

    /// What a document has been told about itself, overriding what its
    /// name implies. Absent fields mean the usual answer applies.
    public struct FileOverride: Equatable, Codable {
        public var language: String?
        public var tabWidth: UInt32?
        /// True for spaces, false for tabs.
        public var spaces: Bool?

        public init(language: String? = nil, tabWidth: UInt32? = nil, spaces: Bool? = nil) {
            self.language = language
            self.tabWidth = tabWidth
            self.spaces = spaces
        }

        public var isEmpty: Bool {
            language == nil && tabWidth == nil && spaces == nil
        }

        enum CodingKeys: String, CodingKey {
            case language
            case tabWidth = "tab_width"
            case spaces
        }
    }

    public func fileOverride(path: String) -> FileOverride {
        var path = path
        let json: String? = path.withUTF8 { bytes in
            guard
                let cString = tc_config_file_override_json(
                    handle,
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count)
                )
            else { return nil }
            defer { tc_string_free(cString) }
            return String(cString: cString)
        }
        guard let json, let data = json.data(using: .utf8) else { return FileOverride() }
        return (try? JSONDecoder().decode(FileOverride.self, from: data)) ?? FileOverride()
    }

    /// Records what a document is. An override with nothing in it
    /// forgets the file.
    public func setFileOverride(path: String, _ entry: FileOverride) {
        guard let data = try? JSONEncoder().encode(entry),
            var json = String(data: data, encoding: .utf8)
        else { return }
        var path = path
        path.withUTF8 { pathBytes in
            json.withUTF8 { jsonBytes in
                tc_config_set_file_override_json(
                    handle,
                    pathBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(pathBytes.count),
                    jsonBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(jsonBytes.count)
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

    /// The navigator's hidden-name globs for a root (nil = the
    /// defaults). `[".*"]` when nothing is configured.
    public func hiddenGlobs(root: String?) -> [String] {
        var root = root ?? ""
        return root.withUTF8 { bytes in
            guard
                let cString = tc_config_hide_globs(
                    handle,
                    bytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(bytes.count)
                )
            else { return [".*"] }
            defer { tc_string_free(cString) }
            return String(cString: cString).split(separator: "\n").map(String.init)
        }
    }

    /// Sets (nil or blank removes) the hidden-name globs — whitespace
    /// separated — for a root, or the defaults when `root` is nil.
    public func setHiddenGlobs(root: String?, globs: String?) {
        var root = root ?? ""
        var globs = globs ?? ""
        root.withUTF8 { rootBytes in
            globs.withUTF8 { globBytes in
                tc_config_set_hide_globs(
                    handle,
                    rootBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(rootBytes.count),
                    globBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(globBytes.count)
                )
            }
        }
    }

    /// The hidden-glob presets the settings UI offers, sorted by name.
    public var hidePresets: [(name: String, globs: [String])] {
        guard let cString = tc_config_hide_presets(handle) else { return [] }
        defer { tc_string_free(cString) }
        return String(cString: cString)
            .split(separator: "\n")
            .compactMap { line in
                let halves = line.split(separator: "\u{1f}", maxSplits: 1)
                guard let name = halves.first, !name.isEmpty else { return nil }
                let globs =
                    halves.count > 1
                    ? halves[1].split(separator: " ").map(String.init) : []
                return (String(name), globs)
            }
    }

    /// Sets (nil or blank removes) one preset by name.
    public func setHidePreset(name: String, globs: String?) {
        var name = name
        var globs = globs ?? ""
        name.withUTF8 { nameBytes in
            globs.withUTF8 { globBytes in
                tc_config_set_hide_preset(
                    handle,
                    nameBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(nameBytes.count),
                    globBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(globBytes.count)
                )
            }
        }
    }

    /// Forgets the user's presets, restoring the built-ins.
    public func resetHidePresets() {
        tc_config_reset_hide_presets(handle)
    }

    /// Whether the navigator reveals the current file as focus moves.
    public var followFile: Bool {
        get { tc_config_follow_file(handle) }
        set { tc_config_set_follow_file(handle, newValue) }
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
