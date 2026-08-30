import CTextchum
import Foundation

/// The interface in another language.
///
/// The catalogues belong to the core, so both shells say the same
/// things in the same words. They are read once at launch and looked up
/// here afterwards: a label should not cross the bridge to be drawn.
public enum CoreI18n {
    private nonisolated(unsafe) static var catalogue: [String: String] = [:]

    /// Every language the build carries, as (tag, name).
    public static var languages: [(tag: String, name: String)] {
        guard let json = tc_i18n_languages() else { return [] }
        defer { tc_string_free(json) }
        let text = String(cString: json)
        guard let data = text.data(using: .utf8),
            let items = (try? JSONSerialization.jsonObject(with: data)) as? [[String]]
        else { return [] }
        return items.compactMap { pair in
            pair.count == 2 ? (pair[0], pair[1]) : nil
        }
    }

    /// The language in use, as a two-letter tag.
    public static var language: String {
        guard let json = tc_i18n_language() else { return "en" }
        defer { tc_string_free(json) }
        return String(cString: json)
    }

    /// Chooses the language: a tag, or `system` to follow the machine.
    /// User catalogues in `catalogueDirectory` are read over the ones
    /// the build carries.
    public static func use(_ tag: String, catalogueDirectory: String? = nil) {
        let locale = Locale.preferredLanguages.first ?? Locale.current.identifier
        tag.withCString { tagPointer in
            locale.withCString { localePointer in
                tc_i18n_set_language(
                    tagPointer, UInt(strlen(tagPointer)),
                    localePointer, UInt(strlen(localePointer)))
            }
        }
        if let directory = catalogueDirectory {
            directory.withCString { pointer in
                tc_i18n_set_catalogue_dir(pointer, UInt(strlen(pointer)))
            }
        }
        reload()
    }

    private static func reload() {
        guard let json = tc_i18n_catalogue() else { return }
        defer { tc_string_free(json) }
        let text = String(cString: json)
        guard let data = text.data(using: .utf8),
            let map = (try? JSONSerialization.jsonObject(with: data)) as? [String: String]
        else { return }
        catalogue = map
    }

    /// `text` in the interface language, or `text` itself when the
    /// catalogue has nothing to say about it.
    public static func translate(_ text: String) -> String {
        catalogue[text] ?? text
    }
}

/// `t("Close Tab")` — short because it wraps every label on screen.
public func t(_ text: String) -> String {
    CoreI18n.translate(text)
}

/// `t("Save changes to {}?", name)`: the argument goes where the braces
/// are, so a translation can put it where its own grammar wants it.
public func t(_ text: String, _ arguments: CVarArg...) -> String {
    var out = CoreI18n.translate(text)
    for argument in arguments {
        guard let at = out.range(of: "{}") else { break }
        out.replaceSubrange(at, with: "\(argument)")
    }
    return out
}
