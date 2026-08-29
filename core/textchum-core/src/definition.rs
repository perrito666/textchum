//! What to do with a definition answer.
//!
//! Jump to Definition has nowhere to go when the caret is already on
//! the definition: the server answers with the range the caret sits in,
//! and jumping there moves nothing. The question that remains at that
//! point is the other half — who uses this — so the same key runs Find
//! References.
//!
//! The answer already in hand decides it. The definition request is
//! being made anyway, and its range says whether the caret is inside
//! it, so nothing extra is asked of the server.

use serde_json::Value;

/// A place a server pointed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub path: String,
    pub line: u32,
    pub character: u32,
}

/// What the shell should do with a definition answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The server pointed nowhere. The shell falls back to ctags, or
    /// says so.
    Nothing,
    /// One place, elsewhere.
    Jump(Target),
    /// The caret is inside the only definition offered: run Find
    /// References instead.
    AlreadyThere,
    /// Several definitions. The reader picks, the way references are
    /// picked.
    Choose(Vec<Target>),
}

/// Reads a `textDocument/definition` result and decides.
///
/// `path`, `line` and `character` are where the caret is, in the LSP's
/// own terms (zero-based, UTF-16 units).
pub fn decide(result: &str, path: &str, line: u32, character: u32) -> Decision {
    let ranges = ranges(result);
    match ranges.len() {
        0 => Decision::Nothing,
        1 => {
            let single = &ranges[0];
            if single.contains(path, line, character) {
                Decision::AlreadyThere
            } else {
                Decision::Jump(single.target())
            }
        }
        _ => {
            // One of several may be the caret's own line — a server
            // that answers with both a declaration and an
            // implementation, say. The reader is still choosing
            // between the rest.
            let elsewhere: Vec<Target> = ranges
                .iter()
                .filter(|range| !range.contains(path, line, character))
                .map(Located::target)
                .collect();
            match elsewhere.len() {
                0 => Decision::AlreadyThere,
                1 => Decision::Jump(elsewhere.into_iter().next().unwrap()),
                _ => Decision::Choose(elsewhere),
            }
        }
    }
}

/// The reference locations that are not the one the caret is in.
///
/// Find References includes the declaration, so a definition nobody
/// calls answers with the line the caret is already on. Dropping it
/// leaves the uses, which is what was asked.
pub fn elsewhere(result: &str, path: &str, line: u32, character: u32) -> Vec<Target> {
    ranges(result)
        .iter()
        .filter(|range| !range.contains(path, line, character))
        .map(Located::target)
        .collect()
}

/// The decision as JSON, for shells that reach the core through the C
/// ABI:
///
/// ```json
/// {"action": "jump", "targets": [{"path": "/p/lib.rs", "line": 40,
///                                 "character": 3}]}
/// ```
///
/// `action` is one of `nothing`, `jump`, `references` or `choose`.
/// `references` means the caret is on the definition and the shell
/// should ask who uses it.
pub fn to_json(decision: &Decision) -> String {
    let (action, targets) = match decision {
        Decision::Nothing => ("nothing", Vec::new()),
        Decision::AlreadyThere => ("references", Vec::new()),
        Decision::Jump(target) => ("jump", vec![target.clone()]),
        Decision::Choose(targets) => ("choose", targets.clone()),
    };
    serde_json::json!({"action": action, "targets": json_targets(&targets)}).to_string()
}

/// A list of places as JSON, in the same shape `to_json` uses.
pub fn targets_to_json(targets: &[Target]) -> String {
    serde_json::Value::Array(json_targets(targets)).to_string()
}

fn json_targets(targets: &[Target]) -> Vec<Value> {
    targets
        .iter()
        .map(|target| {
            serde_json::json!({
                "path": target.path,
                "line": target.line,
                "character": target.character,
            })
        })
        .collect()
}

/// A location with its whole range kept, so containment can be tested.
struct Located {
    path: String,
    start_line: u32,
    start_character: u32,
    end_line: u32,
    end_character: u32,
}

impl Located {
    fn target(&self) -> Target {
        Target {
            path: self.path.clone(),
            line: self.start_line,
            character: self.start_character,
        }
    }

    /// Whether a caret is inside this range, ends included: a caret at
    /// the far end of a name is still on that name.
    fn contains(&self, path: &str, line: u32, character: u32) -> bool {
        if self.path != path {
            return false;
        }
        if line < self.start_line || line > self.end_line {
            return false;
        }
        if line == self.start_line && character < self.start_character {
            return false;
        }
        if line == self.end_line && character > self.end_character {
            return false;
        }
        true
    }
}

