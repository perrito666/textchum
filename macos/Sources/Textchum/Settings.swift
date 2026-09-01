import AppKit
import SwiftUI
import TextchumKit
import UniformTypeIdentifiers

/// The editor-facing result of the current configuration: concrete values
/// windows can apply without knowing where they came from.
struct EditorSettings {
    let font: NSFont
    let tabWidth: Int
    let lineNumbers: Bool
    /// Whether the enclosing constructs' first lines pin at the top.
    let contextLines: Bool
    let hoverDocs: Bool
    /// Whether a file stays open when the window showing it closes.
    let keepBuffers: Bool
    let spellLanguage: String?
    /// The setting split into the dictionaries it names — several can
    /// apply at once, and a word any of them knows is spelled right.
    let spellLanguages: [String]
    /// Words to accept whatever the dictionaries say.
    let spellWords: [String]
    /// Seconds of quiet before an unattended save; zero is off.
    let autosaveSeconds: UInt32
    /// Whether selecting a word marks its other occurrences on screen,
    /// and how those are matched.
    let markOccurrences: Bool
    let occurrencesCaseSensitive: Bool
    let occurrencesWholeWord: Bool

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
        self.contextLines = config.contextLines
        self.hoverDocs = config.hoverDocs
        self.keepBuffers = config.keepBuffers
        self.markOccurrences = config.markOccurrences
        self.occurrencesCaseSensitive = config.occurrencesCaseSensitive
        self.occurrencesWholeWord = config.occurrencesWholeWord
        self.spellLanguage = config.spellLanguage
        self.spellLanguages = config.spellLanguages
        self.spellWords = config.spellWords
        self.autosaveSeconds = config.autosaveSeconds
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
    /// The file-icon pack, as a path; empty for the system's own icons.
    @Published var iconPack: String {
        didSet { persist { $0.iconPack = iconPack.isEmpty ? nil : iconPack } }
    }
    @Published var openTarget: CoreOpenTarget {
        didSet { persist { $0.openTarget = openTarget } }
    }
    @Published var newFileTarget: CoreOpenTarget {
        didSet { persist { $0.newFileTarget = newFileTarget } }
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
    @Published var contextLines: Bool {
        didSet { persist { $0.contextLines = contextLines } }
    }
    /// "head" or "branch": what the gutter compares against.
    @Published var gitMarks: String {
        didSet { persist { $0.gitMarks = gitMarks } }
    }
    /// One branch name per line; empty restores the default list.
    @Published var mergeBaseBranches: String {
        didSet {
            persist {
                $0.mergeBaseBranches = mergeBaseBranches
                    .split(whereSeparator: \.isNewline)
                    .map { $0.trimmingCharacters(in: .whitespaces) }
                    .filter { !$0.isEmpty }
            }
        }
    }
    @Published var hoverDocs: Bool {
        didSet { persist { $0.hoverDocs = hoverDocs } }
    }
    @Published var keepBuffers: Bool {
        didSet { persist { $0.keepBuffers = keepBuffers } }
    }
    @Published var interfaceLanguage: String {
        didSet { persist { $0.interfaceLanguage = interfaceLanguage } }
    }
    @Published var projectStateInProject: Bool {
        didSet { persist { $0.projectStateInProject = projectStateInProject } }
    }
    @Published var projectStateDirectory: String {
        didSet { persist { $0.projectStateDirectory = projectStateDirectory } }
    }
    @Published var projectStateSweep: Bool {
        didSet { persist { $0.projectStateSweep = projectStateSweep } }
    }
    @Published var projectStateKeepDays: Int {
        didSet { persist { $0.projectStateKeepDays = projectStateKeepDays } }
    }
    @Published var followFile: Bool {
        didSet { persist { $0.followFile = followFile } }
    }
    @Published var markOccurrences: Bool {
        didSet { persist { $0.markOccurrences = markOccurrences } }
    }
    @Published var occurrencesCaseSensitive: Bool {
        didSet { persist { $0.occurrencesCaseSensitive = occurrencesCaseSensitive } }
    }
    @Published var occurrencesWholeWord: Bool {
        didSet { persist { $0.occurrencesWholeWord = occurrencesWholeWord } }
    }
    /// Prose spell-check choice: "" = off, "auto", one language code, or
    /// several separated by commas.
    @Published var spellLanguage: String {
        didSet { persist { $0.spellLanguage = spellLanguage.isEmpty ? nil : spellLanguage } }
    }
    /// The personal word list, one per line for editing — the shape the
    /// preprocessor and glob editors already use for the same reason: a
    /// list grown a word at a time is unreadable on one line.
    @Published var spellWords: String {
        didSet {
            persist {
                $0.spellWords = spellWords
                    .split(separator: "\n")
                    .map { $0.trimmingCharacters(in: .whitespaces) }
                    .filter { !$0.isEmpty }
            }
        }
    }
    /// Seconds of quiet before an unattended save; 0 is off.
    @Published var autosaveSeconds: Int {
        didSet { persist { $0.autosaveSeconds = UInt32(max(0, autosaveSeconds)) } }
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
        /// Space-separated hidden-name globs; empty = inherit defaults.
        var hideGlobs: String = ""
        /// "" inherits; "head" or "branch" overrides the gutter's baseline.
        var gitMarks: String = ""
        /// Newline-joined branch priorities; "" inherits.
        var mergeBaseBranches: String = ""
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
    /// The default hidden-name globs, space-separated (".*" when unset).
    @Published var hideGlobsDefault = ".*" {
        didSet {
            guard !isLoading else { return }
            config.setHiddenGlobs(
                root: nil,
                globs: hideGlobsDefault == ".*" ? nil : hideGlobsDefault)
            persistLSPChange()
        }
    }
    @Published private(set) var workspaceEntries: [WorkspaceEntry] = []

    func setHideGlobs(scope: String, globs: String) {
        config.setHiddenGlobs(root: scope, globs: globs.isEmpty ? nil : globs)
        persistWorkspaceChange()
    }

    // MARK: Hide presets

    /// The named glob sets the hide editors offer, sorted by name.
    @Published private(set) var hidePresets: [(name: String, globs: [String])] = []

    private func reloadHidePresets() {
        hidePresets = config.hidePresets
    }

    func setHidePreset(name: String, globs: String?) {
        let name = name.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        config.setHidePreset(name: name, globs: globs)
        persistPresetChange()
    }

    func resetHidePresets() {
        config.resetHidePresets()
        persistPresetChange()
    }

    private func persistPresetChange() {
        do {
            try config.save()
        } catch {
            NSLog("could not save configuration: \(error)")
        }
        reloadHidePresets()
        onChange?()
    }

    /// Built-ins plus the user's theme files; a user file sharing a
    /// built-in's name shows once (the file wins when applied).
    /// The languages the build knows, for the fields that take one.
    /// The set is open: a language absent from this list can still be
    /// typed, since configuration may name one the build has no grammar
    /// for yet.
    var knownLanguages: [String] {
        CoreLanguages.all.map(\.name).sorted()
    }

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
        iconPack = config.iconPack ?? ""
        openTarget = config.openTarget
        newFileTarget = config.newFileTarget
        fontFamily = config.fontFamily ?? ""
        fontSize = config.fontSize
        tabWidth = config.tabWidth
        lineNumbers = config.lineNumbers
        contextLines = config.contextLines
        gitMarks = config.gitMarks
        mergeBaseBranches = config.mergeBaseBranches.joined(separator: "\n")
        hoverDocs = config.hoverDocs
        keepBuffers = config.keepBuffers
        interfaceLanguage = config.interfaceLanguage
        projectStateInProject = config.projectStateInProject
        projectStateDirectory = config.projectStateDirectory
        projectStateSweep = config.projectStateSweep
        projectStateKeepDays = config.projectStateKeepDays
        keysProfile = config.keysProfile
        markOccurrences = config.markOccurrences
        occurrencesCaseSensitive = config.occurrencesCaseSensitive
        occurrencesWholeWord = config.occurrencesWholeWord
        followFile = config.followFile
        spellLanguage = config.spellLanguage ?? ""
        spellWords = config.spellWords.joined(separator: "\n")
        autosaveSeconds = Int(config.autosaveSeconds)
        isLoading = false
        reloadLSPEntries()
        reloadWorkspaceEntries()
        reloadPreprocessorEntries()
        reloadHidePresets()
    }

    init(config: CoreConfig) {
        self.config = config
        self.appearance = config.appearance
        self.theme = config.theme
        self.iconPack = config.iconPack ?? ""
        self.openTarget = config.openTarget
        self.newFileTarget = config.newFileTarget
        self.fontFamily = config.fontFamily ?? ""
        self.fontSize = config.fontSize
        self.tabWidth = config.tabWidth
        self.lineNumbers = config.lineNumbers
        self.contextLines = config.contextLines
        self.gitMarks = config.gitMarks
        self.mergeBaseBranches = config.mergeBaseBranches.joined(separator: "\n")
        self.hoverDocs = config.hoverDocs
        self.keepBuffers = config.keepBuffers
        self.interfaceLanguage = config.interfaceLanguage
        self.projectStateInProject = config.projectStateInProject
        self.projectStateDirectory = config.projectStateDirectory
        self.projectStateSweep = config.projectStateSweep
        self.projectStateKeepDays = config.projectStateKeepDays
        self.followFile = config.followFile
        self.spellLanguage = config.spellLanguage ?? ""
        self.spellWords = config.spellWords.joined(separator: "\n")
        self.autosaveSeconds = Int(config.autosaveSeconds)
        self.keysProfile = config.keysProfile
        self.markOccurrences = config.markOccurrences
        self.occurrencesCaseSensitive = config.occurrencesCaseSensitive
        self.occurrencesWholeWord = config.occurrencesWholeWord
        self.isLoading = false
        reloadLSPEntries()
        reloadWorkspaceEntries()
        reloadPreprocessorEntries()
        reloadHidePresets()
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
            hideGlobsDefault =
                (parsed["hide"] as? [String])?.joined(separator: " ") ?? ".*"
            recursiveConfigDefault = parsed["recursive_config"] as? Bool ?? false
            ctagsFallbackDefault = parsed["ctags_fallback"] as? Bool ?? false
            for (root, raw) in parsed["projects"] as? [String: [String: Any]] ?? [:] {
                let editor = raw["editor"] as? [String: Any] ?? [:]
                let hide = (raw["hide"] as? [String])?.joined(separator: " ") ?? ""
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
                        tabWidth: (editor["tab_width"] as? Int).map(String.init) ?? "",
                        hideGlobs: hide,
                        gitMarks: editor["git_marks"] as? String ?? "",
                        mergeBaseBranches: (editor["merge_base_branches"] as? [String])?
                            .joined(separator: "\n") ?? ""
                    ))
            }
        }
        workspaceEntries = entries.sorted { $0.scope < $1.scope }
    }

    /// Where imported icon packs live:
    /// `~/Library/Application Support/Textchum/icons/`.
    static var iconPacksDirectory: String { AppPaths.iconsDirectory.path }

    /// The packs on offer: the imported ones first, then the ones
    /// opened from elsewhere.
    var iconPacks: [CoreConfig.IconPackEntry] {
        config.iconPacks(in: Self.iconPacksDirectory)
    }

    /// Points at a pack chosen from outside Textchum's folder, and
    /// remembers it so it stays on the list.
    func useIconPack(path: String) {
        config.rememberIconPack(path: path)
        iconPack = path
    }

    /// Copies a pack into Textchum's folder and switches to the copy.
    /// Returns the reason it could not be copied.
    func importIconPack(from source: String) -> String? {
        let outcome = config.importIconPack(from: source, into: Self.iconPacksDirectory)
        guard let path = outcome.path else { return outcome.error }
        iconPack = path
        return nil
    }

    /// Deletes an imported pack.
    func removeIconPack(path: String) {
        config.removeIconPack(path: path, from: Self.iconPacksDirectory)
        // Removing the one in use leaves the system's icons; the core
        // has already cleared it, and this republishes that.
        reloadFromConfig()
        persistNow()
    }

    /// One overridable command, with the menu title it wears and the
    /// shortcut it has now.
    struct Shortcut: Identifiable, Equatable {
        let action: String
        let title: String
        let spec: String

        var id: String { action }
    }

    /// Every overridable command. The app fills this in; Settings has
    /// no idea what the menus hold on its own.
    @Published var shortcutCatalog: [Shortcut] = []

    /// The chosen keyboard profile; empty is the editor's own bindings.
    @Published var keysProfile: String {
        didSet { persist { $0.keysProfile = keysProfile } }
    }

    /// The profiles that can be chosen: the bundled ones and any saved
    /// here.
    var keyProfileChoices: [(id: String, name: String)] {
        config.keyProfileChoices
    }

    /// The shortcut overrides in force, by action.
    var keyOverrides: [String: String] {
        guard let data = config.keysJSON.data(using: .utf8),
            let parsed = try? JSONSerialization.jsonObject(with: data) as? [String: String]
        else { return [:] }
        return parsed
    }

    /// Rebinds one command, or with an empty spec gives it back its
    /// profile's shortcut — or the editor's own.
    func setKeyBinding(action: String, spec: String) {
        config.setKeyBinding(action: action, spec: spec.isEmpty ? nil : spec)
        persistNow()
    }

    /// Forgets every override, keeping the profile.
    func clearKeyBindings() {
        config.clearKeyBindings()
        persistNow()
    }

    /// Saves what is in force now as a profile of its own, and switches
    /// to it — which is what "modify a preset" means when the presets
    /// themselves ship with the build.
    func saveKeyProfile(named name: String) {
        let name = name.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        let bindings = config.effectiveKeys
        guard let data = try? JSONSerialization.data(withJSONObject: bindings),
            let json = String(data: data, encoding: .utf8)
        else { return }
        config.setKeyProfile(name: name, bindingsJSON: json)
        // The overrides are in the profile now; leaving them behind
        // would apply them twice and hide what the profile says.
        config.clearKeyBindings()
        keysProfile = name
    }

    /// Forgets a saved profile. A bundled one cannot be removed, only
    /// shadowed by a saved profile of the same name.
    func removeKeyProfile(named name: String) {
        config.setKeyProfile(name: name, bindingsJSON: nil)
        if keysProfile == name {
            keysProfile = ""
        } else {
            persistNow()
        }
    }

    /// The project roots of the documents that are open, for adding one
    /// by picking it. The app fills this in; Settings has no idea what
    /// is open on its own.
    @Published var openProjectRoots: [String] = []

    /// Open projects that have no entry yet — the ones worth offering.
    var addableProjectRoots: [String] {
        let configured = Set(workspaceEntries.map(\.scope))
        return openProjectRoots.filter { !configured.contains($0) }.sorted()
    }

    /// Configured projects whose directory is gone. Nothing will ever
    /// match these again, so they are worth saying so about.
    var staleProjectRoots: [String] {
        config.configuredProjects.filter {
            !FileManager.default.fileExists(atPath: $0)
        }
    }

    /// Adds an entry for `scope`, optionally taking another project's
    /// settings — a second service in the same layout wants the same
    /// ones, and entering them again is a transcription exercise with a
    /// typo in it.
    func addWorkspaceEntry(scope: String, copyingFrom source: String? = nil) {
        let scope = (scope as NSString).expandingTildeInPath
        guard !scope.isEmpty else { return }
        if let source, !source.isEmpty {
            config.copyProject(from: source, to: scope)
        }
        config.setWorkspaceFlag(
            root: scope, key: "manifest_projects", value: manifestProjectsDefault)
        config.setWorkspaceFlag(
            root: scope, key: "recursive_config", value: recursiveConfigDefault)
        config.setWorkspaceFlag(
            root: scope, key: "ctags_fallback", value: ctagsFallbackDefault)
        persistWorkspaceChange()
    }

    /// Copies one project's settings onto another that already exists.
    func copyProjectSettings(from source: String, to target: String) {
        guard config.copyProject(from: source, to: target) else { return }
        persistWorkspaceChange()
    }

    /// Forgets every configured project whose directory is gone.
    func removeStaleProjects() {
        let stale = staleProjectRoots
        guard !stale.isEmpty else { return }
        for root in stale {
            config.removeProject(root: root)
        }
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

    /// Whether a configured project's directory is still there.
    func projectExists(_ scope: String) -> Bool {
        FileManager.default.fileExists(atPath: scope)
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
        persistNow()
    }

    /// Saves and republishes a change already made to the
    /// configuration.
    private func persistNow() {
        do {
            try config.save()
        } catch {
            NSLog("could not save configuration: \(error)")
        }
        objectWillChange.send()
        onChange?()
    }
}

