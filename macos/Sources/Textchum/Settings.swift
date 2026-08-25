import AppKit
import SwiftUI
import TextchumKit

/// The editor-facing result of the current configuration: concrete values
/// windows can apply without knowing where they came from.
struct EditorSettings {
    let font: NSFont
    let tabWidth: Int
    let lineNumbers: Bool
    let hoverDocs: Bool
    let spellLanguage: String?

    /// Resolves configuration values into a usable font: the configured
    /// family if it exists on this system, the platform monospaced font
    /// otherwise. A project root's `editor` overrides (font family,
    /// size, tab width) win over the globals for windows inside it.
    init(config: CoreConfig, projectRoot: String? = nil) {
        var family = config.fontFamily
        var size = config.fontSize
        var tabWidth = config.tabWidth
        if let projectRoot,
            let data = config.editorOverridesJSON(root: projectRoot).data(using: .utf8),
            let overrides = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        {
            if let value = overrides["font_family"] as? String, !value.isEmpty {
                family = value
            }
            if let value = overrides["font_size"] as? Double { size = value }
            if let value = overrides["tab_width"] as? Int { tabWidth = value }
        }
        if let family, let font = NSFont(name: family, size: size) {
            self.font = font
        } else {
            self.font = .monospacedSystemFont(ofSize: size, weight: .regular)
        }
        self.tabWidth = tabWidth
        self.lineNumbers = config.lineNumbers
        self.hoverDocs = config.hoverDocs
        self.spellLanguage = config.spellLanguage
    }
}

/// Observable bridge between the settings UI and the JSON-backed core
/// configuration: every change writes through to disk immediately and
/// notifies the app so open editors update live.
@MainActor
final class SettingsModel: ObservableObject {
    /// Which settings tab is frontmost; a tag from `SettingsView`.
    @Published var selectedTab = "general"

    /// One language-server configuration row.
    struct LSPEntry: Identifiable, Equatable {
        /// Project root path; empty means the defaults section.
        let scope: String
        let language: String
        let command: String

        var id: String { "\(scope)|\(language)" }
        var scopeLabel: String {
            scope.isEmpty ? "Default" : (scope as NSString).lastPathComponent
        }
    }

    private let config: CoreConfig
    /// Called after any setting changed and was persisted.
    var onChange: (() -> Void)?
    /// Called when the user asks running servers to restart.
    var onRestartServers: (() -> Void)?
    /// Suppresses write-back while the initial values load.
    private var isLoading = true

    @Published var appearance: CoreAppearance {
        didSet { persist { $0.appearance = appearance } }
    }
    @Published var theme: String {
        didSet { persist { $0.theme = theme } }
    }
    @Published var openTarget: CoreOpenTarget {
        didSet { persist { $0.openTarget = openTarget } }
    }
    @Published var fontFamily: String {
        didSet { persist { $0.fontFamily = fontFamily.isEmpty ? nil : fontFamily } }
    }
    @Published var fontSize: Double {
        didSet { persist { $0.fontSize = fontSize } }
    }
    @Published var tabWidth: Int {
        didSet { persist { $0.tabWidth = tabWidth } }
    }
    @Published var lineNumbers: Bool {
        didSet { persist { $0.lineNumbers = lineNumbers } }
    }
    @Published var hoverDocs: Bool {
        didSet { persist { $0.hoverDocs = hoverDocs } }
    }
    /// Prose spell-check choice: "" = off, "auto", or a language code.
    @Published var spellLanguage: String {
        didSet { persist { $0.spellLanguage = spellLanguage.isEmpty ? nil : spellLanguage } }
    }
    @Published private(set) var lspEntries: [LSPEntry] = []

