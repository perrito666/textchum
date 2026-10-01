//! Applying a server's `TextEdit[]` to a text.
//!
//! The shells apply edits to the document they show, each through its
//! own choke point. This is for the case with no document in hand: a
//! text on its way through a save chain, which the server is asked to
//! format and which comes back as edits against the text it was sent.

use serde_json::Value;

use crate::Buffer;

/// `text` with the edits of a `TextEdit[]` result applied. Positions
/// are (zero-based line, UTF-16 column), as servers give them. A result
/// that is not an array — `null`, which servers answer when there is
/// nothing to change — leaves the text as it is.
pub fn apply(text: &str, edits_json: &str) -> String {
    let Ok(Value::Array(edits)) = serde_json::from_str::<Value>(edits_json) else {
        return text.to_owned();
    };
    let mut buffer = Buffer::from_str(text);
    let offset = |buffer: &Buffer, position: &Value| -> usize {
        let line = position["line"].as_u64().unwrap_or(0) as usize;
        let character = position["character"].as_u64().unwrap_or(0) as usize;
        // A line past the last is the end of the text: "replace
        // everything" is often written as a range ending one line after
        // the document.
        if line >= buffer.len_lines() {
            buffer.len_utf16()
        } else {
            buffer.utf16_offset_at(line, character)
        }
    };
    let mut placed: Vec<(usize, usize, &str)> = edits
        .iter()
        .filter_map(|edit| {
            let start = offset(&buffer, &edit["range"]["start"]);
            let end = offset(&buffer, &edit["range"]["end"]).max(start);
            Some((start, end, edit["newText"].as_str()?))
        })
        .collect();
    // Bottom-up, so an edit never moves the ones still to apply. The
    // sort is stable: edits at one position keep the order given.
    placed.sort_by(|a, b| b.0.cmp(&a.0));
    for (start, end, new_text) in placed {
        let _ = buffer.replace_utf16(start, end, new_text);
    }
    buffer.text()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_apply_bottom_up_whatever_order_they_come_in() {
        let edits = r#"[
            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}}, "newText": "fn"},
            {"range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 0}}, "newText": "    "}
        ]"#;
        assert_eq!(apply("FN main() {\nx();\n}\n", edits), "fn main() {\n    x();\n}\n");
    }

    #[test]
    fn a_range_past_the_end_reaches_the_end() {
        let whole = r#"[{"range": {"start": {"line": 0, "character": 0},
                                   "end": {"line": 99, "character": 0}}, "newText": "new\n"}]"#;
        assert_eq!(apply("old\nwithout a last newline", whole), "new\n");
    }

    #[test]
    fn nothing_to_change_changes_nothing() {
        assert_eq!(apply("as it was\n", "null"), "as it was\n");
        assert_eq!(apply("as it was\n", "[]"), "as it was\n");
    }

    #[test]
    fn columns_are_utf16_units() {
        let edit = r#"[{"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 4}}, "newText": "B"}]"#;
        // "𝔘" is two units, so column 3 is the "b" after it and a space... the letter b.
        assert_eq!(apply("𝔘 b c", edit), "𝔘 B c");
    }
}