/// The settings window: General plus Language Servers.
struct SettingsView: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        TabView(selection: $model.selectedTab) {
            GeneralSettingsTab(model: model)
                .tabItem { Label(t("General"), systemImage: "gearshape") }
                .tag("general")
            KeyboardTab(model: model)
                .tabItem { Label(t("Keyboard"), systemImage: "keyboard") }
                .tag("keyboard")
            ProjectsTab(model: model)
                .tabItem { Label(t("Projects"), systemImage: "folder.badge.gearshape") }
                .tag("projects")
            LanguageServersTab(model: model)
                .tabItem { Label(t("Language Servers"), systemImage: "network") }
                .tag("servers")
            PreprocessorsTab(model: model)
                .tabItem { Label(t("Preprocessors"), systemImage: "wand.and.rays") }
                .tag("preprocessors")
            PresetsTab(model: model)
                .tabItem { Label(t("Presets"), systemImage: "eye.slash") }
                .tag("presets")
        }
        .frame(
            minWidth: 620, idealWidth: 700,
            minHeight: 380, idealHeight: 560
        )
        .padding(20)
    }
}

/// The tab every new setting lands on, and so the one that outgrows a
/// fixed window. Its form scrolls: overflow becomes something to scroll
/// to rather than something that is not drawn.
struct GeneralSettingsTab: View {
    @ObservedObject var model: SettingsModel
    /// Why the last import did not work, said once and in place.
    @State private var iconPackError: String?