    /// One workspace-behavior row (a project root, its flags, and any
    /// editor overrides — empty strings mean "inherit the global").
    struct WorkspaceEntry: Identifiable, Equatable {
        let scope: String
        var manifestProjects: Bool
        var recursiveConfig: Bool
        var ctagsFallback: Bool
        var fontFamily: String = ""
        var fontSize: String = ""
        var tabWidth: String = ""
        var id: String { scope }
        var scopeLabel: String { (scope as NSString).lastPathComponent }
    }

    @Published var manifestProjectsDefault = false {
        didSet {
            guard !isLoading else { return }
            config.setWorkspaceFlag(
                root: nil, key: "manifest_projects", value: manifestProjectsDefault)
            persistLSPChange()
        }
    }
    @Published var recursiveConfigDefault = false {
        didSet {
            guard !isLoading else { return }
            config.setWorkspaceFlag(
                root: nil, key: "recursive_config", value: recursiveConfigDefault)
            persistLSPChange()
        }
    }
    @Published var ctagsFallbackDefault = false {
        didSet {
            guard !isLoading else { return }
            config.setWorkspaceFlag(
                root: nil, key: "ctags_fallback", value: ctagsFallbackDefault)
            persistLSPChange()
        }
    }
    @Published private(set) var workspaceEntries: [WorkspaceEntry] = []

    /// Built-ins plus the user's theme files; a user file sharing a
    /// built-in's name shows once (the file wins when applied).
    var availableThemes: [String] {
        var names = CoreTheme.builtinNames
        for user in ThemeFiles.names where !names.contains(user) {
            names.append(user)
        }
        return names
    }

    /// Re-publishes every value from the (just-reloaded) configuration —
    /// the settings window follows external edits like everything else.
    func reloadFromConfig() {
        isLoading = true
        appearance = config.appearance
        theme = config.theme
        openTarget = config.openTarget
        fontFamily = config.fontFamily ?? ""
        fontSize = config.fontSize
        tabWidth = config.tabWidth
        lineNumbers = config.lineNumbers
        hoverDocs = config.hoverDocs
        spellLanguage = config.spellLanguage ?? ""
        isLoading = false
        reloadLSPEntries()
        reloadWorkspaceEntries()
        reloadPreprocessorEntries()
    }

    init(config: CoreConfig) {
        self.config = config
        self.appearance = config.appearance
        self.theme = config.theme
        self.openTarget = config.openTarget
        self.fontFamily = config.fontFamily ?? ""
        self.fontSize = config.fontSize
        self.tabWidth = config.tabWidth
        self.lineNumbers = config.lineNumbers
        self.hoverDocs = config.hoverDocs
        self.spellLanguage = config.spellLanguage ?? ""
        self.isLoading = false
        reloadLSPEntries()
        reloadWorkspaceEntries()
        reloadPreprocessorEntries()
    }

    // MARK: Workspace behavior

    private func reloadWorkspaceEntries() {
        isLoading = true
        defer { isLoading = false }
        var entries: [WorkspaceEntry] = []
        if let data = config.workspaceJSON.data(using: .utf8),
            let parsed = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        {
            manifestProjectsDefault = parsed["manifest_projects"] as? Bool ?? false
            recursiveConfigDefault = parsed["recursive_config"] as? Bool ?? false
            ctagsFallbackDefault = parsed["ctags_fallback"] as? Bool ?? false
            for (root, raw) in parsed["projects"] as? [String: [String: Any]] ?? [:] {
                let editor = raw["editor"] as? [String: Any] ?? [:]
                entries.append(
                    WorkspaceEntry(
                        scope: root,
                        manifestProjects: raw["manifest_projects"] as? Bool
                            ?? manifestProjectsDefault,
                        recursiveConfig: raw["recursive_config"] as? Bool
                            ?? recursiveConfigDefault,
                        ctagsFallback: raw["ctags_fallback"] as? Bool
                            ?? ctagsFallbackDefault,
                        fontFamily: editor["font_family"] as? String ?? "",
                        fontSize: (editor["font_size"] as? Double).map { size in
                            size == size.rounded() ? String(Int(size)) : String(size)
                        } ?? "",
                        tabWidth: (editor["tab_width"] as? Int).map(String.init) ?? ""
                    ))
            }
        }
        workspaceEntries = entries.sorted { $0.scope < $1.scope }
    }

