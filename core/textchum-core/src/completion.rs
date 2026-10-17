//! Completion: what a server offers, and how it is put in order against
//! what has been typed.
//!
//! A server sends its candidates in an order of its own and leaves the
//! rest to the client. Both shells filter and order them here, so a
//! completion list looks the same on either, and the order is the one
//! INTERACTION.md gives (12.3): how well an item matches what was
//! typed, then whether it was chosen before, then the server's order.

use serde_json::Value;

/// One candidate, as far as ranking and showing it need.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    /// The type or signature, as the server writes it.
    pub detail: String,
    /// `labelDetails.detail`: what follows the name on its line, such
    /// as a function's parameters.
    pub label_detail: String,
    /// `labelDetails.description`: where it comes from — for an item
    /// that brings an import, the path it imports.
    pub description: String,
    /// Markdown, or plain text; empty when the server left it for
    /// `completionItem/resolve`.
    pub documentation: String,
    pub sort_text: String,
    pub filter_text: String,
    pub preselect: bool,
}

/// The items of a `textDocument/completion` result — a list, or a
/// `CompletionList` — in the order the server sent them, leaving out
/// any without a label, as both shells' own readers do, so an index
/// here is an index there.
pub fn items(result: &str) -> Vec<Item> {
    let Ok(parsed) = serde_json::from_str::<Value>(result) else { return Vec::new() };
    let list = match &parsed {
        Value::Array(items) => items.as_slice(),
        Value::Object(_) => parsed["items"].as_array().map(Vec::as_slice).unwrap_or(&[]),
        _ => &[],
    };
    list.iter().filter_map(item).collect()
}

fn item(raw: &Value) -> Option<Item> {
    let label = raw["label"].as_str()?.to_owned();
    Some(Item {
        detail: raw["detail"].as_str().unwrap_or_default().to_owned(),
        label_detail: raw["labelDetails"]["detail"].as_str().unwrap_or_default().to_owned(),
        description: raw["labelDetails"]["description"].as_str().unwrap_or_default().to_owned(),
        documentation: documentation_of(raw),
        sort_text: raw["sortText"].as_str().unwrap_or(&label).to_owned(),
        filter_text: raw["filterText"].as_str().unwrap_or(&label).to_owned(),
        preselect: raw["preselect"].as_bool().unwrap_or(false),
        label,
    })
}

/// An item's documentation, string or `MarkupContent`; empty when it
/// has none. For a `completionItem/resolve` answer as well.
pub fn documentation_of(raw: &Value) -> String {
    match &raw["documentation"] {
        Value::String(text) => text.clone(),
        Value::Object(markup) => markup.get("value").and_then(Value::as_str).unwrap_or_default().to_owned(),
        _ => String::new(),
    }
}

/// How well `candidate` matches `typed`: lower is better, `None` is no
/// match. Exact, then the start of the name (the case as typed first),
/// then the starts of its words — `rd` for `read_dir` or `ReadDir` —
/// then anywhere in it, then its letters in order.
pub fn match_quality(candidate: &str, typed: &str) -> Option<u8> {
    if typed.is_empty() {
        return Some(0);
    }
    let lower = candidate.to_lowercase();
    let wanted = typed.to_lowercase();
    if candidate == typed {
        return Some(0);
    }
    if lower == wanted {
        return Some(1);
    }
    if candidate.starts_with(typed) {
        return Some(2);
    }
    if lower.starts_with(&wanted) {
        return Some(3);
    }
    if starts_of_words(candidate).starts_with(&wanted) {
        return Some(4);
    }
    if lower.contains(&wanted) {
        return Some(5);
    }
    let mut letters = wanted.chars().peekable();
    for character in lower.chars() {
        if letters.peek() == Some(&character) {
            letters.next();
        }
    }
    letters.peek().is_none().then_some(6)
}

