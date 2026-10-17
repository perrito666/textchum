//! A language server's colouring of the text: semantic tokens.
//!
//! The grammar knows the shape of the text; the server knows what each
//! name is. `x` after `let` is a variable to the grammar, and a
//! parameter, a constant or a mutable binding to the server. Its tokens
//! are painted over the grammar's spans, under the same capture names
//! the themes already style, so a theme colours both alike and a token
//! of a kind no theme knows leaves the grammar's colour in place.
//!
//! Tokens arrive as the protocol's packed integers: five per token,
//! each line and start relative to the one before, the kind an index
//! into the legend the server gave at initialize. They are decoded
//! against the text as it is when they arrive, held as UTF-16 ranges,
//! and moved by every edit until the next answer replaces them.

use serde_json::Value;

/// One token, as a range of the current text and the style it paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    /// UTF-16 offsets into the text.
    pub start: usize,
    pub end: usize,
    /// Index into the theme's style table.
    pub style: u32,
}

/// The capture a token of `kind` with `modifiers` is painted as; None
/// for the kinds left to the grammar — punctuation, for one, which the
/// grammar already gets right, and names the server could not resolve,
/// which it has no better word for.
pub fn capture(kind: &str, modifiers: &[&str]) -> Option<&'static str> {
    let has = |modifier: &str| modifiers.contains(&modifier);
    let library = has("defaultLibrary");
    Some(match kind {
        "namespace" | "toolModule" | "crateRoot" => "namespace",
        "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" | "typeAlias"
        | "union" | "trait" => {
            if library {
                "type.builtin"
            } else {
                "type"
            }
        }
        "builtinType" | "selfTypeKeyword" => "type.builtin",
        "parameter" => "variable.parameter",
        "variable" => {
            if (has("readonly") && !has("local")) || has("constant") || has("static") {
                "constant"
            } else if library {
                "variable.builtin"
            } else {
                "variable"
            }
        }
        "selfKeyword" => "variable.builtin",
        "property" | "event" => "property",
        "enumMember" | "constParameter" | "const" | "static" => "constant",
        "function" | "method" | "macro" | "procMacro" => {
            if library {
                "function.builtin"
            } else {
                "function"
            }
        }
        "keyword" => "keyword",
        "modifier" => "storageclass",
        "comment" => "comment",
        "string" => "string",
        "number" => "number",
        "boolean" => "boolean",
        "character" => "character",
        "regexp" | "formatSpecifier" => "string.special",
        "escapeSequence" => "escape",
        "operator" => "operator",
        "decorator" | "attribute" | "builtinAttribute" | "derive" | "deriveHelper" => "attribute",
        "label" | "lifetime" => "label",
        _ => return None,
    })
}

/// Decodes a `textDocument/semanticTokens/full` result against the
/// server's `legend`. `offset_at(line, character)` turns an LSP
/// position of the current text into a UTF-16 offset. Tokens of kinds
/// left to the grammar, and any that come out empty, are dropped; the
/// rest come back in text order.
pub fn decode(
    result: &Value,
    legend: &Value,
    offset_at: impl Fn(usize, usize) -> usize,
) -> Vec<Token> {
    let Some(data) = result["data"].as_array() else { return Vec::new() };
    let kinds: Vec<&str> = names(&legend["tokenTypes"]);
    let modifier_names: Vec<&str> = names(&legend["tokenModifiers"]);
    let numbers: Vec<usize> = data.iter().map(|n| n.as_u64().unwrap_or(0) as usize).collect();
    let mut tokens = Vec::new();
    let (mut line, mut character) = (0usize, 0usize);
    for packed in numbers.chunks_exact(5) {
        let [delta_line, delta_start, length, kind, bits] = [packed[0], packed[1], packed[2], packed[3], packed[4]];
        if delta_line > 0 {
            line += delta_line;
            character = delta_start;
        } else {
            character += delta_start;
        }
        let Some(kind) = kinds.get(kind) else { continue };
        let modifiers: Vec<&str> = modifier_names
            .iter()
            .enumerate()
            .filter(|(bit, _)| *bit < usize::BITS as usize && bits & (1 << bit) != 0)
            .map(|(_, name)| *name)
            .collect();
        let Some(style) = capture(kind, &modifiers).and_then(crate::syntax::theme::resolve) else {
            continue;
        };
        let start = offset_at(line, character);
        // A token never runs past its line; one that would, because the
        // text changed under it, is cut at the line's end.
        let end = offset_at(line, character + length);
        if end > start {
            tokens.push(Token { start, end, style });
        }
    }
    tokens.sort_by_key(|token| token.start);
    tokens
}

