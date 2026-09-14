import CTextchum
import Foundation

/// Find and replace in a document's text, in Vim's dialect when asked:
/// the core reads the pattern, finds the matches and fills in the
/// replacement's groups and case specials.
public enum CoreFind {
    public struct Options: Equatable {
        public var regex: Bool
        public var caseSensitive: Bool
        public var wholeWord: Bool

        public init(regex: Bool = false, caseSensitive: Bool = false, wholeWord: Bool = false) {
            self.regex = regex
            self.caseSensitive = caseSensitive
            self.wholeWord = wholeWord
        }
    }

    /// Every match, as UTF-16 ranges in order; nil when the pattern
    /// cannot be read.
    public static func matches(in text: String, pattern: String, options: Options) -> [NSRange]? {
        var text = text
        var pattern = pattern
        var ranges: UnsafeMutablePointer<TcRange>?
        var count: UInt = 0
        let ok = text.withUTF8 { textBytes in
            pattern.withUTF8 { patternBytes in
                tc_find_matches(
                    textBytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                    UInt(textBytes.count),
                    patternBytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                    UInt(patternBytes.count),
                    options.regex, options.caseSensitive, options.wholeWord,
                    &ranges, &count)
            }
        }
        guard ok else { return nil }
        guard let ranges else { return [] }
        defer { tc_ranges_free(ranges, count) }
        return (0..<Int(count)).map { index in
            let range = ranges[index]
            return NSRange(location: Int(range.start), length: Int(range.end - range.start))
        }
    }

    /// What is wrong with the pattern, or nil when nothing is.
    public static func problem(pattern: String, options: Options) -> String? {
        var pattern = pattern
        let raw = pattern.withUTF8 { bytes in
            tc_find_problem(
                bytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                UInt(bytes.count), options.regex, options.caseSensitive, options.wholeWord)
        }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The replacement for the `index`-th match, its groups and case
    /// specials filled in; nil when there is no such match.
    public static func expansion(
        in text: String, pattern: String, replacement: String, options: Options, matchIndex index: Int
    ) -> String? {
        var text = text
        var pattern = pattern
        var replacement = replacement
        let raw = text.withUTF8 { textBytes in
            pattern.withUTF8 { patternBytes in
                replacement.withUTF8 { replacementBytes in
                    tc_find_expansion(
                        textBytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                        UInt(textBytes.count),
                        patternBytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                        UInt(patternBytes.count),
                        replacementBytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                        UInt(replacementBytes.count),
                        options.regex, options.caseSensitive, options.wholeWord, UInt(index))
                }
            }
        }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The text with every match replaced, and how many there were;
    /// nil when the pattern cannot be read.
    public static func replaceAll(
        in text: String, pattern: String, replacement: String, options: Options
    ) -> (text: String, count: Int)? {
        var text = text
        var pattern = pattern
        var replacement = replacement
        let raw = text.withUTF8 { textBytes in
            pattern.withUTF8 { patternBytes in
                replacement.withUTF8 { replacementBytes in
                    tc_find_replace_all(
                        textBytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                        UInt(textBytes.count),
                        patternBytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                        UInt(patternBytes.count),
                        replacementBytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                        UInt(replacementBytes.count),
                        options.regex, options.caseSensitive, options.wholeWord)
                }
            }
        }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        guard let parsed = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
            let replaced = parsed["text"] as? String, let count = parsed["count"] as? Int
        else { return nil }
        return (replaced, count)
    }
}
