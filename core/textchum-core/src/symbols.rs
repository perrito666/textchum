//! Symbols across a project, as a list to pick from.
//!
//! `workspace/symbol` answers with every symbol whose name matches a
//! query, in one of two shapes depending on the server's age, each
//! entry carrying a numeric kind and a URI. A picker wants rows: a
//! name, what kind of thing it is in a word, what it sits inside, and a
//! path and line to jump to. The reduction is here so both shells list
//! the same rows.

use std::path::Path;

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
    list_json(&rows(result_json))
}

fn list_json(symbols: &[Symbol]) -> String {
    Value::Array(symbols.iter().map(to_json).collect()).to_string()
}

fn to_json(symbol: &Symbol) -> Value {
    serde_json::json!({
        "name": symbol.name,
        "kind": symbol.kind,
        "container": symbol.container,
        "path": symbol.path,
        "line": symbol.line,
        "character": symbol.character,
    })
}

/// A row as [`to_json`] wrote it; None for anything else.
pub fn from_json(value: &Value) -> Option<Symbol> {
    let kind = value["kind"].as_str()?;
    Some(Symbol {
        name: value["name"].as_str()?.to_owned(),
        kind: KINDS.iter().find(|known| **known == kind).copied().unwrap_or("symbol"),
        container: value["container"].as_str().unwrap_or_default().to_owned(),
        path: value["path"].as_str()?.to_owned(),
        line: value["line"].as_u64().unwrap_or(0) as u32,
        character: value["character"].as_u64().unwrap_or(0) as u32,
    })
}

// MARK: Recent symbols

/// The kind of choice recent symbols are kept as in a project's memory.
pub const RECENT: &str = "symbols";

/// Whether two rows are the same symbol: the same name, kind and
/// container in the same file. The line is not part of it; code moves.
fn same_symbol(a: &Symbol, b: &Symbol) -> bool {
    a.name == b.name && a.kind == b.kind && a.container == b.container && a.path == b.path
}

/// Puts `symbol` first among a project's recent symbols. Chosen again
/// after its line moved, it replaces its old entry rather than
/// standing beside it.
pub fn remember(root: &Path, dir: &Path, in_project: bool, symbol: &Symbol) -> std::io::Result<()> {
    let mut state = crate::project_state::load(root, dir, in_project);
    let list = state.memory.recent.entry(RECENT.to_owned()).or_default();
    list.retain(|known| {
        serde_json::from_str::<Value>(known)
            .ok()
            .and_then(|value| from_json(&value))
            .is_none_or(|known| !same_symbol(&known, symbol))
    });
    list.insert(0, to_json(symbol).to_string());
    list.truncate(crate::project_state::RECENT_CAP);
    crate::project_state::save(&state, root, dir, in_project)
}

/// A project's recent symbols, most recent first, each where its name
/// stands in its file now. A symbol is remembered with the line it was
/// on, and lines move; the nearest line that still has the name as a
/// word is taken instead, and the remembered one when none does. A
/// symbol whose file is gone is left out.
pub fn recent(root: &Path, dir: &Path, in_project: bool) -> Vec<Symbol> {
    let state = crate::project_state::load(root, dir, in_project);
    state
        .memory
        .recent(RECENT)
        .iter()
        .filter_map(|entry| from_json(&serde_json::from_str(entry).ok()?))
        .filter_map(|mut symbol| {
            let text = std::fs::read_to_string(&symbol.path).ok()?;
            if let Some((line, character)) = relocate(&text, symbol.line, &symbol.name) {
                (symbol.line, symbol.character) = (line, character);
            }
            Some(symbol)
        })
        .collect()
}

/// [`recent`] as the JSON [`rows_json`] gives.
pub fn recent_json(root: &Path, dir: &Path, in_project: bool) -> String {
    list_json(&recent(root, dir, in_project))
}

/// The line nearest `line` on which `name` stands as a whole word, and
/// its column in UTF-16 units. Between two as near, the one below wins:
/// code moves down as lines are written above it.
fn relocate(text: &str, line: u32, name: &str) -> Option<(u32, u32)> {
    let lines: Vec<&str> = text.lines().collect();
    let line = line as usize;
    let at = |number: usize| -> Option<(u32, u32)> {
        let text = lines.get(number)?;
        let column = whole_word(text, name)?;
        Some((number as u32, text[..column].encode_utf16().count() as u32))
    };
    (0..lines.len().max(line + 1)).find_map(|distance| {
        at(line + distance).or_else(|| line.checked_sub(distance).and_then(at))
    })
}

fn whole_word(text: &str, name: &str) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(name).map(|(at, _)| at).find(|&at| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

/// The protocol's symbol kinds in a word, by number less one.
const KINDS: [&str; 26] = [
    "file", "module", "namespace", "package", "class", "method", "property", "field",
    "constructor", "enum", "interface", "function", "variable", "constant", "string", "number",
    "boolean", "array", "object", "key", "null", "enum member", "struct", "event", "operator",
    "type parameter",
];

fn kind_name(kind: u64) -> &'static str {
    kind.checked_sub(1).and_then(|at| KINDS.get(at as usize)).copied().unwrap_or("symbol")
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
    fn recent_symbols_are_one_each_and_found_where_they_moved() {
        let dir = std::env::temp_dir().join(format!("textchum-recent-symbols-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("proj")).unwrap();
        let root = dir.join("proj");
        let file = root.join("lib.rs");
        std::fs::write(&file, "struct Server;\n\nfn serve() {}\n").unwrap();
        let records = dir.join("records");
        let symbol = |name: &str, kind: &'static str, line: u32| Symbol {
            name: name.into(),
            kind,
            container: String::new(),
            path: file.to_string_lossy().into_owned(),
            line,
            character: 0,
        };
        remember(&root, &records, false, &symbol("Server", "struct", 0)).unwrap();
        remember(&root, &records, false, &symbol("serve", "function", 2)).unwrap();
        // Chosen again from another line, it moves to the front once.
        remember(&root, &records, false, &symbol("Server", "struct", 5)).unwrap();
        let names: Vec<(String, u32)> =
            recent(&root, &records, false).into_iter().map(|s| (s.name, s.line)).collect();
        // Line 5 is past the end; the name is found where it stands.
        assert_eq!(names, vec![("Server".into(), 0), ("serve".into(), 2)]);
        // Lines typed above move it; "serve" is not found inside
        // "Server" or "server_name", only as a word.
        std::fs::write(&file, "// a\n// b\nstruct Server;\nlet server_name = 1;\nfn serve() {}\n").unwrap();
        let lines: Vec<u32> = recent(&root, &records, false).iter().map(|s| s.line).collect();
        assert_eq!(lines, vec![2, 4]);
        // A file that is gone takes its symbols with it.
        std::fs::remove_file(&file).unwrap();
        assert!(recent(&root, &records, false).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_is_found_as_a_word_nearest_its_line() {
        let text = "fn a() {}\nfn ab() {}\nfn a() {}\n";
        assert_eq!(relocate(text, 1, "a"), Some((2, 3)), "below wins a tie: lines are added above");
        assert_eq!(relocate(text, 2, "a"), Some((2, 3)));
        assert_eq!(relocate(text, 0, "zz"), None);
        assert_eq!(relocate("é a", 0, "a"), Some((0, 2)), "columns in UTF-16 units");
    }

    #[test]
    fn no_answer_is_no_rows() {
        assert!(rows("null").is_empty());
        assert_eq!(rows_json("not json"), "[]");
    }
}
