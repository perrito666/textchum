//! Reading a `textDocument/codeAction` answer.
//!
//! A diagnostic that says what is wrong while the server is holding the
//! fix is the gap people notice first. The answer to that request is
//! also the most loosely specified thing in the protocol: an array
//! mixing two shapes, where one of them may arrive without the edit it
//! is about and has to be sent back to have it filled in.
//!
//! So the shell gets a list of titles and, for the one that is chosen,
//! what to do with it: apply this edit, run this command, or ask the
//! server to finish this action first.

use serde_json::Value;

/// One thing the server offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// What to call it on screen.
    pub title: String,
    /// The LSP kind (`quickfix`, `refactor.extract`, …), empty when the
    /// server did not say.
    pub kind: String,
    /// Whether the server marked it as the one to reach for.
    pub preferred: bool,
    /// The action as the server sent it, for resolving or running.
    pub raw: Value,
}

impl Action {
    /// What to do when this one is chosen.
    pub fn outcome(&self) -> Outcome {
        // A `Command` has a `command` string at the top level; a
        // `CodeAction` has a `title` and may carry either.
        if let Some(edit) = self.raw.get("edit").filter(|edit| edit.is_object()) {
            return Outcome::Edit(edit.clone());
        }
        if let Some(command) = self.raw.get("command") {
            // A CodeAction's `command` is an object; a Command's own is
            // a string, and its arguments sit beside it.
            if let Some(name) = command.as_str() {
                return Outcome::Command {
                    name: name.to_string(),
                    arguments: self.raw.get("arguments").cloned().unwrap_or(Value::Null),
                };
            }
            if let Some(name) = command.get("command").and_then(Value::as_str) {
                return Outcome::Command {
                    name: name.to_string(),
                    arguments: command.get("arguments").cloned().unwrap_or(Value::Null),
                };
            }
        }
        // Neither: the server answered cheaply and will fill in the
        // edit for the one actually chosen.
        Outcome::Resolve(self.raw.clone())
    }
}

/// What choosing an action means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Apply this `WorkspaceEdit`.
    Edit(Value),
    /// Ask the server to run this.
    Command { name: String, arguments: Value },
    /// Send this action back to have its edit filled in.
    Resolve(Value),
}

/// Every action in a `textDocument/codeAction` result.
///
/// Anything without a title is skipped: a row with no label is a row
/// nobody can choose on purpose.
pub fn actions(result: &str) -> Vec<Action> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(result) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            Some(Action {
                title,
                kind: item
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                preferred: item
                    .get("isPreferred")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                raw: item,
            })
        })
        .collect()
}

/// The findings at a caret, as the `context.diagnostics` of a code
/// action request.
///
/// `diagnostics` is what the server last published for the document.
/// Only the ones whose range covers the caret go in: a quick fix is
/// about the problem you are looking at, and sending the file's whole
/// list asks about all of them at once.
pub fn diagnostics_at(diagnostics: &str, line: u32, character: u32) -> Value {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(diagnostics) else {
        return Value::Array(Vec::new());
    };
    let covering: Vec<Value> = items
        .into_iter()
        .filter(|item| covers(item, line, character))
        .collect();
    Value::Array(covering)
}

fn covers(diagnostic: &Value, line: u32, character: u32) -> bool {
    let Some(range) = diagnostic.get("range") else {
        return false;
    };
    let number = |at: &str, field: &str| -> Option<u32> {
        range.get(at)?.get(field)?.as_u64().map(|value| value as u32)
    };
    let (Some(start_line), Some(start_character), Some(end_line), Some(end_character)) = (
        number("start", "line"),
        number("start", "character"),
        number("end", "line"),
        number("end", "character"),
    ) else {
        return false;
    };
    if line < start_line || line > end_line {
        return false;
    }
    if line == start_line && character < start_character {
        return false;
    }
    if line == end_line && character > end_character {
        return false;
    }
    true
}

/// The actions as JSON — `[{"title", "kind", "preferred"}, …]` — for
/// shells that reach the core through the C ABI. The action itself
/// stays here; the shell names the one it chose by its place in this
/// list.
pub fn to_json(actions: &[Action]) -> String {
    let items: Vec<Value> = actions
        .iter()
        .map(|action| {
            serde_json::json!({
                "title": action.title,
                "kind": action.kind,
                "preferred": action.preferred,
            })
        })
        .collect();
    Value::Array(items).to_string()
}

