# Markdown

Markdown is a first-class citizen: the same file gets tree-sitter
[highlighting](highlighting.md) in the editor — including fenced code
blocks colored in their own language — and a **live preview** beside it.

## The preview

Opening a Markdown document opens the preview pane automatically, to the
right of the source. **View → Toggle Markdown Preview** (⌥⌘P) hides and
shows it.

- The preview updates as you type, patched in place — no reload, no
  flicker, no lost scroll position.
- Scrolling either pane follows in the other.
- Rendering supports CommonMark plus tables, strikethrough, task lists,
  and footnotes, and its styles follow the system (or configured)
  appearance.

Rendering happens in the core (the shell only owns the pane), so the
exact same HTML will later feed other outputs.

## Hugo

Blog posts written for [Hugo](https://gohugo.io) are Markdown with two
additions, and Textchum reads both without Hugo being installed.

**Front matter** — TOML between `+++`, YAML between `---` — is
highlighted as the language it actually is, kept out of the prose the
spell checker reads (a slug is not a misspelling), and rendered in the
preview as a small metadata block rather than a paragraph of fences.

**Shortcodes** — `{{< figure src="…" >}}` and
`{{% notice %}}…{{% /notice %}}` — are highlighted as the calls they
are, skipped by the spell checker, and shown in the preview as a
labelled placeholder. They are never executed: running one needs
Hugo's template engine and your site's own layouts, so a placeholder
is the honest thing to show. The body of a paired `{{% … %}}` keeps
rendering as Markdown, which is what Hugo does with it too.

The **outline** (⇧⌘O) lists a post's headings even with no language
server running, nested by depth — a long article navigates like a
source file. Headings inside fenced code and front matter are not
mistaken for structure.

Finally, files under a `layouts/` directory are treated as **Go
templates** rather than plain HTML: the markup highlights as HTML and
the `{{ … }}` actions stand out from it.

JSON front matter (the brace form) is not recognized yet; TOML and
YAML cover what Hugo writes by default.

## Not there yet

- Precise scroll sync via source anchors (today's sync is proportional,
  which drifts on documents with very uneven block heights).
- Syntax colors inside preview code blocks (the editor colors them; the
  preview shows them plain).
- The hybrid/WYSIWYG editing mode — the preview pane is deliberately the
  first of the three Markdown tiers.
