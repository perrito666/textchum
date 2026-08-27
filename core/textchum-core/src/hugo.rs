//! The parts of Hugo's Markdown that plain Markdown does not model.
//!
//! Two of them, and neither is a different language: **front matter**
//! (TOML between `+++`, YAML between `---`) opens a post with
//! structured data rather than prose, and **shortcodes**
//! (`{{< figure >}}`, `{{% notice %}}…{{% /notice %}}`) are template
//! calls sitting inside prose. The Markdown grammar already gives us
//! front matter as its own node — this module exists for the places
//! that need the *ranges* rather than the syntax tree: the spell
//! checker (which must not read a slug as a misspelling), the preview
//! (which must not render front matter as a paragraph of junk), and
//! shortcode highlighting, which no Markdown grammar provides.
//!
//! Nothing here executes a shortcode. Doing that would mean Hugo's
//! template engine and the site's own layouts; a labelled placeholder
//! is the honest thing to show instead.

/// Which syntax opened a document's front matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontMatterKind {
    /// `+++` fences, TOML inside.
    Toml,
    /// `---` fences, YAML inside.
    Yaml,
}

impl FrontMatterKind {
    /// The language name the body should be read as.
    pub fn language(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Yaml => "yaml",
        }
    }
}

/// A document's front matter: the whole block including its fences,
/// and the body between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontMatter {
    pub kind: FrontMatterKind,
    /// Byte range of the whole block, fences included.
    pub range: std::ops::Range<usize>,
    /// Byte range of the data between the fences.
    pub body: std::ops::Range<usize>,
}

/// The front matter opening `text`, if it has any. Hugo only honors it
/// at the very start of a file, so neither do we — a `+++` further
/// down is a thematic break, not metadata.
pub fn front_matter(text: &str) -> Option<FrontMatter> {
    let (kind, fence) = if text.starts_with("+++") {
        (FrontMatterKind::Toml, "+++")
    } else if text.starts_with("---") {
        (FrontMatterKind::Yaml, "---")
    } else {
        return None;
    };
    // The opening fence owns its whole line; anything else on it means
    // this is not front matter.
    let first_line_end = text.find('\n')?;
    if !text[fence.len()..first_line_end].trim().is_empty() {
        return None;
    }
    let body_start = first_line_end + 1;
    let mut offset = body_start;
    for line in text[body_start..].split_inclusive('\n') {
        if line.trim_end() == fence {
            let body_end = offset;
            let block_end = offset + line.len();
            return Some(FrontMatter {
                kind,
                range: 0..block_end,
                body: body_start..body_end,
            });
        }
        offset += line.len();
    }
    // An unterminated block is a document being typed, not front
    // matter yet.
    None
}

/// What a shortcode call looks like to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcode {
    /// The name being called (`figure`, `notice`, …); empty when the
    /// call is malformed enough to have none.
    pub name: String,
    /// Byte range of the whole call, delimiters included.
    pub range: std::ops::Range<usize>,
    /// True for `{{% … %}}`, whose body Hugo treats as Markdown.
    pub percent: bool,
    /// True for a closing call — `{{< /figure >}}`.
    pub closing: bool,
}

/// Every shortcode call in `text`, in order. The shape is
/// unambiguous — `{{<` … `>}}` or `{{%` … `%}}` — so this is a scan
/// rather than a parse, and an unterminated call is simply not one.
pub fn shortcodes(text: &str) -> Vec<Shortcode> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find("{{") {
        let start = index + offset;
        let after = start + 2;
        let (percent, close) = match bytes.get(after) {
            Some(b'<') => (false, ">}}"),
            Some(b'%') => (true, "%}}"),
            // `{{ .Title }}` is Go templating, not a shortcode; it
            // belongs to layouts, which are their own language.
            _ => {
                index = after;
                continue;
            }
        };
        let Some(end_offset) = text[after + 1..].find(close) else {
            break;
        };
        let end = after + 1 + end_offset + close.len();
        let inner = text[after + 1..end - close.len()].trim();
        let closing = inner.starts_with('/');
        let name = inner
            .trim_start_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned();
        found.push(Shortcode {
            name,
            range: start..end,
            percent,
            closing,
        });
        index = end;
    }
    found
}

/// Every Go template action in `text` — `{{ .Title }}`,
/// `{{- range .Pages }}`, `{{/* a comment */}}` — as byte ranges.
/// This is what a Hugo *layout* is made of, as opposed to the
/// shortcodes that appear in content.
pub fn template_actions(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut found = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find("{{") {
        let start = index + offset;
        let Some(end_offset) = text[start + 2..].find("}}") else {
            break;
        };
        let end = start + 2 + end_offset + 2;
        found.push(start..end);
        index = end;
    }
    found
}

/// One heading of a Markdown document, for the outline a post deserves
/// when no language server is answering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// 1 for `#`, 2 for `##`, …
    pub level: usize,
    pub text: String,
    /// Zero-based line, LSP style.
    pub line: usize,
    /// Zero-based UTF-16 column of the heading text.
    pub character: usize,
}