    func addWorkspaceEntry(scope: String) {
        let scope = (scope as NSString).expandingTildeInPath
        guard !scope.isEmpty else { return }
        config.setWorkspaceFlag(
            root: scope, key: "manifest_projects", value: manifestProjectsDefault)
        config.setWorkspaceFlag(
            root: scope, key: "recursive_config", value: recursiveConfigDefault)
        config.setWorkspaceFlag(
            root: scope, key: "ctags_fallback", value: ctagsFallbackDefault)
        persistWorkspaceChange()
    }

    func setWorkspaceFlag(scope: String, key: String, value: Bool) {
        config.setWorkspaceFlag(root: scope, key: key, value: value)
        persistWorkspaceChange()
    }

    func removeWorkspaceEntry(_ entry: WorkspaceEntry) {
        config.setWorkspaceFlag(root: entry.scope, key: "manifest_projects", value: nil)
        config.setWorkspaceFlag(root: entry.scope, key: "recursive_config", value: nil)
        config.setWorkspaceFlag(root: entry.scope, key: "ctags_fallback", value: nil)
        persistWorkspaceChange()
    }

    private func persistWorkspaceChange() {
        do {
            try config.save()
        } catch {
            NSLog("could not save configuration: \(error)")
        }
        reloadWorkspaceEntries()
        onChange?()
    }

    var workspaceJSON: String { config.workspaceJSON }

    var currentSettings: EditorSettings {
        EditorSettings(config: config)
    }

    /// The effective settings for a window whose document lives under
    /// `root` — per-project overrides applied over the globals.
    func currentSettings(forRoot root: String?) -> EditorSettings {
        EditorSettings(config: config, projectRoot: root)
    }

    /// One per-project editor override changed (a JSON value, nil
    /// removes); persists and reapplies everywhere.
    func setEditorOverride(scope: String, key: String, valueJSON: String?) {
        config.setEditorOverride(root: scope, key: key, valueJSON: valueJSON)
        persistWorkspaceChange()
    }

    // MARK: Language servers

    private func reloadLSPEntries() {
        var entries: [LSPEntry] = []
        if let data = config.lspJSON.data(using: .utf8),
            let parsed = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        {
            for (language, command) in parsed["defaults"] as? [String: String] ?? [:] {
                entries.append(LSPEntry(scope: "", language: language, command: command))
            }
            for (root, languages) in parsed["projects"] as? [String: [String: String]] ?? [:] {
                for (language, command) in languages {
                    entries.append(LSPEntry(scope: root, language: language, command: command))
                }
            }
        }
        lspEntries = entries.sorted { ($0.scope, $0.language) < ($1.scope, $1.language) }
    }

    func addLSPEntry(scope: String, language: String, command: String) {
        let language = language.trimmingCharacters(in: .whitespaces).lowercased()
        guard !language.isEmpty, !command.trimmingCharacters(in: .whitespaces).isEmpty else {
            return
        }
        config.setLSPEntry(
            root: scope.isEmpty ? nil : (scope as NSString).expandingTildeInPath,
            language: language,
            command: command
        )
        persistLSPChange()
    }

    /// Rewrites an existing override's command in place — same (scope,
    /// language) key, new command line.
    func updateLSPEntry(_ entry: LSPEntry, command: String) {
        let command = command.trimmingCharacters(in: .whitespaces)
        guard !command.isEmpty else { return }
        config.setLSPEntry(
            root: entry.scope.isEmpty ? nil : entry.scope,
            language: entry.language,
            command: command
        )
        persistLSPChange()
    }