    /// The system's dictionaries, plus whatever the file already names
    /// — a hand-written `en_US` on a machine that spells it `en` would
    /// otherwise select nothing and read as "off".
    private var spellLanguages: [String] {
        var languages = NSSpellChecker.shared.availableLanguages
        for configured in chosenSpellLanguages
        where configured != "auto" && !languages.contains(configured) {
            languages.insert(configured, at: 0)
        }
        return languages
    }

    /// The setting read as the list it is allowed to be.
    private var chosenSpellLanguages: [String] {
        model.spellLanguage
            .split(whereSeparator: { $0 == "," || $0 == " " })
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
    }

    /// What the menu says when closed.
    private var spellSummary: String {
        let chosen = chosenSpellLanguages
        if chosen.isEmpty { return "Off" }
        if chosen == ["auto"] { return "Automatic by content" }
        return chosen
            .map { Locale.current.localizedString(forIdentifier: $0) ?? $0 }
            .joined(separator: ", ")
    }

    private func toggleSpellLanguage(_ language: String, on: Bool) {
        // "auto" and a named dictionary contradict each other; naming one
        // is the more specific instruction, so it wins.
        var chosen = chosenSpellLanguages.filter { $0 != "auto" }
        if on {
            if !chosen.contains(language) { chosen.append(language) }
        } else {
            chosen.removeAll { $0 == language }
        }
        model.spellLanguage = chosen.joined(separator: ", ")
    }

    /// Font families with a fixed-pitch face, plus the platform default.
    private var monospacedFamilies: [String] {
        NSFontManager.shared.availableFontFamilies.filter { family in
            guard let font = NSFont(name: family, size: 12) else { return false }
            return font.isFixedPitch
        }
    }

    var body: some View {
        ScrollView {
            form
        }
    }

