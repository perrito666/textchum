import Foundation

/// One place in a file that a list can jump to.
struct ReferenceLocation {
    let path: String
    /// Zero-based, LSP-style.
    let line: Int
    let character: Int
}

/// One symbol of a document outline, with the depth its nesting had.
struct OutlineSymbol {
    let name: String
    let kind: String
    /// Zero-based, LSP-style.
    let line: Int
    let character: Int
    let depth: Int
}

extension OutlineSymbol {
    /// Parses a documentSymbol result: `DocumentSymbol[]` (hierarchical,
    /// flattened depth-first) or `SymbolInformation[]` (already flat).
    static func parse(resultJSON json: String) -> [OutlineSymbol] {
        guard let data = json.data(using: .utf8),
            let array = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]]
        else { return [] }
        var flattened: [OutlineSymbol] = []
        func position(of raw: [String: Any], key: String) -> (Int, Int)? {
            guard let range = raw[key] as? [String: Any],
                let start = range["start"] as? [String: Any],
                let line = start["line"] as? Int,
                let character = start["character"] as? Int
            else { return nil }
            return (line, character)
        }
        func walk(_ nodes: [[String: Any]], depth: Int) {
            for node in nodes {
                guard let name = node["name"] as? String else { continue }
                let kind = kindLabel(node["kind"] as? Int ?? 0)
                if let position = position(of: node, key: "selectionRange")
                    ?? position(of: node, key: "range")
                {
                    // DocumentSymbol: ranges live on the node itself.
                    flattened.append(
                        OutlineSymbol(
                            name: name, kind: kind,
                            line: position.0, character: position.1, depth: depth))
                    if let children = node["children"] as? [[String: Any]] {
                        walk(children, depth: depth + 1)
                    }
                } else if let location = node["location"] as? [String: Any],
                    let position = position(of: location, key: "range")
                {
                    // SymbolInformation: flat, with a Location.
                    flattened.append(
                        OutlineSymbol(
                            name: name, kind: kind,
                            line: position.0, character: position.1, depth: 0))
                }
            }
        }
        walk(array, depth: 0)
        return flattened
    }

    private static func kindLabel(_ kind: Int) -> String {
        switch kind {
        case 1: "file"
        case 2: "module"
        case 3: "namespace"
        case 4: "package"
        case 5: "class"
        case 6: "method"
        case 7: "property"
        case 8: "field"
        case 9: "constructor"
        case 10: "enum"
        case 11: "interface"
        case 12: "function"
        case 13: "variable"
        case 14: "constant"
        case 22: "enum member"
        case 23: "struct"
        case 25: "operator"
        case 26: "type parameter"
        default: ""
        }
    }
}
