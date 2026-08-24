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

/// Observable bridge between the SwiftUI settings form and the JSON-backed
/// core configuration: every change writes through to disk immediately and
/// notifies the app so open editors update live.
@MainActor
final class SettingsModel: ObservableObject {
    private let config: CoreConfig
    /// Called after any setting changed and was persisted.
    var onChange: (() -> Void)?
    /// Suppresses write-back while the initial values load.
    private var isLoading = true

    @Published var appearance: CoreAppearance {
        didSet { persist { $0.appearance = appearance } }
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

    init(config: CoreConfig) {
        self.config = config
        self.appearance = config.appearance
        self.fontFamily = config.fontFamily ?? ""
        self.fontSize = config.fontSize
        self.tabWidth = config.tabWidth
        self.isLoading = false
    }

    var currentSettings: EditorSettings {
        EditorSettings(config: config)
    }

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

/// The settings form. Deliberately small: these are the only recognized
/// settings today, and the JSON file remains the escape hatch for anything
/// beyond them.
struct SettingsView: View {
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
        .padding(20)
        .frame(width: 380)
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
