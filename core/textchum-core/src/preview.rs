//! What the Markdown preview does with a link.
//!
//! The preview shows a document, and a click on a link would take the
//! pane somewhere else with no way back. Links go to the browser
//! instead — except the ones that point into the page already on
//! screen, which are a place to scroll to.

/// Whether `target` names a place in the page at `current`: the same
/// address but for the part after `#`.
///
/// Both shells resolve a bare `#anchor` against the page before asking,
/// so the comparison is between two whole addresses.
pub fn is_place_in_page(current: &str, target: &str) -> bool {
    if target.starts_with('#') {
        return true;
    }
    let address = |uri: &str| uri.split('#').next().unwrap_or("").to_string();
    !target.is_empty() && address(current) == address(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_anchor_is_a_place_in_the_page() {
        assert!(is_place_in_page("about:blank", "about:blank#notes"));
        assert!(is_place_in_page(
            "file:///tmp/readme.md",
            "file:///tmp/readme.md#install"
        ));
        assert!(is_place_in_page("about:blank", "#notes"));
    }

    #[test]
    fn another_document_is_not() {
        assert!(!is_place_in_page("about:blank", "https://example.com/"));
        assert!(!is_place_in_page(
            "file:///tmp/readme.md",
            "file:///tmp/other.md#install"
        ));
        assert!(!is_place_in_page("about:blank", ""));
    }
}
