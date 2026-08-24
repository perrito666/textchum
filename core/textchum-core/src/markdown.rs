//! Markdown → HTML for the live preview.
//!
//! The core renders the document body only; the shell owns the HTML shell
//! around it (styles, scripts, appearance). Rendering the whole document
//! per update is deliberate: pulldown-cmark chews through megabyte
//! documents in milliseconds, and the shell patches the result into the
//! existing page rather than reloading, so simplicity costs nothing
//! user-visible.

use pulldown_cmark::{html, Options, Parser};

/// Renders CommonMark (plus tables, strikethrough, task lists, and
/// footnotes) to an HTML fragment.
pub fn to_html(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let parser = Parser::new_ext(markdown, options);
    let mut out = String::with_capacity(markdown.len() * 2);
    html::push_html(&mut out, parser);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_blocks_and_extensions() {
        let html = to_html(
            "# Title\n\nsome *em* and `code`\n\n- [x] done\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        );
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<em>em</em>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("checkbox"));
        assert!(html.contains("<table>"));
    }

    #[test]
    fn escapes_raw_html_dangerous_content_stays_as_written() {
        // Raw HTML passes through (CommonMark semantics); script text is
        // the author's own document, rendered in a sandboxed preview.
        let html = to_html("hello <b>bold</b>\n");
        assert!(html.contains("<b>bold</b>"));
    }
}
