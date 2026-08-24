import AppKit
import SwiftUI
import TextchumKit

/// The editor-facing result of the current configuration: concrete values
/// windows can apply without knowing where they came from.
struct EditorSettings {
    let font: NSFont
    let tabWidth: Int

    /// Resolves configuration values into a usable font: the configured
    /// family if it exists on this system, the platform monospaced font
    /// otherwise.
    init(config: CoreConfig) {
        let size = config.fontSize
        if let family = config.fontFamily, let font = NSFont(name: family, size: size) {
            self.font = font
        } else {
            self.font = .monospacedSystemFont(ofSize: size, weight: .regular)
        }
        self.tabWidth = config.tabWidth
    }
}

/// Observable bridge between the settings UI and the JSON-backed core
/// configuration: every change writes through to disk immediately and
/// notifies the app so open editors update live.
@MainActor
final class SettingsModel: ObservableObject {
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
    @Published private(set) var lspEntries: [LSPEntry] = []

    init(config: CoreConfig) {
        self.config = config
        self.appearance = config.appearance
        self.openTarget = config.openTarget
        self.fontFamily = config.fontFamily ?? ""
        self.fontSize = config.fontSize
        self.tabWidth = config.tabWidth
        self.isLoading = false
        reloadLSPEntries()
    }

    var currentSettings: EditorSettings {
        EditorSettings(config: config)
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
        TabView {
            GeneralSettingsTab(model: model)
                .tabItem { Label("General", systemImage: "gearshape") }
            LanguageServersTab(model: model)
                .tabItem { Label("Language Servers", systemImage: "network") }
        }
        .frame(width: 620, height: 420)
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
        }
        .padding(.horizontal, 28)
        .padding(.vertical, 20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
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
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text(entry.scopeLabel)
                                    .fontWeight(.semibold)
                                Text(entry.language)
                                    .foregroundStyle(.secondary)
                            }
                            Text(entry.command)
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
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
                    TextField(
                        "Project root (empty = default for all projects)", text: $newScope)
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