    /// Asks for an icon pack: the theme's JSON, or the extension folder
    /// holding it. Both are offered because packs are distributed both
    /// ways, and requiring someone to know which is inside is asking
    /// them to read a manifest.
    /// The picker's selection, which is the pack in use.
    private var iconPackSelection: Binding<String> {
        Binding(
            get: { model.iconPack },
            set: { model.iconPack = $0 }
        )
    }

    /// Chooses a pack. Importing copies it into Textchum's folder, so
    /// it survives the original being moved or deleted; opening points
    /// at it where it is.
    private func chooseIconPack(copying: Bool) {
        let panel = NSOpenPanel()
        panel.message = "Choose an icon theme file, or the extension folder holding one."
        panel.prompt = copying ? "Import" : "Use"
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [UTType.json].compactMap { $0 }
        panel.treatsFilePackagesAsDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else { return }
        iconPackError = copying ? model.importIconPack(from: url.path) : nil
        if !copying { model.useIconPack(path: url.path) }
    }

    private var form: some View {
        Form {
            Picker(t("Appearance:"), selection: $model.appearance) {
                Text(t("System")).tag(CoreAppearance.system)
                Text(t("Light")).tag(CoreAppearance.light)
                Text(t("Dark")).tag(CoreAppearance.dark)
            }
            .pickerStyle(.segmented)
            Picker(t("Theme:"), selection: $model.theme) {
                ForEach(model.availableThemes, id: \.self) { name in
                    Text(name).tag(name)
                }
            }
            // A pack is a folder of images somewhere on disk, not a
            // name from a list, so it gets a chooser rather than a
            // picker — and a way back to the system's icons.
            // The packs already seen are a list; a new one is a folder
            // somewhere on disk, so both a picker and a chooser.
            LabeledContent(t("File icons:")) {
                HStack(spacing: 8) {
                    Picker("", selection: iconPackSelection) {
                        Text(t("System icons")).tag("")
                        let imported = model.iconPacks.filter(\.imported)
                        let elsewhere = model.iconPacks.filter { !$0.imported }
                        if !imported.isEmpty {
                            Section(t("Imported")) {
                                ForEach(imported, id: \.path) { pack in
                                    Text(pack.name).tag(pack.path)
                                }
                            }
                        }
                        if !elsewhere.isEmpty {
                            Section(t("Elsewhere")) {
                                ForEach(elsewhere, id: \.path) { pack in
                                    Text(pack.name).tag(pack.path)
                                }
                            }
                        }
                    }
                    .labelsHidden()
                    .frame(width: 220)
                    Button(t("Import…")) { chooseIconPack(copying: true) }
                    Button(t("Open…")) { chooseIconPack(copying: false) }
                    if !model.iconPack.isEmpty,
                        model.iconPacks.first(where: { $0.path == model.iconPack })?.imported
                            == true
                    {
                        Button(t("Delete")) { model.removeIconPack(path: model.iconPack) }
                    }
                }
            }
            if let iconPackError {
                Text(iconPackError)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Picker(t("Open files in:"), selection: $model.openTarget) {
                Text(t("Tabs")).tag(CoreOpenTarget.tab)
                Text(t("Windows")).tag(CoreOpenTarget.window)
            }
            Picker(t("New files in:"), selection: $model.newFileTarget) {
                Text(t("Tabs")).tag(CoreOpenTarget.tab)
                Text(t("Windows")).tag(CoreOpenTarget.window)
            }
            .pickerStyle(.segmented)
            Picker(t("Font:"), selection: $model.fontFamily) {
                Text(t("System Monospaced")).tag("")
                Divider()
                ForEach(monospacedFamilies, id: \.self) { family in
                    Text(family).tag(family)
                }
            }
            Stepper(value: $model.fontSize, in: 6...72, step: 1) {
                Text(t("Font size: {} pt", Int(model.fontSize)))
            }
            Stepper(value: $model.tabWidth, in: 1...16) {
                Text(t("Tab width: {} columns", model.tabWidth))
            }
            Toggle(t("Show line numbers"), isOn: $model.lineNumbers)
            Toggle(t("Pin enclosing context lines"), isOn: $model.contextLines)
            Picker(t("Gutter marks compare against"), selection: $model.gitMarks) {
                Text(t("The last commit")).tag("head")
                Text(t("Where the branch forked")).tag("branch")
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(t("Merge-base branches, most likely first"))
                Text(t("Tried in order when git does not name a default branch."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextEditor(text: $model.mergeBaseBranches)
                    .font(.body.monospaced())
                    .frame(height: 64)
                    .overlay(
                        RoundedRectangle(cornerRadius: 4)
                            .stroke(Color.secondary.opacity(0.3)))
            }
            Toggle(t("Hover documentation"), isOn: $model.hoverDocs)
            Toggle(t("Keep files open when their window closes"), isOn: $model.keepBuffers)
            Picker(t("Interface language"), selection: $model.interfaceLanguage) {
                Text(t("System")).tag("system")
                ForEach(CoreI18n.languages, id: \.tag) { language in
                    Text(language.name).tag(language.tag)
                }
            }
            Text(t("Restart to apply"))
                .font(.caption)
                .foregroundStyle(.secondary)
            ProjectRecordsSettings(model: model)
            Toggle(t("Reveal the current file in the tree"), isOn: $model.followFile)
            Toggle(t("Mark the selected word elsewhere on screen"), isOn: $model.markOccurrences)
            Toggle(t("  Match case"), isOn: $model.occurrencesCaseSensitive)
                .disabled(!model.markOccurrences)
            Toggle(t("  Whole words only"), isOn: $model.occurrencesWholeWord)
                .disabled(!model.markOccurrences)
            // Not a Picker: several dictionaries can apply at once, and
            // a picker can only say one thing. Each language is a toggle
            // that adds itself to the list.
            LabeledContent(t("Spell check prose")) {
                Menu(spellSummary) {
                    Button(t("Off")) { model.spellLanguage = "" }
                    Button(t("Automatic by content")) { model.spellLanguage = "auto" }
                    Divider()
                    ForEach(spellLanguages, id: \.self) { language in
                        Toggle(
                            Locale.current.localizedString(forIdentifier: language)
                                ?? language,
                            isOn: Binding(
                                get: { chosenSpellLanguages.contains(language) },
                                set: { on in toggleSpellLanguage(language, on: on) }
                            ))
                    }
                }
            }
            Text("Checks comments in code, and everything in Markdown, git commit messages, and plain text. Several dictionaries can apply at once; a word any of them knows is spelled correctly.")
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            LabeledContent(t("Dictionary")) {
                VStack(alignment: .leading, spacing: 4) {
                    CommandsEditor(placeholder: "SBX\nTextchum", text: $model.spellWords)
                    Text(t("Words to accept whatever the dictionaries say, one per line."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Stepper(value: $model.autosaveSeconds, in: 0...600, step: 5) {
                Text(
                    model.autosaveSeconds == 0
                        ? "Autosave: off"
                        : "Autosave after \(model.autosaveSeconds) s of quiet")
            }
            Text("Files that have a name only, and without running save preprocessors — a formatter reflowing the line you are typing is not a favour.")
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 28)
        .padding(.vertical, 20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
}

/// The presets themselves, edited the same way the hide lists are:
/// one pattern per line. Editing any preset takes ownership of the
/// whole set, so a deleted one stays deleted until Restore Built-ins.
private struct PresetsTab: View {
    @ObservedObject var model: SettingsModel
    @State private var newName = ""

    var body: some View {
        // One scrolling column per tab: a settings screen that
        // cannot scroll turns one more setting into a setting
        // nobody can reach.
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Named glob sets the hide editors can add in one click. They are yours to change: edit any preset and this list replaces the built-in one, so removals stick.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

                List {
                    if model.hidePresets.isEmpty {
                        Text(t("No presets — add one below, or restore the built-ins."))
                            .foregroundStyle(.secondary)
                    }
                    ForEach(model.hidePresets, id: \.name) { preset in
                        HStack(alignment: .top, spacing: 8) {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(preset.name)
                                    .fontWeight(.semibold)
                                PresetGlobsField(
                                    name: preset.name,
                                    initial: preset.globs.joined(separator: "\n")
                                ) { globs in
                                    model.setHidePreset(name: preset.name, globs: globs)
                                }
                            }
                            Spacer()
                            Button {
                                model.setHidePreset(name: preset.name, globs: nil)
                            } label: {
                                Image(systemName: "minus.circle")
                            }
                            .buttonStyle(.borderless)
                        }
                    }
                }
                .frame(height: 240)

                HStack(spacing: 8) {
                    TextField("New preset name", text: $newName)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 200)
                    Button(t("Add")) {
                        // A fresh preset starts with a placeholder pattern:
                        // an empty one would remove itself immediately.
                        model.setHidePreset(name: newName, globs: newName.lowercased())
                        newName = ""
                    }
                    .disabled(newName.trimmingCharacters(in: .whitespaces).isEmpty)
                    Spacer()
                    Button(t("Restore Built-ins")) {
                        model.resetHidePresets()
                    }
                }
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 16)
        }
    }
}

/// One preset's patterns, committed when focus leaves — the same
/// multi-line editor the hide lists use.
private struct PresetGlobsField: View {
    let name: String
    let initial: String
    let commit: (String) -> Void
    @State private var text: String

    init(name: String, initial: String, commit: @escaping (String) -> Void) {
        self.name = name
        self.initial = initial
        self.commit = commit
        _text = State(initialValue: initial)
    }

    var body: some View {
        CommandsEditor(placeholder: t("one pattern per line"), text: $text) {
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard trimmed != initial else { return }
            commit(trimmed.split(whereSeparator: \.isNewline).joined(separator: " "))
        }
        .frame(maxWidth: 360)
    }
}

/// A multi-line glob list with a presets menu: one pattern per line,
/// because a space-separated one-liner is unreadable past two entries.
/// Presets append their patterns, skipping ones already present.
struct GlobEditor: View {
    let presets: [(name: String, globs: [String])]
    @Binding var text: String
    var onCommit: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            CommandsEditor(
                placeholder: t("one pattern per line — *.pyc, target, .git"),
                text: $text, onFocusLost: onCommit)
            HStack {
                Menu("Add preset") {
                    if presets.isEmpty {
                        Text(t("No presets — add some in the Presets tab"))
                    }
                    ForEach(presets, id: \.name) { preset in
                        Button(preset.name) { append(preset.globs) }
                    }
                }
                .fixedSize()
                Spacer()
                Text(t("Glob patterns: * and ? match; names only, not paths."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func append(_ globs: [String]) {
        var lines = text.split(whereSeparator: \.isNewline).map(String.init)
        for glob in globs where !lines.contains(glob) {
            lines.append(glob)
        }
        text = lines.joined(separator: "\n")
        onCommit()
    }
}

/// The button + popover pair the hide rows use: the list stays out of
/// the way until asked for, and edits commit when the popover closes.
/// The per-project gutter-baseline choice, spelled out: inherit, the
/// last commit, or where the branch forked — a menu, not a word to
/// remember and type.
struct GitMarksOverridePicker: View {
    let initial: String
    let commit: (String) -> Void
    @State private var choice = ""
    @State private var loaded = false

    var body: some View {
        Picker(t("Gutter marks"), selection: $choice) {
            Text(t("Inherit")).tag("")
            Text(t("The last commit")).tag("head")
            Text(t("Where the branch forked")).tag("branch")
        }
        .frame(width: 220)
        .onAppear {
            guard !loaded else { return }
            loaded = true
            choice = initial
        }
        .onChange(of: choice) { _, chosen in
            guard loaded else { return }
            commit(chosen)
        }
    }
}

/// A button opening the merge-base list for one project: one branch
/// name per line, empty inherits the global list.
struct BranchListButton: View {
    let title: String
    /// Newline-joined on the way in; the editor works in lines too.
    let initial: String
    let commit: ([String]) -> Void

    @State private var showing = false
    @State private var text = ""

    var body: some View {
        Button(
            initial.isEmpty
                ? t("Merge-base branches: inherited")
                : t("Merge-base branches: {}", initial.replacingOccurrences(of: "\n", with: ", "))
        ) {
            text = initial
            showing = true
        }
        .popover(isPresented: $showing, arrowEdge: .bottom) {
            VStack(alignment: .leading, spacing: 10) {
                Text(title)
                    .font(.headline)
                Text(t("One branch name per line, most likely first; empty inherits."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextEditor(text: $text)
                    .font(.body.monospaced())
                    .frame(width: 260, height: 96)
                HStack {
                    Spacer()
                    Button(t("Done")) {
                        commit(
                            text.split(whereSeparator: \.isNewline)
                                .map { $0.trimmingCharacters(in: .whitespaces) }
                                .filter { !$0.isEmpty })
                        showing = false
                    }
                    .keyboardShortcut(.defaultAction)
                }
            }
            .padding(14)
        }
    }
}

struct GlobEditorButton: View {
    let title: String
    let presets: [(name: String, globs: [String])]
    /// Space-separated on the way in and out; the editor works in lines.
    let initial: String
    let commit: (String) -> Void
    /// Debug hook: opens the popover as soon as the row appears, so the
    /// editor is screenshot-verifiable without synthesizing clicks.
    var autoOpen: Bool = false

    @State private var showing = false
    @State private var text = ""

    var body: some View {
        Button(label) { 
            text = initial.split(separator: " ").joined(separator: "\n")
            showing = true
        }
        .popover(isPresented: $showing, arrowEdge: .bottom) {
            VStack(alignment: .leading, spacing: 10) {
                Text(title)
                    .font(.headline)
                GlobEditor(presets: presets, text: $text)
                    .frame(width: 320)
                HStack {
                    Spacer()
                    Button(t("Done")) {
                        commit(
                            text.split(whereSeparator: \.isNewline)
                                .map(String.init).joined(separator: " "))
                        showing = false
                    }
                    .keyboardShortcut(.defaultAction)
                }
            }
            .padding(14)
        }
        .onAppear {
            guard autoOpen else { return }
            text = initial.split(separator: " ").joined(separator: "\n")
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { showing = true }
        }
    }

    private var label: String {
        let count = initial.split(separator: " ").count
        if count == 0 { return "Hide: inherit" }
        return "Hide: \(count) pattern\(count == 1 ? "" : "s")"
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

/// The keyboard shortcuts, and the profiles that set them.
///
/// People arrive from another editor with its shortcuts in their
/// fingers, so the three those editors are known for are offered
/// whole. A profile names the commands it moves and nothing else, and
/// a single shortcut can still be changed on top of one — that change
/// is an override, and **Save as profile** turns the result into a
/// profile of its own.
private struct KeyboardTab: View {
    @ObservedObject var model: SettingsModel
    @State private var newProfileName = ""
    @State private var filter = ""

    private var shown: [SettingsModel.Shortcut] {
        guard !filter.isEmpty else { return model.shortcutCatalog }
        return model.shortcutCatalog.filter {
            $0.title.localizedCaseInsensitiveContains(filter)
                || $0.action.localizedCaseInsensitiveContains(filter)
                || $0.spec.localizedCaseInsensitiveContains(filter)
        }
    }

    var body: some View {
        // One scrolling column per tab: a settings screen that
        // cannot scroll turns one more setting into a setting
        // nobody can reach.
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text(
                    "A profile sets the shortcuts its editor is known for and leaves the "
                        + "rest alone. Changing one on top of a profile keeps the profile; "
                        + "\"Save as profile\" turns what is in force into one of your own. "
                        + "Shortcuts are written as \"cmd+shift+f\" — cmd is Command here "
                        + "and Ctrl on Linux."
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

                HStack(spacing: 8) {
                    Picker(t("Profile"), selection: $model.keysProfile) {
                        Text("Textchum").tag("")
                        ForEach(model.keyProfileChoices, id: \.id) { choice in
                            Text(choice.name).tag(choice.id)
                        }
                    }
                    .frame(width: 320)
                    Button(t("Reset changes")) { model.clearKeyBindings() }
                        .disabled(model.keyOverrides.isEmpty)
                    Spacer()
                }

                HStack(spacing: 8) {
                    TextField("Save as profile…", text: $newProfileName)
                        .frame(width: 200)
                    Button(t("Save as profile")) {
                        model.saveKeyProfile(named: newProfileName)
                        newProfileName = ""
                    }
                    .disabled(newProfileName.trimmingCharacters(in: .whitespaces).isEmpty)
                    if !model.keysProfile.isEmpty {
                        Button(t("Delete profile")) {
                            model.removeKeyProfile(named: model.keysProfile)
                        }
                    }
                    Spacer()
                }

                TextField("Filter commands…", text: $filter)

                List {
                    if model.shortcutCatalog.isEmpty {
                        Text(t("Open the Settings window from the app to see the commands."))
                            .foregroundStyle(.secondary)
                    }
                    ForEach(shown) { shortcut in
                        HStack(spacing: 12) {
                            Text(shortcut.title)
                                .frame(width: 220, alignment: .leading)
                                .help(shortcut.action)
                            ShortcutField(
                                shortcut: shortcut,
                                overridden: model.keyOverrides[shortcut.action] != nil
                            ) { spec in
                                model.setKeyBinding(action: shortcut.action, spec: spec)
                            }
                            Spacer()
                        }
                    }
                }
                .frame(height: 260)
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 16)
        }
    }
}

/// One command's shortcut, editable in place. Commits on ⏎ or when
/// focus leaves; emptying it gives the command back the shortcut its
/// profile — or the editor — says it has.
private struct ShortcutField: View {
    let shortcut: SettingsModel.Shortcut
    let overridden: Bool
    let commit: (String) -> Void
    @State private var text: String
    @FocusState private var focused: Bool

    init(
        shortcut: SettingsModel.Shortcut, overridden: Bool,
        commit: @escaping (String) -> Void
    ) {
        self.shortcut = shortcut
        self.overridden = overridden
        self.commit = commit
        _text = State(initialValue: shortcut.spec)
    }

    var body: some View {
        HStack(spacing: 6) {
            TextField("unbound", text: $text)
                .font(.system(.body, design: .monospaced))
                .frame(width: 180)
                .focused($focused)
                .onSubmit { commit(text) }
                .onChange(of: focused) { _, isFocused in
                    if !isFocused, text != shortcut.spec { commit(text) }
                }
                .onChange(of: shortcut.spec) { _, spec in
                    if !focused { text = spec }
                }
            if overridden {
                // Which rows you changed is the question this screen
                // gets asked next.
                Text(t("changed"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

private struct ProjectsTab: View {
    @ObservedObject var model: SettingsModel
    @State private var newScope = ""
    /// Which configured project the new one should start from.
    @State private var copyFrom = ""

    var body: some View {
        // One scrolling column per tab: a settings screen that
        // cannot scroll turns one more setting into a setting
        // nobody can reach.
        ScrollView {
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
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 24) {
                            Toggle(t("Manifest projects"), isOn: $model.manifestProjectsDefault)
                            Toggle(t("Recursive config"), isOn: $model.recursiveConfigDefault)
                            Toggle(t("Ctags fallback"), isOn: $model.ctagsFallbackDefault)
                            Spacer()
                        }
                        HStack(spacing: 8) {
                            Text(t("Hide in tree:"))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            GlobEditorButton(
                                title: t("Hidden in every project"),
                                presets: model.hidePresets,
                                initial: model.hideGlobsDefault,
                                commit: { globs in
                                    model.hideGlobsDefault = globs.isEmpty ? ".*" : globs
                                },
                                autoOpen: ProcessInfo.processInfo
                                    .environment["TEXTCHUM_DEBUG_GLOBS"] != nil
                            )
                            Text(model.hideGlobsDefault)
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.tail)
                            Spacer()
                        }
                    }
                    .padding(6)
                }

                List {
                    if model.workspaceEntries.isEmpty {
                        Text(t("No per-project overrides."))
                            .foregroundStyle(.secondary)
                    }
                    ForEach(model.workspaceEntries) { entry in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack(spacing: 12) {
                                Text(entry.scopeLabel)
                                    .fontWeight(.semibold)
                                    .help(entry.scope)
                                if !model.projectExists(entry.scope) {
                                    // Nothing will ever match this root
                                    // again, which is worth saying rather
                                    // than leaving the reader to wonder.
                                    Text(t("missing"))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                        .help("This directory no longer exists")
                                }
                                Menu {
                                    ForEach(
                                        model.workspaceEntries.filter { $0.scope != entry.scope }
                                    ) { source in
                                        Button(source.scopeLabel) {
                                            model.copyProjectSettings(
                                                from: source.scope, to: entry.scope)
                                        }
                                        .help(source.scope)
                                    }
                                } label: {
                                    Text(t("Copy from…"))
                                }
                                .frame(width: 120)
                                .disabled(model.workspaceEntries.count < 2)
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
                            // Editor overrides for windows inside this root.
                            // An empty field inherits, and says what it
                            // inherits: a blank box otherwise leaves the
                            // reader to go and look.
                            HStack(spacing: 8) {
                                Text(t("Editor:"))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                OverrideField(
                                    placeholder: model.fontFamily.isEmpty
                                        ? "font family" : model.fontFamily,
                                    width: 140,
                                    initial: entry.fontFamily
                                ) { text in
                                    model.setEditorOverride(
                                        scope: entry.scope, key: "font_family",
                                        valueJSON: text.isEmpty
                                            ? nil
                                            : "\"\(text.replacingOccurrences(of: "\"", with: ""))\"")
                                }
                                OverrideField(
                                    placeholder: String(Int(model.fontSize)), width: 52,
                                    initial: entry.fontSize
                                ) { text in
                                    model.setEditorOverride(
                                        scope: entry.scope, key: "font_size",
                                        valueJSON: Double(text).map { String($0) })
                                }
                                OverrideField(
                                    placeholder: String(model.tabWidth), width: 72,
                                    initial: entry.tabWidth
                                ) { text in
                                    model.setEditorOverride(
                                        scope: entry.scope, key: "tab_width",
                                        valueJSON: Int(text).map(String.init))
                                }
                                GitMarksOverridePicker(initial: entry.gitMarks) { choice in
                                    model.setEditorOverride(
                                        scope: entry.scope, key: "git_marks",
                                        valueJSON: choice.isEmpty ? nil : "\"\(choice)\"")
                                }
                                BranchListButton(
                                    title: t("Merge-base branches in {}", entry.scopeLabel),
                                    initial: entry.mergeBaseBranches
                                ) { names in
                                    let json =
                                        (try? JSONSerialization.data(withJSONObject: names))
                                        .flatMap { String(data: $0, encoding: .utf8) }
                                    model.setEditorOverride(
                                        scope: entry.scope, key: "merge_base_branches",
                                        valueJSON: names.isEmpty ? nil : json)
                                }
                                GlobEditorButton(
                                    title: "Hidden in \(entry.scopeLabel)",
                                    presets: model.hidePresets,
                                    initial: entry.hideGlobs
                                ) { globs in
                                    model.setHideGlobs(scope: entry.scope, globs: globs)
                                }
                            }
                        }
                    }
                }
                .frame(height: 220)

                GroupBox("Add project override") {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 8) {
                            PathPicker(text: $newScope, placeholder: t("Project root path"))
                            // The roots are known — every open document has
                            // one — so a project is added by picking it.
                            Menu("Open projects") {
                                if model.addableProjectRoots.isEmpty {
                                    Text(t("Every open project is already listed"))
                                }
                                ForEach(model.addableProjectRoots, id: \.self) { root in
                                    Button((root as NSString).lastPathComponent) {
                                        newScope = root
                                    }
                                    .help(root)
                                }
                            }
                            .frame(width: 150)
                            Button(t("Add")) {
                                model.addWorkspaceEntry(
                                    scope: newScope,
                                    copyingFrom: copyFrom.isEmpty ? nil : copyFrom)
                                newScope = ""
                                copyFrom = ""
                            }
                            .disabled(newScope.isEmpty)
                        }
                        HStack(spacing: 8) {
                            Text(t("Copy settings from:"))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Menu(copyFrom.isEmpty ? "Nothing" : (copyFrom as NSString).lastPathComponent) {
                                Button(t("Nothing")) { copyFrom = "" }
                                ForEach(model.workspaceEntries) { entry in
                                    Button(entry.scopeLabel) { copyFrom = entry.scope }
                                        .help(entry.scope)
                                }
                            }
                            .frame(width: 200)
                            Text(t("servers, save commands, flags and editor overrides"))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Spacer()
                        }
                    }
                    .padding(6)
                }

                if !model.staleProjectRoots.isEmpty {
                    HStack(spacing: 8) {
                        Text(
                            model.staleProjectRoots.count == 1
                                ? "1 configured project no longer exists on disk."
                                : "\(model.staleProjectRoots.count) configured projects "
                                    + "no longer exist on disk."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        Button(t("Remove missing")) { model.removeStaleProjects() }
                            .controlSize(.small)
                        Spacer()
                    }
                }
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 16)
        }
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
        // One scrolling column per tab: a settings screen that
        // cannot scroll turns one more setting into a setting
        // nobody can reach.
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Override which server command runs per language — for every project (Default) or for one project root. Project entries win over defaults; unlisted languages use the built-in registry.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

                List {
                    if model.lspEntries.isEmpty {
                        Text(t("No overrides — the built-in registry serves all languages."))
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
                .frame(height: 220)

                // What there is to configure, and whether it would start.
                // A screen listing only overrides cannot answer either
                // question, and "not installed" is the answer behind "I
                // installed a server and nothing happened".
                GroupBox("Built-in servers") {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(CoreLSPRegistry.all, id: \.id) { server in
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text(server.languages.joined(separator: ", "))
                                    .fontWeight(.semibold)
                                    .frame(width: 110, alignment: .leading)
                                Text(server.command)
                                    .font(.system(.caption, design: .monospaced))
                                Spacer()
                                if CoreLSPRegistry.isInstalled(server.command) {
                                    Label(t("found"), systemImage: "checkmark.circle")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                } else {
                                    Label(t("not installed"), systemImage: "exclamationmark.circle")
                                        .font(.caption)
                                        .foregroundStyle(.orange)
                                        .help(server.installHint)
                                }
                            }
                        }
                    }
                    .padding(6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }

                GroupBox("Add override") {
                    VStack(spacing: 8) {
                        HStack(spacing: 8) {
                            PathPicker(
                                text: $newScope,
                                placeholder: t("Project root (empty = default for all projects)"))
                            // The roots in question are usually open
                            // already; offer them instead of a path to type.
                            Menu(t("Open projects")) {
                                if model.openProjectRoots.isEmpty {
                                    Text(t("No project is open"))
                                }
                                ForEach(model.openProjectRoots, id: \.self) { root in
                                    Button((root as NSString).lastPathComponent) {
                                        newScope = root
                                    }
                                    .help(root)
                                }
                            }
                            .frame(width: 150)
                        }
                        HStack(spacing: 8) {
                            EditableCombo(
                                text: $newLanguage,
                                placeholder: t("Language (e.g. python)"),
                                options: model.knownLanguages
                            )
                            .frame(width: 180)
                            TextField(
                                "Server command (e.g. pyright-langserver --stdio)",
                                text: $newCommand)
                            Button(t("Add")) {
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
                    Text(t("Changes apply to servers started afterwards."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button(t("Restart Servers Now")) {
                        model.onRestartServers?()
                    }
                }
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 16)
        }
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
        // One scrolling column per tab: a settings screen that
        // cannot scroll turns one more setting into a setting
        // nobody can reach.
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Save preprocessors run before every save (and on Run Save Preprocessors), one command per line in order — each reads the document on standard input and writes it back on standard output, like `ruff check --fix -` then `black -`. {path} and {filename} expand to the document's. A project entry replaces the default chain.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

                List {
                    if model.preprocessorEntries.isEmpty {
                        Text(t("No preprocessors — documents save exactly as typed."))
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
                .frame(height: 220)

                GroupBox("Add preprocessor chain") {
                    VStack(spacing: 8) {
                        HStack(spacing: 8) {
                            PathPicker(
                                text: $newScope,
                                placeholder: t("Project root (empty = default for all projects)"))
                            // The roots in question are usually open
                            // already; offer them instead of a path to type.
                            Menu(t("Open projects")) {
                                if model.openProjectRoots.isEmpty {
                                    Text(t("No project is open"))
                                }
                                ForEach(model.openProjectRoots, id: \.self) { root in
                                    Button((root as NSString).lastPathComponent) {
                                        newScope = root
                                    }
                                    .help(root)
                                }
                            }
                            .frame(width: 150)
                        }
                        HStack(alignment: .top, spacing: 8) {
                            EditableCombo(
                                text: $newLanguage,
                                placeholder: t("Language (e.g. python)"),
                                options: model.knownLanguages
                            )
                            .frame(width: 180)
                            CommandsEditor(
                                placeholder: t("Commands, one per line — Return adds a line"),
                                text: $newCommands)
                            Button(t("Add")) {
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
        CommandsEditor(placeholder: t("commands, one per line"), text: $text) {
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
        window.title = t("Settings")
        // Resizable, because the tallest tab grows every time a setting
        // is added and a fixed window turns that into content nobody
        // can reach. The minimum keeps the forms from being squeezed
        // into nonsense; the autosave name means a size chosen once is
        // the size next time.
        window.styleMask = [.titled, .closable, .resizable]
        window.contentMinSize = NSSize(width: 620, height: 380)
        window.setContentSize(NSSize(width: 700, height: 560))
        window.setFrameAutosaveName("TextchumSettings")
        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("SettingsWindowController is created in code")
    }
}

// MARK: - Project records

/// Where each project's record is kept, and what becomes of the ones
/// nobody opens any more.
///
/// A record holds what the files of a project remember about
/// themselves: how each is split, where its views were looking, what is
/// folded, what it was told it is.
struct ProjectRecordsSettings: View {
    @ObservedObject var model: SettingsModel
    @State private var showingRecords = false

    var body: some View {
        Toggle(t("Keep each project's state with the checkout"), isOn: $model.projectStateInProject)
        Text(
            t("A file remembers how it is split, where each view was looking, what is folded, and what it was told it is. With this on, that is written to .tchum in the project; otherwise it is kept here, one record per project.")
        )
        .font(.caption)
        .foregroundStyle(.secondary)
        .fixedSize(horizontal: false, vertical: true)

        LabeledContent(t("Records folder")) {
            HStack(spacing: 6) {
                TextField(
                    ProjectState.directory.path, text: $model.projectStateDirectory,
                    prompt: Text(ProjectState.directory.path)
                )
                .textFieldStyle(.roundedBorder)
                Button(t("Choose…")) { chooseFolder() }
                Button(t("Manage…")) { showingRecords = true }
            }
        }
        .disabled(model.projectStateInProject)

        Toggle(t("Forget records at launch"), isOn: $model.projectStateSweep)
        Stepper(value: $model.projectStateKeepDays, in: 0...3650, step: 30) {
            Text(
                model.projectStateKeepDays == 0
                    ? "Keep records until they are forgotten by hand"
                    : "Keep records for \(model.projectStateKeepDays) days"
            )
        }
        .disabled(!model.projectStateSweep)
        Text(
            t("The sweep runs on a thread of its own at launch, and forgets the records of projects that are no longer there whatever the window says.")
        )
        .font(.caption)
        .foregroundStyle(.secondary)
        .fixedSize(horizontal: false, vertical: true)
        .sheet(isPresented: $showingRecords) {
            ProjectRecordsList(days: model.projectStateKeepDays)
        }
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        model.projectStateDirectory = url.path
    }
}

/// The records that exist, with what they are about and when they were
/// last written — and the two ways to be rid of them.
struct ProjectRecordsList: View {
    let days: Int
    @Environment(\.dismiss) private var dismiss
    @State private var records: [CoreProjectState.Record] = []
    @State private var chosen: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(t("Project Records")).font(.headline)
            if records.isEmpty {
                Text(t("No project has anything recorded yet."))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 160)
            } else {
                List(records, selection: $chosen) { record in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text((record.root as NSString).lastPathComponent)
                                .fontWeight(.medium)
                            Text(record.root)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.head)
                        }
                        Spacer()
                        VStack(alignment: .trailing, spacing: 2) {
                            Text(
                                record.missing
                                    ? t("missing") : tn("{} file", "{} files", record.files)
                            )
                            .foregroundStyle(record.missing ? .red : .secondary)
                            Text(record.updated, style: .date)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .tag(record.id)
                }
                .frame(minHeight: 220)
            }
            HStack {
                Button(t("Forget Selected")) {
                    for path in chosen { CoreProjectState.forget(recordAt: path) }
                    chosen = []
                    reload()
                }
                .disabled(chosen.isEmpty)
                Button(days == 0 ? "Forget Missing" : "Forget Older Than \(days) Days") {
                    CoreProjectState.sweep(
                        directory: ProjectState.directory.path, keepDays: UInt64(max(0, days)))
                    reload()
                }
                Spacer()
                Button(t("Done")) { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(16)
        .frame(width: 520)
        .onAppear(perform: reload)
    }

    private func reload() {
        records = CoreProjectState.records(directory: ProjectState.directory.path)
    }
}
