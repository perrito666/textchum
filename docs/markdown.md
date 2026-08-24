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

## Not there yet

- Precise scroll sync via source anchors (today's sync is proportional,
  which drifts on documents with very uneven block heights).
- Syntax colors inside preview code blocks (the editor colors them; the
  preview shows them plain).
- The hybrid/WYSIWYG editing mode — the preview pane is deliberately the
  first of the three Markdown tiers.
