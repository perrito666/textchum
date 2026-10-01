//! Who calls a function: the call hierarchy, as a list of places.
//!
//! The protocol answers in two steps. `prepareCallHierarchy` turns a
//! position into the item it names, and `incomingCalls` takes that item
//! back and lists the functions that call it, each with the ranges of
//! its call sites. Both shells already have a list that shows places —
//! the one Find References fills — so the second answer is reduced here
//! to the shape that list takes, an LSP `Location[]`, and neither shell
//! learns the hierarchy's own vocabulary.

use serde_json::Value;

/// The item a `prepareCallHierarchy` result names: the first, when a
/// server offers several. `None` when the position is not on anything
/// callable.
pub fn prepared_item(result_json: &str) -> Option<Value> {
    let result: Value = serde_json::from_str(result_json).ok()?;
    result.as_array()?.first().cloned()
}

/// An `incomingCalls` result as a `Location[]`: one place per call
/// site, in the caller's file. A caller the server gave no call sites
/// for is listed at its own name.
pub fn callers_as_locations(result_json: &str) -> String {
    let Ok(Value::Array(calls)) = serde_json::from_str::<Value>(result_json) else {
        return "[]".into();
    };
    let mut places = Vec::new();
    for call in &calls {
        let from = &call["from"];
        let Some(uri) = from["uri"].as_str() else {
            continue;
        };
        let sites = call["fromRanges"].as_array().filter(|sites| !sites.is_empty());
        match sites {
            Some(sites) => {
                for site in sites {
                    places.push(serde_json::json!({"uri": uri, "range": site}));
                }
            }
            None => places.push(serde_json::json!({
                "uri": uri,
                "range": if from["selectionRange"].is_object() {
                    from["selectionRange"].clone()
                } else {
                    from["range"].clone()
                },
            })),
        }
    }
    Value::Array(places).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_prepared_item_is_the_one_asked_about() {
        let item = prepared_item(r#"[{"name": "broadcast", "kind": 6}, {"name": "other"}]"#);
        assert_eq!(item.unwrap()["name"], "broadcast");
        assert!(prepared_item("null").is_none());
        assert!(prepared_item("[]").is_none());
    }

    #[test]
    fn every_call_site_is_a_place_and_a_caller_without_sites_is_its_name() {
        let json = r#"[
            {"from": {"name": "main", "uri": "file:///p/main.rs",
                      "range": {"start": {"line": 0, "character": 0}, "end": {"line": 9, "character": 1}},
                      "selectionRange": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}},
             "fromRanges": [
                {"start": {"line": 2, "character": 4}, "end": {"line": 2, "character": 13}},
                {"start": {"line": 5, "character": 4}, "end": {"line": 5, "character": 13}}]},
            {"from": {"name": "test", "uri": "file:///p/tests.rs",
                      "selectionRange": {"start": {"line": 7, "character": 3}, "end": {"line": 7, "character": 7}}},
             "fromRanges": []}
        ]"#;
        let places: Value = serde_json::from_str(&callers_as_locations(json)).unwrap();
        let places = places.as_array().unwrap();
        assert_eq!(places.len(), 3);
        assert_eq!(places[1]["uri"], "file:///p/main.rs");
        assert_eq!(places[1]["range"]["start"]["line"], 5);
        assert_eq!(places[2]["uri"], "file:///p/tests.rs");
        assert_eq!(places[2]["range"]["start"]["line"], 7);
        assert_eq!(callers_as_locations("null"), "[]");
    }
}
