//! Parsing and applying LSP text edits — the Rust twin of the macOS
//! shell's LSPEdits helper. Positions are (zero-based line, UTF-16
//! column); application is bottom-up so earlier ranges never shift.

use std::collections::HashMap;

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct TextEdit {
    pub start_line: i32,
    pub start_character: usize,
    pub end_line: i32,
    pub end_character: usize,
    pub new_text: String,
}

fn edit_from(value: &Value) -> Option<TextEdit> {
    let range = &value["range"];
    Some(TextEdit {
        start_line: range["start"]["line"].as_i64()? as i32,
        start_character: range["start"]["character"].as_u64()? as usize,
        end_line: range["end"]["line"].as_i64()? as i32,
        end_character: range["end"]["character"].as_u64()? as usize,
        new_text: value["newText"].as_str()?.to_owned(),
    })
}

/// Parses a `TextEdit[]` result (formatting).
pub fn text_edits(json: &str) -> Vec<TextEdit> {
    let Ok(parsed) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    parsed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(edit_from)
        .collect()
}

/// Parses a `WorkspaceEdit` (rename): both the `changes` map and the
/// `documentChanges` array of TextDocumentEdits. Keys are file paths.
pub fn workspace_edits(json: &str) -> HashMap<String, Vec<TextEdit>> {
    let mut result: HashMap<String, Vec<TextEdit>> = HashMap::new();
    let Ok(parsed) = serde_json::from_str::<Value>(json) else {
        return result;
    };
    let mut add = |uri: &str, edits: &Value| {
        let Some(path) = uri.strip_prefix("file://") else { return };
        let path = crate::workbench::percent_decode(path);
        let list = result.entry(path).or_default();
        for edit in edits.as_array().into_iter().flatten() {
            if let Some(edit) = edit_from(edit) {
                list.push(edit);
            }
        }
    };
    if let Some(changes) = parsed["changes"].as_object() {
        for (uri, edits) in changes {
            add(uri, edits);
        }
    }
    for change in parsed["documentChanges"].as_array().into_iter().flatten() {
        if let Some(uri) = change["textDocument"]["uri"].as_str() {
            add(uri, &change["edits"]);
        }
    }
    result
}

/// Bottom-up order: later positions first, so applying one edit never
/// shifts the ranges of those still pending.
pub fn bottom_up(mut edits: Vec<TextEdit>) -> Vec<TextEdit> {
    edits.sort_by(|a, b| {
        (b.start_line, b.start_character).cmp(&(a.start_line, a.start_character))
    });
    edits
}

/// Applies edits to a plain string (for files no page has open). Lines
/// are addressed the LSP way; columns are UTF-16 units.
pub fn apply_to_string(text: &str, edits: Vec<TextEdit>) -> String {
    fn byte_offset(text: &str, line: i32, character: usize) -> usize {
        let mut current_line = 0;
        let mut line_start = 0;
        if line > 0 {
            for (index, byte) in text.bytes().enumerate() {
                if byte == b'\n' {
                    current_line += 1;
                    if current_line == line {
                        line_start = index + 1;
                        break;
                    }
                }
            }
            if current_line < line {
                return text.len();
            }
        }
        let rest = &text[line_start..];
        let mut utf16 = 0usize;
        for (offset, ch) in rest.char_indices() {
            if utf16 >= character || ch == '\n' {
                return line_start + offset;
            }
            utf16 += ch.len_utf16();
        }
        text.len()
    }

    let mut result = text.to_owned();
    for edit in bottom_up(edits) {
        let start = byte_offset(&result, edit.start_line, edit.start_character);
        let end = byte_offset(&result, edit.end_line, edit.end_character).max(start);
        result.replace_range(start..end, &edit.new_text);
    }
    result
}
