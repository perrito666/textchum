//! Markdown → HTML for the live preview.
//!
//! The core renders the document body only; the shell owns the HTML shell
//! around it (styles, scripts, appearance). Rendering the whole document
//! per update is deliberate: pulldown-cmark chews through megabyte
//! documents in milliseconds, and the shell patches the result into the
//! existing page rather than reloading, so simplicity costs nothing
//! user-visible.

use pulldown_cmark::{html, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

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
    // The highlight palette rides with the document: both shells and a
    // printed PDF style the fences without knowing how.
    out.push_str(&highlight_css());
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
    // Fenced code whose language the registry knows is coloured here,
    // by the same grammars the editor colours with — both shells and
    // the saved PDF get it for free. Everything else passes through.
    let mut events: Vec<Event> = Vec::new();
    let mut fence: Option<(String, String)> = None;
    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                let language = info
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                fence = Some((language, String::new()));
            }
            Event::Text(text) if fence.is_some() => {
                if let Some((_, code)) = fence.as_mut() {
                    code.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) if fence.is_some() => {
                let (language, code) = fence.take().expect("a fence is open");
                events.push(Event::Html(fence_html(&language, &code).into()));
            }
            other => events.push(other),
        }
    }
    html::push_html(&mut out, events.into_iter());
    out
}

/// One fenced block as HTML: highlighted when the language is known,
/// plain monospace when it is not — an unknown fence is not an error.
fn fence_html(language: &str, code: &str) -> String {
    let mut out = String::from("<pre><code>");
    match highlight_spans(language, code) {
        Some(styles) => {
            let mut current: Option<u32> = None;
            let mut utf16 = 0usize;
            for character in code.chars() {
                let style = styles.get(utf16).copied().flatten();
                if style != current {
                    if current.is_some() {
                        out.push_str("</span>");
                    }
                    if let Some(style) = style {
                        out.push_str(&format!("<span class=\"hl-{style}\">"));
                    }
                    current = style;
                }
                push_escaped(&mut out, character);
                utf16 += character.len_utf16();
            }
            if current.is_some() {
                out.push_str("</span>");
            }
        }
        None => escape_into(&mut out, code),
    }
    out.push_str("</code></pre>");
    out
}

/// The style id under each UTF-16 unit of `code`, or `None` where the
/// grammar says nothing. `None` altogether when the language is not
/// known here.
fn highlight_spans(language: &str, code: &str) -> Option<Vec<Option<u32>>> {
    if language.is_empty() {
        return None;
    }
    let spec = crate::syntax::languages::by_name(language)?;
    let rope = ropey::Rope::from_str(code);
    let syntax = crate::syntax::SyntaxState::new(spec, &rope)?;
    let spans = syntax.highlights(&rope, 0..code.len());
    let mut styles: Vec<Option<u32>> = vec![None; code.encode_utf16().count()];
    // Application order: a later span wins where they overlap, the
    // same contract the editors paint by.
    let length = styles.len();
    for span in spans {
        for slot in styles
            .iter_mut()
            .take(span.end_utf16.min(length))
            .skip(span.start_utf16)
        {
            *slot = Some(span.style);
        }
    }
    Some(styles)
}

fn push_escaped(out: &mut String, character: char) {
    match character {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        other => out.push(other),
    }
}

/// The palette for the `hl-` classes as CSS: the light colours, the
/// dark ones under the dark scheme, and the light ones again for
/// print — a PDF is paper, whatever the window looks like.
pub fn highlight_css() -> String {
    let styles = crate::syntax::theme::styles();
    let mut css = String::from("<style>");
    let rgb = |color: u32| format!("#{:06x}", color >> 8);
    for (index, style) in styles.iter().enumerate() {
        css.push_str(&format!(".hl-{index}{{color:{}", rgb(style.light)));
        if style.flags & crate::syntax::theme::STYLE_BOLD != 0 {
            css.push_str(";font-weight:600");
        }
        if style.flags & crate::syntax::theme::STYLE_ITALIC != 0 {
            css.push_str(";font-style:italic");
        }
        css.push('}');
    }
    css.push_str("@media (prefers-color-scheme: dark){");
    for (index, style) in styles.iter().enumerate() {
        css.push_str(&format!(".hl-{index}{{color:{}}}", rgb(style.dark)));
    }
    css.push('}');
    css.push_str("@media print{:root{color-scheme:light}");
    for (index, style) in styles.iter().enumerate() {
        css.push_str(&format!(".hl-{index}{{color:{}}}", rgb(style.light)));
    }
    css.push('}');
    css.push_str("</style>");
    css
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
    fn a_known_fence_is_highlighted_and_an_unknown_one_is_left_plain() {
        let html = to_html("```rust\nfn main() {}\n```\n");
        assert!(html.contains("class=\"hl-"), "{html}");
        assert!(html.contains("main"), "{html}");
        let plain = to_html("```nosuchtongue\nfn main() {}\n```\n");
        assert!(!plain.contains("class=\"hl-"), "{plain}");
        assert!(plain.contains("fn main"), "{plain}");
        // The palette rides along: light rules, a dark block, and a
        // print block that goes back to light — a PDF is paper.
        assert!(html.contains("prefers-color-scheme: dark"), "{html}");
        assert!(html.contains("@media print"), "{html}");
    }

    #[test]
    fn fence_code_is_escaped_even_when_highlighted() {
        let html = to_html("```rust\nlet a = b < c && d > e;\n```\n");
        assert!(html.contains("&lt;"), "{html}");
        assert!(html.contains("&amp;&amp;"), "{html}");
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