    func removeLSPEntry(_ entry: LSPEntry) {
        config.setLSPEntry(
            root: entry.scope.isEmpty ? nil : entry.scope,
            language: entry.language,
            command: nil
        )
        persistLSPChange()
    }

    private func persistLSPChange() {
        do {
            try config.save()
        } catch {
            NSLog("could not save configuration: \(error)")
        }
        reloadLSPEntries()
        onChange?()
    }

    // MARK: Save preprocessors

    /// One preprocessor row: a language's command chain, one command
    /// per line, for every project (empty scope) or one root.
    struct PreprocessorEntry: Identifiable, Equatable {
        let scope: String
        let language: String
        /// Newline-joined for the multi-line editor.
        let commands: String

        var id: String { "\(scope)|\(language)" }
        var scopeLabel: String {
            scope.isEmpty ? "Default" : (scope as NSString).lastPathComponent
        }
    }

    @Published private(set) var preprocessorEntries: [PreprocessorEntry] = []

    private func reloadPreprocessorEntries() {
        var entries: [PreprocessorEntry] = []
        func commandLines(_ raw: Any?) -> String? {
            if let one = raw as? String { return one }
            if let many = raw as? [String] { return many.joined(separator: "\n") }
            return nil
        }
        if let data = config.preprocessorsJSON.data(using: .utf8),
            let parsed = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        {
            for (language, raw) in parsed["defaults"] as? [String: Any] ?? [:] {
                guard let commands = commandLines(raw) else { continue }
                entries.append(
                    PreprocessorEntry(scope: "", language: language, commands: commands))
            }
            for (root, languages) in parsed["projects"] as? [String: [String: Any]] ?? [:] {
                for (language, raw) in languages {
                    guard let commands = commandLines(raw) else { continue }
                    entries.append(
                        PreprocessorEntry(scope: root, language: language, commands: commands))
                }
            }
        }
        preprocessorEntries = entries.sorted { ($0.scope, $0.language) < ($1.scope, $1.language) }
    }

    func addPreprocessorEntry(scope: String, language: String, commands: String) {
        let language = language.trimmingCharacters(in: .whitespaces).lowercased()
        guard !language.isEmpty,
            !commands.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else { return }
        config.setPreprocessorEntry(
            root: scope.isEmpty ? nil : (scope as NSString).expandingTildeInPath,
            language: language,
            commands: commands
        )
        persistPreprocessorChange()
    }

    func updatePreprocessorEntry(_ entry: PreprocessorEntry, commands: String) {
        guard !commands.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        config.setPreprocessorEntry(
            root: entry.scope.isEmpty ? nil : entry.scope,
            language: entry.language,
            commands: commands
        )
        persistPreprocessorChange()
    }

    func removePreprocessorEntry(_ entry: PreprocessorEntry) {
        config.setPreprocessorEntry(
            root: entry.scope.isEmpty ? nil : entry.scope,
            language: entry.language,
            commands: nil
        )
        persistPreprocessorChange()
    }

    private func persistPreprocessorChange() {
        do {
            try config.save()
        } catch {
            NSLog("could not save configuration: \(error)")
        }
        reloadPreprocessorEntries()
        onChange?()
    }

    var lspJSON: String { config.lspJSON }

    private func persist(_ apply: (CoreConfig) -> Void) {
        guard !isLoading else { return }
        apply(config)
        do {
            try config.save()
        } catch {
            NSLog("could not save configuration: \(error)")
        }
        onChange?()
    }
}

/// The settings window: General plus Language Servers.
struct SettingsView: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        TabView(selection: $model.selectedTab) {
            GeneralSettingsTab(model: model)
                .tabItem { Label("General", systemImage: "gearshape") }
                .tag("general")
            ProjectsTab(model: model)
                .tabItem { Label("Projects", systemImage: "folder.badge.gearshape") }
                .tag("projects")
            LanguageServersTab(model: model)
                .tabItem { Label("Language Servers", systemImage: "network") }
                .tag("servers")
            PreprocessorsTab(model: model)
                .tabItem { Label("Preprocessors", systemImage: "wand.and.rays") }
                .tag("preprocessors")
        }
        .frame(width: 640, height: 480)
        .padding(20)
    }
}