/// The ATX headings of a Markdown document, skipping front matter and
/// fenced code (`# not a heading` inside a fence is a comment).
pub fn headings(text: &str) -> Vec<Heading> {
    let skip_until = front_matter(text).map(|matter| matter.range.end).unwrap_or(0);
    let mut headings = Vec::new();
    let mut fence: Option<String> = None;
    let mut offset = 0;
    for (line_number, line) in text.split('\n').enumerate() {
        let line_start = offset;
        offset += line.len() + 1;
        if line_start < skip_until {
            continue;
        }
        let trimmed = line.trim_start();
        // Track fenced code so its contents never masquerade as
        // headings; a fence closes on its own marker.
        if let Some(open) = &fence {
            if trimmed.starts_with(open.as_str()) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(trimmed.chars().take(3).collect());
            continue;
        }
        if !trimmed.starts_with('#') {
            continue;
        }
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if level > 6 {
            continue;
        }
        let rest = &trimmed[level..];
        // `#hashtag` is not a heading; ATX needs a space.
        if !rest.starts_with(' ') && !rest.is_empty() {
            continue;
        }
        let title = rest.trim().trim_end_matches('#').trim();
        if title.is_empty() {
            continue;
        }
        let indent = line.len() - trimmed.len();
        headings.push(Heading {
            level,
            text: title.to_owned(),
            line: line_number,
            character: line[..indent].encode_utf16().count(),
        });
    }
    headings
}

/// The byte ranges a spell checker should skip in a Hugo document:
/// the front matter block and every shortcode call. Prose is
/// everything else.
pub fn non_prose_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    if let Some(matter) = front_matter(text) {
        ranges.push(matter.range);
    }
    ranges.extend(shortcodes(text).into_iter().map(|code| code.range));
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_and_yaml_front_matter() {
        let toml = "+++\ntitle = \"Harbor\"\n+++\n\nProse.\n";
        let matter = front_matter(toml).expect("toml front matter");
        assert_eq!(matter.kind, FrontMatterKind::Toml);
        assert_eq!(&toml[matter.range.clone()], "+++\ntitle = \"Harbor\"\n+++\n");
        assert_eq!(&toml[matter.body.clone()], "title = \"Harbor\"\n");

        let yaml = "---\ntitle: Harbor\n---\nProse.\n";
        let matter = front_matter(yaml).expect("yaml front matter");
        assert_eq!(matter.kind, FrontMatterKind::Yaml);
        assert_eq!(matter.kind.language(), "yaml");
        assert_eq!(&yaml[matter.body.clone()], "title: Harbor\n");
    }

    #[test]
    fn front_matter_only_opens_a_document() {
        // A thematic break further down is not metadata.
        assert!(front_matter("Prose.\n\n---\ntitle: no\n---\n").is_none());
        // An unterminated block is a document mid-typing.
        assert!(front_matter("+++\ntitle = \"x\"\n").is_none());
        // A fence sharing its line with anything else is not a fence.
        assert!(front_matter("--- not front matter\nx\n---\n").is_none());
    }

    #[test]
    fn shortcodes_are_scanned_with_their_shape() {
        let text = "a {{< figure src=\"x.png\" >}} b {{% notice warning %}}c{{% /notice %}}";
        let found = shortcodes(text);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].name, "figure");
        assert!(!found[0].percent && !found[0].closing);
        assert_eq!(&text[found[0].range.clone()], "{{< figure src=\"x.png\" >}}");
        assert_eq!(found[1].name, "notice");
        assert!(found[1].percent && !found[1].closing);
        assert_eq!(found[2].name, "notice");
        assert!(found[2].closing);
    }

    #[test]
    fn go_template_actions_are_not_shortcodes() {
        // `{{ .Title }}` belongs to layouts, a different language.
        assert!(shortcodes("{{ .Title }} and {{ range .Pages }}").is_empty());
        // An unterminated call is not a call.
        assert!(shortcodes("{{< figure src=").is_empty());
    }

    #[test]
    fn headings_skip_front_matter_and_fences() {
        let post = "+++\ntitle = \"t\"\n+++\n\n# One\n\n```sh\n# not a heading\n```\n\n## Two\n\n#hashtag\n";
        let found = headings(post);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].level, found[0].text.as_str()), (1, "One"));
        assert_eq!((found[1].level, found[1].text.as_str()), (2, "Two"));
        // Lines are zero-based and point at the real heading lines.
        assert_eq!(found[0].line, 4);
        assert_eq!(found[1].line, 10);
    }

    #[test]
    fn template_actions_cover_layout_syntax() {
        let layout = "<h1>{{ .Title }}</h1>\n{{- range .Pages }}\n{{/* note */}}\n";
        let actions = template_actions(layout);
        assert_eq!(actions.len(), 3);
        assert_eq!(&layout[actions[0].clone()], "{{ .Title }}");
        assert_eq!(&layout[actions[1].clone()], "{{- range .Pages }}");
        assert_eq!(&layout[actions[2].clone()], "{{/* note */}}");
        // An unterminated action is not one.
        assert!(template_actions("<p>{{ .Title").is_empty());
    }

    #[test]
    fn non_prose_covers_metadata_and_calls() {
        let post = "+++\nslug = \"harbr\"\n+++\n\nProse {{< figure >}} more.\n";
        let ranges = non_prose_ranges(post);
        assert_eq!(ranges.len(), 2);
        // The misspelled-looking slug is inside a skipped range.
        let slug = post.find("harbr").unwrap();
        assert!(ranges.iter().any(|range| range.contains(&slug)));
        let call = post.find("{{<").unwrap();
        assert!(ranges.iter().any(|range| range.contains(&call)));
    }
}
