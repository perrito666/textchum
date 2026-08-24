import Foundation

/// Subsequence matching with a light preference for tight, early
/// matches — the fzf spirit at panel scale. Shared by the command
/// palette and the document outline.
enum Fuzzy {
    /// Higher is better; nil means no match. The empty query matches
    /// everything equally.
    static func score(_ haystack: String, query: String) -> Int? {
        if query.isEmpty { return 0 }
        let haystack = Array(haystack.lowercased())
        let needle = Array(query.lowercased())
        var position = 0
        var first = -1
        var last = -1
        for character in needle {
            while position < haystack.count, haystack[position] != character {
                position += 1
            }
            guard position < haystack.count else { return nil }
            if first < 0 { first = position }
            last = position
            position += 1
        }
        // Smaller span and earlier start rank higher.
        return -(last - first) * 4 - first
    }
}