private struct GeneralSettingsTab: View {
    @ObservedObject var model: SettingsModel

    /// Font families with a fixed-pitch face, plus the platform default.
    private var monospacedFamilies: [String] {
        NSFontManager.shared.availableFontFamilies.filter { family in
            guard let font = NSFont(name: family, size: 12) else { return false }
            return font.isFixedPitch
        }
    }

    var body: some View {
        Form {
            Picker("Appearance:", selection: $model.appearance) {
                Text("System").tag(CoreAppearance.system)
                Text("Light").tag(CoreAppearance.light)
                Text("Dark").tag(CoreAppearance.dark)
            }
            .pickerStyle(.segmented)
            Picker("Theme:", selection: $model.theme) {
                ForEach(model.availableThemes, id: \.self) { name in
                    Text(name).tag(name)
                }
            }
            Picker("Open files in:", selection: $model.openTarget) {
                Text("Tabs").tag(CoreOpenTarget.tab)
                Text("Windows").tag(CoreOpenTarget.window)
            }
            .pickerStyle(.segmented)
            Picker("Font:", selection: $model.fontFamily) {
                Text("System Monospaced").tag("")
                Divider()
                ForEach(monospacedFamilies, id: \.self) { family in
                    Text(family).tag(family)
                }
            }
            Stepper(value: $model.fontSize, in: 6...72, step: 1) {
                Text("Font size: \(Int(model.fontSize)) pt")
            }
            Stepper(value: $model.tabWidth, in: 1...16) {
                Text("Tab width: \(model.tabWidth) columns")
            }
            Toggle("Show line numbers", isOn: $model.lineNumbers)
            Toggle("Hover documentation", isOn: $model.hoverDocs)
            Picker("Spell check prose", selection: $model.spellLanguage) {
                Text("Off").tag("")
                Text("Automatic by content").tag("auto")
                Divider()
                ForEach(NSSpellChecker.shared.availableLanguages, id: \.self) { language in
                    Text(Locale.current.localizedString(forIdentifier: language) ?? language)
                        .tag(language)
                }
            }
            Text("Checks comments in code, and everything in Markdown, git commit messages, and plain text.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 28)
        .padding(.vertical, 20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
}

/// A small inherit-when-empty field for per-project editor overrides:
/// commits on ⏎ or focus loss, and an emptied field removes the
/// override (that is its meaning, unlike the command fields).
private struct OverrideField: View {
    let placeholder: String
    let width: CGFloat
    let initial: String
    let commit: (String) -> Void
    @State private var text: String
    @FocusState private var focused: Bool

    init(
        placeholder: String, width: CGFloat, initial: String,
        commit: @escaping (String) -> Void
    ) {
        self.placeholder = placeholder
        self.width = width
        self.initial = initial
        self.commit = commit
        _text = State(initialValue: initial)
    }

    var body: some View {
        TextField(placeholder, text: $text)
            .textFieldStyle(.roundedBorder)
            .font(.caption)
            .frame(width: width)
            .focused($focused)
            .onSubmit(commitIfChanged)
            .onChange(of: focused) { _, isFocused in
                if !isFocused { commitIfChanged() }
            }
    }

    private func commitIfChanged() {
        let trimmed = text.trimmingCharacters(in: .whitespaces)
        guard trimmed != initial else { return }
        commit(trimmed)
    }
}

private struct ProjectsTab: View {
    @ObservedObject var model: SettingsModel
    @State private var newScope = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(
                "How projects are detected. By default a repository is one project; "
                    + "\"manifest projects\" lets nested language manifests (pyproject.toml, "
                    + "Cargo.toml, …) split it into sub-projects, and \"recursive config\" "
                    + "makes a root's per-project settings apply to the nested projects "
                    + "beneath it. \"Ctags fallback\" answers Jump to Definition from a "
                    + "Universal Ctags index when no language server is available."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            GroupBox("Defaults (all projects)") {
                HStack(spacing: 24) {
                    Toggle("Manifest projects", isOn: $model.manifestProjectsDefault)
                    Toggle("Recursive config", isOn: $model.recursiveConfigDefault)
                    Toggle("Ctags fallback", isOn: $model.ctagsFallbackDefault)
                    Spacer()
                }
                .padding(6)
            }

            List {
                if model.workspaceEntries.isEmpty {
                    Text("No per-project overrides.")
                        .foregroundStyle(.secondary)
                }
                ForEach(model.workspaceEntries) { entry in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack(spacing: 12) {
                            Text(entry.scopeLabel)
                                .fontWeight(.semibold)
                                .help(entry.scope)
                            Spacer()
                            Toggle(
                                "Manifest projects",
                                isOn: Binding(
                                    get: { entry.manifestProjects },
                                    set: {
                                        model.setWorkspaceFlag(
                                            scope: entry.scope, key: "manifest_projects",
                                            value: $0)
                                    }
                                ))
                            Toggle(
                                "Recursive config",
                                isOn: Binding(
                                    get: { entry.recursiveConfig },
                                    set: {
                                        model.setWorkspaceFlag(
                                            scope: entry.scope, key: "recursive_config",
                                            value: $0)
                                    }
                                ))
                            Toggle(
                                "Ctags fallback",
                                isOn: Binding(
                                    get: { entry.ctagsFallback },
                                    set: {
                                        model.setWorkspaceFlag(
                                            scope: entry.scope, key: "ctags_fallback",
                                            value: $0)
                                    }
                                ))
                            Button {
                                model.removeWorkspaceEntry(entry)
                            } label: {
                                Image(systemName: "minus.circle")
                            }
                            .buttonStyle(.borderless)
                        }
                        // Editor overrides for windows inside this root;
                        // empty fields inherit the General tab's values.
                        HStack(spacing: 8) {
                            Text("Editor:")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            OverrideField(
                                placeholder: "font family", width: 140,
                                initial: entry.fontFamily
                            ) { text in
                                model.setEditorOverride(
                                    scope: entry.scope, key: "font_family",
                                    valueJSON: text.isEmpty
                                        ? nil
                                        : "\"\(text.replacingOccurrences(of: "\"", with: ""))\"")
                            }
                            OverrideField(
                                placeholder: "size", width: 52, initial: entry.fontSize
                            ) { text in
                                model.setEditorOverride(
                                    scope: entry.scope, key: "font_size",
                                    valueJSON: Double(text).map { String($0) })
                            }
                            OverrideField(
                                placeholder: "tab width", width: 72, initial: entry.tabWidth
                            ) { text in
                                model.setEditorOverride(
                                    scope: entry.scope, key: "tab_width",
                                    valueJSON: Int(text).map(String.init))
                            }
                        }
                    }
                }
            }
            .frame(minHeight: 120)

            GroupBox("Add project override") {
                HStack(spacing: 8) {
                    PathPicker(text: $newScope, placeholder: "Project root path")
                    Button("Add") {
                        model.addWorkspaceEntry(scope: newScope)
                        newScope = ""
                    }
                    .disabled(newScope.isEmpty)
                }
                .padding(6)
            }
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 16)
    }
}