/// The first letter of each word of a name, lowercased: `read_dir` and
/// `ReadDir` are both `rd`, `HTTPServer` is `hs`.
fn starts_of_words(name: &str) -> String {
    let characters: Vec<char> = name.chars().collect();
    let mut starts = String::new();
    for (at, character) in characters.iter().enumerate() {
        if !character.is_alphanumeric() {
            continue;
        }
        let previous = at.checked_sub(1).map(|before| characters[before]);
        let next = characters.get(at + 1);
        let starts_word = match previous {
            None => true,
            Some(before) if !before.is_alphanumeric() => true,
            Some(before) if character.is_uppercase() && before.is_lowercase() => true,
            // The last capital of a run before a lowercase letter starts
            // a word: the `S` of `HTTPServer`.
            Some(before) if character.is_uppercase() && before.is_uppercase() => {
                next.is_some_and(|next| next.is_lowercase())
            }
            _ => false,
        };
        if starts_word {
            starts.extend(character.to_lowercase());
        }
    }
    starts
}

/// The items that match `typed`, best first, as indices into `items`:
/// by how well they match, then by how lately one with that label was
/// chosen (`recent`, most recent first), then by the server's order —
/// its preselected item first, then its sort text, then the order it
/// sent them in.
pub fn rank(items: &[Item], typed: &str, recent: &[String]) -> Vec<usize> {
    let mut ranked: Vec<(u8, usize, bool, &str, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(at, item)| {
            let quality = match_quality(&item.filter_text, typed)?;
            let chosen = recent.iter().position(|label| *label == item.label).unwrap_or(usize::MAX);
            Some((quality, chosen, !item.preselect, item.sort_text.as_str(), at))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().map(|(.., at)| at).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(label: &str, sort: &str) -> Item {
        Item {
            label: label.into(),
            detail: String::new(),
            label_detail: String::new(),
            description: String::new(),
            documentation: String::new(),
            sort_text: sort.into(),
            filter_text: label.into(),
            preselect: false,
        }
    }

    #[test]
    fn a_match_is_better_the_more_of_the_name_it_starts() {
        assert_eq!(match_quality("iter", "iter"), Some(0));
        assert_eq!(match_quality("Iter", "iter"), Some(1));
        assert_eq!(match_quality("iter_mut", "iter"), Some(2));
        assert_eq!(match_quality("IterMut", "iter"), Some(3));
        assert_eq!(match_quality("read_dir", "rd"), Some(4));
        assert_eq!(match_quality("ReadDir", "rd"), Some(4));
        assert_eq!(match_quality("HTTPServer", "hs"), Some(4));
        assert_eq!(match_quality("into_iter", "iter"), Some(5));
        assert_eq!(match_quality("swap_remove", "srm"), Some(6));
        assert_eq!(match_quality("len", "x"), None);
    }

    #[test]
    fn ranked_by_match_then_by_what_was_chosen_then_by_the_server() {
        let items = vec![
            named("into_iter", "0"),
            named("iter_mut", "1"),
            named("iter", "2"),
            named("len", "3"),
            named("is_empty", "4"),
        ];
        // `iter` is exact; `iter_mut` starts with it; `into_iter` only
        // has it inside; `len` does not match at all.
        assert_eq!(rank(&items, "iter", &[]), vec![2, 1, 0]);
        // Nothing typed: the server's order, until something is chosen.
        assert_eq!(rank(&items, "", &[]), vec![0, 1, 2, 3, 4]);
        let recent = vec!["len".to_owned(), "iter".to_owned()];
        assert_eq!(rank(&items, "", &recent), vec![3, 2, 0, 1, 4]);
        // Chosen before beats the server, never a better match.
        assert_eq!(rank(&items, "i", &["is_empty".to_owned()]), vec![4, 0, 1, 2]);
    }

    #[test]
    fn a_result_is_read_with_its_details_and_documentation() {
        let read = items(
            r#"{"isIncomplete": false, "items": [
                {"label": "HashMap", "detail": "struct", "sortText": "b",
                 "labelDetails": {"description": "std::collections::HashMap"},
                 "documentation": {"kind": "markdown", "value": "A **hash** map."}},
                {"detail": "no label, left out"},
                {"label": "len", "labelDetails": {"detail": "(&self)"}, "documentation": "plain",
                 "preselect": true}
            ]}"#,
        );
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].description, "std::collections::HashMap");
        assert_eq!(read[0].documentation, "A **hash** map.");
        assert_eq!(read[1].label_detail, "(&self)");
        assert_eq!(read[1].documentation, "plain");
        assert_eq!(read[1].sort_text, "len", "no sort text sorts by the label");
        assert_eq!(rank(&read, "", &[]), vec![1, 0], "the server's preselected item leads");
    }
}