/// Every location in a `Location`, `Location[]` or `LocationLink[]`
/// result. Anything else, `null` included, yields none.
fn ranges(result: &str) -> Vec<Located> {
    let Ok(parsed) = serde_json::from_str::<Value>(result) else {
        return Vec::new();
    };
    match parsed {
        Value::Array(items) => items.iter().filter_map(located).collect(),
        Value::Object(_) => located(&parsed).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn located(value: &Value) -> Option<Located> {
    let uri = value
        .get("uri")
        .or_else(|| value.get("targetUri"))?
        .as_str()?;
    // `targetRange` covers the whole definition — signature, body and
    // all — which is the area the caret has not left yet.
    // `targetSelectionRange` is only the name.
    let range = value
        .get("range")
        .or_else(|| value.get("targetRange"))
        .or_else(|| value.get("targetSelectionRange"))?;
    let start = range.get("start")?;
    let end = range.get("end").unwrap_or(start);
    Some(Located {
        path: uri_path(uri)?,
        start_line: start.get("line")?.as_u64()? as u32,
        start_character: start.get("character")?.as_u64()? as u32,
        end_line: end.get("line").and_then(Value::as_u64).unwrap_or(0) as u32,
        end_character: end
            .get("character")
            .and_then(Value::as_u64)
            .unwrap_or(u32::MAX as u64) as u32,
    })
}

fn uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    // A host is never there in practice; a path always is.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    Some(percent_decoded(rest))
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(path: &str, line: u32, from: u32, to: u32) -> String {
        format!(
            r#"{{"uri": "file://{path}", "range": {{"start": {{"line": {line},
               "character": {from}}}, "end": {{"line": {line},
               "character": {to}}}}}}}"#
        )
    }

    #[test]
    fn a_definition_elsewhere_is_a_jump() {
        let result = location("/p/lib.rs", 40, 3, 9);
        assert_eq!(
            decide(&result, "/p/main.rs", 10, 4),
            Decision::Jump(Target {
                path: "/p/lib.rs".into(),
                line: 40,
                character: 3,
            })
        );
    }

    #[test]
    fn the_caret_inside_the_definition_asks_for_references() {
        let result = location("/p/lib.rs", 40, 3, 9);
        assert_eq!(decide(&result, "/p/lib.rs", 40, 5), Decision::AlreadyThere);
        // Both ends count: a caret after the last letter of a name is
        // on the name.
        assert_eq!(decide(&result, "/p/lib.rs", 40, 9), Decision::AlreadyThere);
        assert_eq!(decide(&result, "/p/lib.rs", 40, 3), Decision::AlreadyThere);
        // Just outside is not.
        assert!(matches!(
            decide(&result, "/p/lib.rs", 40, 10),
            Decision::Jump(_)
        ));
    }

    #[test]
    fn the_same_line_in_another_file_is_not_the_definition() {
        let result = location("/p/lib.rs", 40, 3, 9);
        assert!(matches!(
            decide(&result, "/p/other.rs", 40, 5),
            Decision::Jump(_)
        ));
    }

    #[test]
    fn nothing_comes_back_as_nothing() {
        assert_eq!(decide("null", "/p/lib.rs", 1, 1), Decision::Nothing);
        assert_eq!(decide("[]", "/p/lib.rs", 1, 1), Decision::Nothing);
        assert_eq!(decide("not json", "/p/lib.rs", 1, 1), Decision::Nothing);
    }

    #[test]
    fn several_definitions_are_offered_as_a_choice() {
        let result = format!(
            "[{}, {}]",
            location("/p/a.rs", 4, 0, 5),
            location("/p/b.rs", 9, 0, 5)
        );
        let Decision::Choose(targets) = decide(&result, "/p/main.rs", 1, 1) else {
            panic!("expected a choice");
        };
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].path, "/p/a.rs");
    }

    #[test]
    fn a_choice_that_includes_the_caret_drops_it() {
        // A declaration and its implementation, asked for from the
        // declaration: only the implementation is somewhere to go.
        let result = format!(
            "[{}, {}]",
            location("/p/a.rs", 4, 0, 5),
            location("/p/b.rs", 9, 0, 5)
        );
        assert_eq!(
            decide(&result, "/p/a.rs", 4, 2),
            Decision::Jump(Target {
                path: "/p/b.rs".into(),
                line: 9,
                character: 0,
            })
        );
    }

    #[test]
    fn location_links_carry_their_range_under_another_name() {
        let result = r#"[{"targetUri": "file:///p/lib.rs",
            "targetRange": {"start": {"line": 40, "character": 0},
                            "end": {"line": 48, "character": 1}},
            "targetSelectionRange": {"start": {"line": 40, "character": 3},
                                     "end": {"line": 40, "character": 9}}}]"#;
        // Anywhere in the body counts as being on the definition.
        assert_eq!(decide(result, "/p/lib.rs", 44, 8), Decision::AlreadyThere);
        assert!(matches!(
            decide(result, "/p/lib.rs", 49, 0),
            Decision::Jump(_)
        ));
    }

    #[test]
    fn a_path_with_spaces_survives_the_uri() {
        let result = r#"{"uri": "file:///p/my%20code/lib.rs",
            "range": {"start": {"line": 1, "character": 0},
                      "end": {"line": 1, "character": 4}}}"#;
        let Decision::Jump(target) = decide(result, "/p/main.rs", 0, 0) else {
            panic!("expected a jump");
        };
        assert_eq!(target.path, "/p/my code/lib.rs");
    }

    #[test]
    fn the_json_says_what_to_do() {
        let result = location("/p/lib.rs", 40, 3, 9);
        assert_eq!(
            to_json(&decide(&result, "/p/lib.rs", 40, 5)),
            r#"{"action":"references","targets":[]}"#
        );
        assert_eq!(
            to_json(&decide(&result, "/p/main.rs", 1, 1)),
            r#"{"action":"jump","targets":[{"character":3,"line":40,"path":"/p/lib.rs"}]}"#
        );
        assert_eq!(
            to_json(&decide("null", "/p/main.rs", 1, 1)),
            r#"{"action":"nothing","targets":[]}"#
        );
    }

    #[test]
    fn references_drop_the_one_under_the_caret() {
        let result = format!(
            "[{}, {}, {}]",
            location("/p/a.rs", 4, 0, 5),
            location("/p/b.rs", 9, 2, 7),
            location("/p/c.rs", 3, 1, 6)
        );
        let rest = elsewhere(&result, "/p/a.rs", 4, 1);
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0].path, "/p/b.rs");
    }
}