/// One override row's command line, editable in place. Commits on ⏎ or
/// when focus leaves; an emptied field reverts (removal has its own
/// button). Local state buffers typing so the file is written once per
/// edit, not once per keystroke.
private struct CommandField: View {
    let entry: SettingsModel.LSPEntry
    let commit: (String) -> Void
    @State private var text: String
    @FocusState private var focused: Bool

    init(entry: SettingsModel.LSPEntry, commit: @escaping (String) -> Void) {
        self.entry = entry
        self.commit = commit
        _text = State(initialValue: entry.command)
    }

    var body: some View {
        TextField("server command", text: $text)
            .textFieldStyle(.roundedBorder)
            .font(.system(.caption, design: .monospaced))
            .focused($focused)
            .onSubmit(commitIfChanged)
            .onChange(of: focused) { _, isFocused in
                if !isFocused { commitIfChanged() }
            }
            .onDisappear(perform: commitIfChanged)
    }

    private func commitIfChanged() {
        let trimmed = text.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else {
            text = entry.command
            return
        }
        guard trimmed != entry.command else { return }
        commit(trimmed)
    }
}

private struct LanguageServersTab: View {
    @ObservedObject var model: SettingsModel
    @State private var newScope = ""
    @State private var newLanguage = ""
    @State private var newCommand = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(
                "Override which server command runs per language — for every project "
                    + "(Default) or for one project root. Project entries win over defaults; "
                    + "unlisted languages use the built-in registry."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            List {
                if model.lspEntries.isEmpty {
                    Text("No overrides — the built-in registry serves all languages.")
                        .foregroundStyle(.secondary)
                }
                ForEach(model.lspEntries) { entry in
                    HStack(alignment: .center, spacing: 8) {
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text(entry.scopeLabel)
                                    .fontWeight(.semibold)
                                Text(entry.language)
                                    .foregroundStyle(.secondary)
                            }
                            // Editable in place: ⏎ or clicking away
                            // applies; scope and language are the entry's
                            // identity and stay fixed.
                            CommandField(entry: entry) { command in
                                model.updateLSPEntry(entry, command: command)
                            }
                        }
                        .help(entry.scope.isEmpty ? "All projects" : entry.scope)
                        Spacer()
                        Button {
                            model.removeLSPEntry(entry)
                        } label: {
                            Image(systemName: "minus.circle")
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
            .frame(minHeight: 140)

            GroupBox("Add override") {
                VStack(spacing: 8) {
                    PathPicker(
                        text: $newScope,
                        placeholder: "Project root (empty = default for all projects)")
                    HStack(spacing: 8) {
                        TextField("Language (e.g. python)", text: $newLanguage)
                            .frame(width: 180)
                        TextField(
                            "Server command (e.g. pyright-langserver --stdio)",
                            text: $newCommand)
                        Button("Add") {
                            model.addLSPEntry(
                                scope: newScope, language: newLanguage, command: newCommand)
                            newScope = ""
                            newLanguage = ""
                            newCommand = ""
                        }
                        .disabled(newLanguage.isEmpty || newCommand.isEmpty)
                    }
                }
                .textFieldStyle(.roundedBorder)
                .padding(6)
            }

            HStack {
                Text("Changes apply to servers started afterwards.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Restart Servers Now") {
                    model.onRestartServers?()
                }
            }
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 16)
    }
}

/// Save preprocessors get their own tab: chains grow multi-line, and
/// sharing a 480-point tab with the server table pushed the tab bar
/// clean out of the window.
private struct PreprocessorsTab: View {
    @ObservedObject var model: SettingsModel
    @State private var newScope = ""
    @State private var newLanguage = ""
    @State private var newCommands = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(
                "Save preprocessors run before every save (and on Run Save Preprocessors), "
                    + "one command per line in order — each reads the document on standard "
                    + "input and writes it back on standard output, like `ruff check --fix -` "
                    + "then `black -`. {path} and {filename} expand to the document's. "
                    + "A project entry replaces the default chain."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            List {
                if model.preprocessorEntries.isEmpty {
                    Text("No preprocessors — documents save exactly as typed.")
                        .foregroundStyle(.secondary)
                }
                ForEach(model.preprocessorEntries) { entry in
                    HStack(alignment: .top, spacing: 8) {
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text(entry.scopeLabel)
                                    .fontWeight(.semibold)
                                Text(entry.language)
                                    .foregroundStyle(.secondary)
                            }
                            CommandsField(entry: entry) { commands in
                                model.updatePreprocessorEntry(entry, commands: commands)
                            }
                        }
                        .help(entry.scope.isEmpty ? "All projects" : entry.scope)
                        Spacer()
                        Button {
                            model.removePreprocessorEntry(entry)
                        } label: {
                            Image(systemName: "minus.circle")
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
            .frame(minHeight: 160)

            GroupBox("Add preprocessor chain") {
                VStack(spacing: 8) {
                    PathPicker(
                        text: $newScope,
                        placeholder: "Project root (empty = default for all projects)")
                    HStack(alignment: .top, spacing: 8) {
                        TextField("Language (e.g. python)", text: $newLanguage)
                            .frame(width: 180)
                        CommandsEditor(
                            placeholder: "Commands, one per line — Return adds a line",
                            text: $newCommands)
                        Button("Add") {
                            model.addPreprocessorEntry(
                                scope: newScope,
                                language: newLanguage,
                                commands: newCommands)
                            newScope = ""
                            newLanguage = ""
                            newCommands = ""
                        }
                        .disabled(newLanguage.isEmpty || newCommands.isEmpty)
                    }
                }
                .textFieldStyle(.roundedBorder)
                .padding(6)
            }
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 16)
    }
}

/// A real multi-line editor for command chains: Return adds a line
/// (a vertical TextField would submit instead), the height follows
/// the content, and the caption explains the one-command-per-line
/// contract. Commits when focus leaves.
struct CommandsEditor: View {
    let placeholder: String
    @Binding var text: String
    var onFocusLost: () -> Void = {}
    @FocusState private var focused: Bool

