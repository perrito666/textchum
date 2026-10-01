//! Symbols across a project, as a list to pick from.
//!
//! `workspace/symbol` answers with every symbol whose name matches a
//! query, in one of two shapes depending on the server's age, each
//! entry carrying a numeric kind and a URI. A picker wants rows: a
//! name, what kind of thing it is in a word, what it sits inside, and a
//! path and line to jump to. The reduction is here so both shells list
//! the same rows.

use serde_json::Value;

use crate::definition::uri_path;

/// One symbol, as a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// The kind in a word: `function`, `struct`, `constant`…
    pub kind: &'static str,
    /// What it is declared inside, when the server says.
    pub container: String,
    pub path: String,
    /// Zero-based, as servers count.
    pub line: u32,
    pub character: u32,
}

/// The rows of a `workspace/symbol` result. Entries without a file to
/// open — a URI that is not a file, or none at all — are left out.
pub fn rows(result_json: &str) -> Vec<Symbol> {
    let Ok(Value::Array(found)) = serde_json::from_str::<Value>(result_json) else {
        return Vec::new();
    };
    found
        .iter()
        .filter_map(|item| {
            let location = &item["location"];
            let start = &location["range"]["start"];
            Some(Symbol {
                name: item["name"].as_str()?.to_owned(),
                kind: kind_name(item["kind"].as_u64().unwrap_or(0)),
                container: item["containerName"].as_str().unwrap_or_default().to_owned(),
                path: uri_path(location["uri"].as_str()?)?,
                // The newer shape may leave the range for a later
                // request; the top of the file is where it opens then.
                line: start["line"].as_u64().unwrap_or(0) as u32,
                character: start["character"].as_u64().unwrap_or(0) as u32,
            })
        })
        .collect()
}

/// [`rows`] as a JSON array of `{name, kind, container, path, line,
/// character}`, for a shell across the C boundary.
pub fn rows_json(result_json: &str) -> String {
    let rows: Vec<Value> = rows(result_json)
        .into_iter()
        .map(|symbol| {
            serde_json::json!({
                "name": symbol.name,
                "kind": symbol.kind,
                "container": symbol.container,
                "path": symbol.path,
                "line": symbol.line,
                "character": symbol.character,
            })
        })
        .collect();
    Value::Array(rows).to_string()
}

/// The protocol's symbol kinds, by number.
fn kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enum member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type parameter",
        _ => "symbol",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_shapes_of_answer_become_rows() {
        let json = r#"[
            {"name": "Server", "kind": 23, "containerName": "net",
             "location": {"uri": "file:///work/a%20b/server.rs",
                          "range": {"start": {"line": 11, "character": 11},
                                    "end": {"line": 11, "character": 17}}}},
            {"name": "broadcast", "kind": 6,
             "location": {"uri": "file:///work/host.rs"}},
            {"name": "remote", "kind": 12, "location": {"uri": "untitled:1"}},
            {"kind": 12, "location": {"uri": "file:///work/nameless.rs"}}
        ]"#;
        let found = rows(json);
        assert_eq!(found.len(), 2, "what cannot be opened or named is left out");
        assert_eq!(
            found[0],
            Symbol {
                name: "Server".into(),
                kind: "struct",
                container: "net".into(),
                path: "/work/a b/server.rs".into(),
                line: 11,
                character: 11,
            }
        );
        assert_eq!((found[1].kind, found[1].line, &found[1].container[..]), ("method", 0, ""));
    }

    #[test]
    fn no_answer_is_no_rows() {
        assert!(rows("null").is_empty());
        assert_eq!(rows_json("not json"), "[]");
    }
}