/// What choosing the action at `index` means, as JSON, for shells that
/// reach the core through the C ABI:
///
/// ```json
/// {"do": "edit", "edit": {…}}
/// {"do": "command", "name": "…", "arguments": […]}
/// {"do": "resolve", "action": {…}}
/// ```
///
/// `{"do": "nothing"}` for an index that names no action, so a stale
/// choice does nothing rather than something else.
pub fn outcome_json(result: &str, index: usize) -> String {
    let actions = actions(result);
    let Some(action) = actions.get(index) else {
        return serde_json::json!({"do": "nothing"}).to_string();
    };
    match action.outcome() {
        Outcome::Edit(edit) => serde_json::json!({"do": "edit", "edit": edit}),
        Outcome::Command { name, arguments } => {
            serde_json::json!({"do": "command", "name": name, "arguments": arguments})
        }
        Outcome::Resolve(action) => serde_json::json!({"do": "resolve", "action": action}),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUICK_FIX: &str = r#"[
        {"title": "Import `HashMap`", "kind": "quickfix", "isPreferred": true,
         "edit": {"changes": {"file:///p/a.rs": []}}},
        {"title": "Extract into function", "kind": "refactor.extract"},
        {"title": "Organize imports", "command": "rust-analyzer.organizeImports",
         "arguments": ["file:///p/a.rs"]}
    ]"#;

    #[test]
    fn every_offer_with_a_title_is_listed() {
        let actions = actions(QUICK_FIX);
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].title, "Import `HashMap`");
        assert_eq!(actions[0].kind, "quickfix");
        assert!(actions[0].preferred);
        assert!(!actions[1].preferred);
    }

    #[test]
    fn an_action_says_what_choosing_it_means() {
        let actions = actions(QUICK_FIX);
        assert!(matches!(actions[0].outcome(), Outcome::Edit(_)));
        // No edit and no command: the server will fill it in.
        assert!(matches!(actions[1].outcome(), Outcome::Resolve(_)));
        let Outcome::Command { name, arguments } = actions[2].outcome() else {
            panic!("expected a command");
        };
        assert_eq!(name, "rust-analyzer.organizeImports");
        assert_eq!(arguments[0], "file:///p/a.rs");
    }

    #[test]
    fn a_code_action_carrying_a_command_object_is_read_the_same_way() {
        let result = r#"[{"title": "Fix all", "command":
            {"title": "Fix all", "command": "eslint.applyAllFixes",
             "arguments": [{"uri": "file:///p/a.js"}]}}]"#;
        let Outcome::Command { name, arguments } = actions(result)[0].outcome() else {
            panic!("expected a command");
        };
        assert_eq!(name, "eslint.applyAllFixes");
        assert_eq!(arguments[0]["uri"], "file:///p/a.js");
    }

    #[test]
    fn nothing_usable_is_no_actions_rather_than_a_panic() {
        assert!(actions("null").is_empty());
        assert!(actions("not json").is_empty());
        assert!(actions(r#"[{"kind": "quickfix"}]"#).is_empty());
        assert!(actions(r#"[{"title": "   "}]"#).is_empty());
    }

    #[test]
    fn only_the_findings_under_the_caret_are_asked_about() {
        let diagnostics = r#"[
            {"message": "here", "range": {"start": {"line": 4, "character": 2},
                                          "end": {"line": 4, "character": 9}}},
            {"message": "elsewhere", "range": {"start": {"line": 40, "character": 0},
                                               "end": {"line": 40, "character": 5}}}
        ]"#;
        let at = diagnostics_at(diagnostics, 4, 5);
        assert_eq!(at.as_array().map(Vec::len), Some(1));
        assert_eq!(at[0]["message"], "here");
        // Both ends count: a caret at the far end of a mark is on it.
        assert_eq!(diagnostics_at(diagnostics, 4, 9).as_array().map(Vec::len), Some(1));
        assert_eq!(diagnostics_at(diagnostics, 4, 10).as_array().map(Vec::len), Some(0));
        assert_eq!(diagnostics_at("null", 4, 5).as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn the_chosen_action_says_what_to_do_across_the_abi() {
        // Keys come out sorted, so the shape is checked by content.
        assert!(outcome_json(QUICK_FIX, 0).contains(r#""do":"edit""#));
        assert!(outcome_json(QUICK_FIX, 1).contains(r#""do":"resolve""#));
        assert!(outcome_json(QUICK_FIX, 2).contains(r#""name":"rust-analyzer.organizeImports""#));
        // An index that names nothing does nothing.
        assert_eq!(outcome_json(QUICK_FIX, 99), r#"{"do":"nothing"}"#);
    }

    #[test]
    fn the_json_carries_what_the_list_shows() {
        assert_eq!(
            to_json(&actions(r#"[{"title": "Fix", "kind": "quickfix", "isPreferred": true}]"#)),
            r#"[{"kind":"quickfix","preferred":true,"title":"Fix"}]"#
        );
    }
}