    var body: some View {
        ZStack(alignment: .topLeading) {
            TextEditor(text: $text)
                .font(.system(.caption, design: .monospaced))
                .frame(minHeight: 38, maxHeight: 110)
                .fixedSize(horizontal: false, vertical: true)
                .scrollContentBackground(.hidden)
                .padding(4)
                .background(
                    RoundedRectangle(cornerRadius: 6)
                        .fill(Color(nsColor: .textBackgroundColor)))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .strokeBorder(Color(nsColor: .separatorColor)))
                .focused($focused)
                .onChange(of: focused) { _, isFocused in
                    if !isFocused { onFocusLost() }
                }
            if text.isEmpty {
                Text(placeholder)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .padding(.top, 8)
                    .padding(.leading, 10)
                    .allowsHitTesting(false)
            }
        }
    }
}

/// A preprocessor chain, editable in place — one command per line,
/// Return adds one. Commits when focus leaves; an emptied editor
/// reverts (removal has its own button).
private struct CommandsField: View {
    let entry: SettingsModel.PreprocessorEntry
    let commit: (String) -> Void
    @State private var text: String

    init(entry: SettingsModel.PreprocessorEntry, commit: @escaping (String) -> Void) {
        self.entry = entry
        self.commit = commit
        _text = State(initialValue: entry.commands)
    }

    var body: some View {
        CommandsEditor(placeholder: "commands, one per line", text: $text) {
            commitIfChanged()
        }
        .onDisappear(perform: commitIfChanged)
    }

    private func commitIfChanged() {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            text = entry.commands
            return
        }
        guard trimmed != entry.commands else { return }
        commit(trimmed)
    }
}

/// A plain titled window hosting the settings form.
final class SettingsWindowController: NSWindowController {
    init(model: SettingsModel) {
        let window = NSWindow(contentViewController: NSHostingController(
            rootView: SettingsView(model: model)
        ))
        window.title = "Settings"
        window.styleMask = [.titled, .closable]
        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("SettingsWindowController is created in code")
    }
}