fn names(list: &Value) -> Vec<&str> {
    list.as_array()
        .map(|names| names.iter().map(|name| name.as_str().unwrap_or_default()).collect())
        .unwrap_or_default()
}

/// Where a token ends up after `start..end` of the text was replaced
/// by `new_len` units; None when the edit touched it. A token is a
/// name the server read: once its text is changed, what it named is
/// not known until the server reads it again, and the grammar's colour
/// is the honest one meanwhile. Typing right after a name is typing in
/// it, so that drops it too.
pub fn shifted(token: (usize, usize), start: usize, end: usize, new_len: usize) -> Option<(usize, usize)> {
    let (from, to) = token;
    if to < start {
        Some(token)
    } else if from >= end {
        let delta = new_len as isize - (end - start) as isize;
        let moved = |offset: usize| (offset as isize + delta).max(0) as usize;
        Some((moved(from), moved(to)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn legend() -> Value {
        json!({
            "tokenTypes": ["variable", "parameter", "function", "operator", "unknownKind"],
            "tokenModifiers": ["declaration", "readonly", "defaultLibrary"],
        })
    }

    #[test]
    fn decodes_relative_positions_against_the_legend() {
        // Lines of ten units each, so an offset is line * 10 + column.
        let result = json!({"data": [
            0, 4, 1, 0, 0,     // line 0, col 4: a variable
            0, 4, 3, 2, 4,     // line 0, col 8: a library function
            2, 1, 2, 1, 0,     // line 2, col 1: a parameter
            0, 3, 1, 3, 0,     // line 2, col 4: an operator
            0, 2, 1, 4, 0,     // a kind no theme knows: left to the grammar
            1, 0, 5, 0, 2,     // line 3: a readonly variable is a constant
        ]});
        let tokens = decode(&result, &legend(), |line, character| line * 10 + character);
        let resolve = |name| crate::syntax::theme::resolve(name).unwrap();
        assert_eq!(
            tokens,
            vec![
                Token { start: 4, end: 5, style: resolve("variable") },
                Token { start: 8, end: 11, style: resolve("function.builtin") },
                Token { start: 21, end: 23, style: resolve("variable.parameter") },
                Token { start: 24, end: 25, style: resolve("operator") },
                Token { start: 30, end: 35, style: resolve("constant") },
            ]
        );
    }

    #[test]
    fn kinds_a_server_adds_map_onto_the_ones_themes_know() {
        // rust-analyzer's own kinds, beside the standard ones.
        assert_eq!(capture("const", &[]), Some("constant"));
        assert_eq!(capture("static", &[]), Some("constant"));
        assert_eq!(capture("builtinType", &[]), Some("type.builtin"));
        assert_eq!(capture("selfKeyword", &[]), Some("variable.builtin"));
        assert_eq!(capture("lifetime", &[]), Some("label"));
        // Punctuation and names the server could not resolve stay the
        // grammar's.
        assert_eq!(capture("semicolon", &[]), None);
        assert_eq!(capture("unresolvedReference", &[]), None);
    }

    #[test]
    fn a_result_without_data_decodes_to_nothing() {
        assert!(decode(&json!(null), &legend(), |l, c| l + c).is_empty());
        assert!(decode(&json!({"data": [0, 0, 3]}), &legend(), |l, c| l + c).is_empty());
    }

    #[test]
    fn tokens_move_with_edits_and_go_when_touched() {
        // Before the edit, untouched.
        assert_eq!(shifted((2, 5), 8, 8, 3), Some((2, 5)));
        // After it, moved by the change in length.
        assert_eq!(shifted((10, 14), 2, 4, 5), Some((13, 17)));
        assert_eq!(shifted((10, 14), 2, 6, 0), Some((6, 10)));
        // An insertion right before the name pushes it along.
        assert_eq!(shifted((10, 14), 10, 10, 2), Some((12, 16)));
        // Typing inside it, or right after it, or replacing across it.
        assert_eq!(shifted((10, 14), 12, 12, 1), None);
        assert_eq!(shifted((10, 14), 14, 14, 1), None);
        assert_eq!(shifted((10, 14), 8, 11, 0), None);
    }
    #[test]
    fn a_gopls_answer_reads_as_go_does() {
        // gopls's own legend and its answer for this file, as it sent
        // them: its constants are readonly variables, its builtin types
        // carry defaultLibrary.
        let text = "package main\n\nimport \"fmt\"\n\nconst Limit = 3\n\nfunc add(a, b int) int {\n\ttotal := a + b\n\treturn total\n}\n\nfunc main() {\n\tfmt.Println(add(1, Limit))\n}\n";
        let legend = json!({
            "tokenTypes": ["namespace", "type", "typeParameter", "parameter", "property", "variable",
                "function", "method", "macro", "keyword", "comment", "string", "number", "operator", "label"],
            "tokenModifiers": ["definition", "readonly", "defaultLibrary", "static", "array", "bool",
                "chan", "format", "interface", "map", "number", "pointer", "signature", "slice",
                "string", "struct", "shadowing"],
        });
        let result: Value = serde_json::from_str(r#"{"data": [0, 0, 7, 9, 0, 0, 8, 4, 0, 0, 2, 0, 6, 9, 0, 0, 8, 3, 0, 0, 2, 0, 5, 9, 0, 0, 6, 5, 5, 3, 0, 8, 1, 12, 0, 2, 0, 4, 9, 0, 0, 5, 3, 6, 1, 0, 4, 1, 3, 1, 0, 3, 1, 3, 1, 0, 2, 3, 1, 4, 0, 5, 3, 1, 4, 1, 1, 5, 5, 1, 0, 6, 2, 13, 0, 0, 3, 1, 3, 0, 0, 2, 1, 13, 0, 0, 2, 1, 3, 0, 1, 1, 6, 9, 0, 0, 7, 5, 5, 0, 3, 0, 4, 9, 0, 0, 5, 4, 6, 1, 1, 1, 3, 0, 0, 0, 4, 7, 6, 0, 0, 8, 3, 6, 0, 0, 4, 1, 12, 0, 0, 3, 5, 5, 2]}"#).unwrap();
        let starts: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        let tokens = decode(&result, &legend, |line, character| starts[line] + character);
        let style_of = |needle: &str, nth: usize| {
            let at = text.match_indices(needle).nth(nth).unwrap().0;
            tokens.iter().find(|token| token.start == at).map(|token| token.style)
        };
        let resolve = |name| crate::syntax::theme::resolve(name);
        assert_eq!(style_of("Limit", 0), resolve("constant"));
        assert_eq!(style_of("Limit", 1), resolve("constant"));
        assert_eq!(style_of("int", 0), resolve("type.builtin"));
        assert_eq!(style_of("add", 0), resolve("function"));
        assert_eq!(style_of("a,", 0), resolve("variable.parameter"));
        assert_eq!(style_of("total", 0), resolve("variable"));
        assert_eq!(style_of("fmt", 1), resolve("namespace"));
        assert_eq!(style_of("func", 0), resolve("keyword"));
    }
}
