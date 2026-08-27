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
///
/// Hugo's additions are rendered honestly rather than executed: front
/// matter becomes a small metadata header instead of a paragraph of
/// `+++`, and a shortcode becomes a labelled placeholder — running one
/// would need Hugo's template engine and the site's own layouts, which
/// the editor deliberately does not depend on.
pub fn to_html(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len() * 2);
    let body = match crate::hugo::front_matter(markdown) {
        Some(matter) => {
            out.push_str(&front_matter_html(&markdown[matter.body.clone()], matter.kind));
            &markdown[matter.range.end..]
        }
        None => markdown,
    };
    let body = placeholder_shortcodes(body);

    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let parser = Parser::new_ext(&body, options);
    html::push_html(&mut out, parser);
    out
}

/// Front matter as a definition list: the keys a post carries, shown
/// as data. The body is not parsed as TOML or YAML — one line, one
/// row, split at the first separator — because the preview only needs
/// to be readable, and a half-typed document must never break it.
fn front_matter_html(body: &str, kind: crate::hugo::FrontMatterKind) -> String {
    let mut out = String::from("<dl class=\"front-matter\" data-format=\"");
    out.push_str(kind.language());
    out.push_str("\">");
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let separator = match kind {
            crate::hugo::FrontMatterKind::Toml => '=',
            crate::hugo::FrontMatterKind::Yaml => ':',
        };
        let (key, value) = match line.split_once(separator) {
            Some((key, value)) => (key.trim(), value.trim()),
            // A list item or a continuation line: show it as a value
            // under the key above rather than inventing a key.
            None => ("", line),
        };
        out.push_str("<dt>");
        escape_into(&mut out, key);
        out.push_str("</dt><dd>");
        escape_into(&mut out, value.trim_matches('"'));
        out.push_str("</dd>");
    }
    out.push_str("</dl>");
    out
}

/// Replaces shortcode calls with an inline HTML placeholder naming the
/// call, so the preview shows what is there without pretending to run
/// it. Paired `{{% … %}}` bodies keep rendering as Markdown, which is
/// what Hugo does with them too.
fn placeholder_shortcodes(text: &str) -> String {
    let calls = crate::hugo::shortcodes(text);
    if calls.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for call in calls {
        out.push_str(&text[cursor..call.range.start]);
        if !call.closing {
            let inner = text[call.range.clone()]
                .trim_start_matches("{{<")
                .trim_start_matches("{{%")
                .trim_end_matches(">}}")
                .trim_end_matches("%}}")
                .trim();
            out.push_str("<span class=\"shortcode\" title=\"");
            escape_into(&mut out, inner);
            out.push_str("\">");
            escape_into(&mut out, if call.name.is_empty() { "shortcode" } else { &call.name });
            out.push_str("</span>");
        }
        cursor = call.range.end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn escape_into(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_becomes_a_header_not_prose() {
        let html = to_html("+++\ntitle = \"Harbor\"\ntags = [\"go\"]\n+++\n\n# Post\n");
        assert!(html.contains("class=\"front-matter\""), "{html}");
        assert!(html.contains("data-format=\"toml\""), "{html}");
        assert!(html.contains("<dt>title</dt><dd>Harbor</dd>"), "{html}");
        // The fences never reach the prose.
        assert!(!html.contains("+++"), "{html}");
        assert!(html.contains("<h1>Post</h1>"), "{html}");
    }

    #[test]
    fn shortcodes_render_as_placeholders_not_braces() {
        let html = to_html("Text {{< figure src=\"a.png\" >}} more.\n");
        assert!(html.contains("class=\"shortcode\""), "{html}");
        assert!(html.contains(">figure</span>"), "{html}");
        assert!(html.contains("title=\"figure src=&quot;a.png&quot;\""), "{html}");
        assert!(!html.contains("{{<"), "{html}");
    }

    #[test]
    fn paired_shortcode_bodies_still_render() {
        let html = to_html("{{% notice %}}\n**bold** body\n{{% /notice %}}\n");
        assert!(html.contains("<strong>bold</strong>"), "{html}");
        // The closing call leaves nothing behind.
        assert!(!html.contains("/notice"), "{html}");
    }

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
