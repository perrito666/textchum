//! The signature of the call being typed.
//!
//! A server answers `textDocument/signatureHelp` with every overload it
//! knows, which one it thinks is meant, and which parameter the caret
//! is in. What a balloon over the caret wants is one line and a
//! stretch of it to set in bold. The reduction lives here so both
//! shells show the same line, and so the two ways a server may name a
//! parameter — its text, or a pair of offsets into the label — are
//! told apart once.

use serde_json::Value;

/// One signature, reduced to what is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub label: String,
    /// The active parameter's stretch of `label`, in UTF-16 units.
    pub parameter: Option<(usize, usize)>,
    pub documentation: String,
}

/// The signature a `signatureHelp` result means, or `None` when the
/// server had nothing (a `null` result, or no signatures).
pub fn active(result_json: &str) -> Option<Signature> {
    let result: Value = serde_json::from_str(result_json).ok()?;
    let signatures = result.get("signatures")?.as_array()?;
    let chosen = result["activeSignature"].as_u64().unwrap_or(0) as usize;
    let signature = signatures.get(chosen).or_else(|| signatures.first())?;
    let label = signature["label"].as_str()?.to_owned();
    // The signature's own answer wins over the result's: a server that
    // gives both means the former for this overload.
    let index = signature["activeParameter"]
        .as_u64()
        .or_else(|| result["activeParameter"].as_u64())
        .unwrap_or(0) as usize;
    let parameter = signature["parameters"]
        .as_array()
        .and_then(|parameters| parameters.get(index))
        .and_then(|parameter| stretch_of(&label, &parameter["label"]));
    let documentation = match &signature["documentation"] {
        Value::String(text) => text.clone(),
        other => other["value"].as_str().unwrap_or_default().to_owned(),
    };
    Some(Signature { label, parameter, documentation })
}

/// Where a parameter sits in its signature's label: the offsets the
/// server gave, or the place its text is found.
fn stretch_of(label: &str, parameter: &Value) -> Option<(usize, usize)> {
    match parameter {
        Value::Array(pair) => {
            let start = pair.first()?.as_u64()? as usize;
            let end = pair.get(1)?.as_u64()? as usize;
            (start <= end && end <= label.encode_utf16().count()).then_some((start, end))
        }
        Value::String(text) if !text.is_empty() => {
            let byte = label.find(text.as_str())?;
            let start = label[..byte].encode_utf16().count();
            Some((start, start + text.encode_utf16().count()))
        }
        _ => None,
    }
}

/// [`active`] as JSON for a shell across the C boundary: `{label,
/// start, end, documentation}`, `start` and `end` null when no
/// parameter is marked.
pub fn active_json(result_json: &str) -> Option<String> {
    let signature = active(result_json)?;
    Some(
        serde_json::json!({
            "label": signature.label,
            "start": signature.parameter.map(|(start, _)| start),
            "end": signature.parameter.map(|(_, end)| end),
            "documentation": signature.documentation,
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_active_parameter_is_found_by_offsets_or_by_text() {
        let by_offsets = r#"{"signatures": [{"label": "frob(x: int, y: str)",
            "parameters": [{"label": [5, 11]}, {"label": [13, 19]}]}],
            "activeSignature": 0, "activeParameter": 1}"#;
        let signature = active(by_offsets).unwrap();
        assert_eq!(signature.label, "frob(x: int, y: str)");
        assert_eq!(signature.parameter, Some((13, 19)));

        let by_text = r#"{"signatures": [{"label": "frob(x: int, y: str)",
            "parameters": [{"label": "x: int"}, {"label": "y: str"}],
            "documentation": {"kind": "markdown", "value": "Frobs."}}],
            "activeParameter": 0}"#;
        let signature = active(by_text).unwrap();
        assert_eq!(signature.parameter, Some((5, 11)));
        assert_eq!(signature.documentation, "Frobs.");
    }

    #[test]
    fn the_signatures_own_parameter_wins_and_the_chosen_overload_is_shown() {
        let json = r#"{"signatures": [
            {"label": "a()", "parameters": []},
            {"label": "a(first, second)", "activeParameter": 1,
             "parameters": [{"label": "first"}, {"label": "second"}]}],
            "activeSignature": 1, "activeParameter": 0}"#;
        let signature = active(json).unwrap();
        assert_eq!(signature.label, "a(first, second)");
        assert_eq!(signature.parameter, Some((9, 15)));
    }

    #[test]
    fn offsets_count_utf16_units() {
        let json = r#"{"signatures": [{"label": "f(é: 𝔘, b)",
            "parameters": [{"label": "é: 𝔘"}, {"label": "b"}]}], "activeParameter": 1}"#;
        // "f(" is 2 units, "é: 𝔘" is 1 + 1 + 1 + 2, ", " is 2.
        assert_eq!(active(json).unwrap().parameter, Some((9, 10)));
    }

    #[test]
    fn nothing_to_show_is_none_and_a_bad_stretch_is_left_unmarked() {
        assert!(active("null").is_none());
        assert!(active(r#"{"signatures": []}"#).is_none());
        let past = r#"{"signatures": [{"label": "f(a)", "parameters": [{"label": [2, 40]}]}]}"#;
        assert_eq!(active(past).unwrap().parameter, None);
        assert!(active_json("null").is_none());
        let json: Value = serde_json::from_str(&active_json(past).unwrap()).unwrap();
        assert!(json["start"].is_null());
    }
}
