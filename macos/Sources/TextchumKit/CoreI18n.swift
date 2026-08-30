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

    /// One or many, asked of the core: the plural rule belongs to the
    /// catalogue, and only the core can read it.
    public static func plural(one: String, many: String, count: Int) -> String {
        let answer = one.withCString { onePointer in
            many.withCString { manyPointer in
                tc_i18n_ngettext(
                    onePointer, UInt(strlen(onePointer)),
                    manyPointer, UInt(strlen(manyPointer)),
                    UInt64(max(0, count)))
            }
        }
        guard let answer else { return count == 1 ? one : many }
        defer { tc_string_free(answer) }
        let text = String(cString: answer)
        return text.isEmpty ? (count == 1 ? one : many) : text
    }
}

/// Marks a string for translation without translating it yet — the
/// tables whose titles are looked up later through a variable, which no
/// extractor can follow. C spells this `N_()`.
public func n_(_ text: String) -> String { text }

/// `t("Close Tab")` — short because it wraps every label on screen.
public func t(_ text: String) -> String {
    CoreI18n.translate(text)
}

/// `tn("{} file", "{} files", n)`: the count decides which form the
/// language wants — two in English, and whatever its own rule says
/// elsewhere — and then goes where the braces are.
public func tn(_ one: String, _ many: String, _ count: Int) -> String {
    var out = CoreI18n.plural(one: one, many: many, count: count)
    if let at = out.range(of: "{}") {
        out.replaceSubrange(at, with: "\(count)")
    }
    return out
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
