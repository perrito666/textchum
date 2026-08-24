# Syntax highlighting

Textchum highlights code with [tree-sitter](https://tree-sitter.github.io):
every document with a recognized language keeps a real parse tree, updated
incrementally on each edit, and coloring is computed from that tree — not
from regular expressions.

## Languages

Detection is by file extension, on open and on the first save of an
untitled document. Currently recognized: Rust, Python, Go, C, JavaScript,
JSON, Bash, HTML, CSS, TOML, YAML, Swift, Zig, and Markdown. The window
subtitle shows the active language; unrecognized files simply stay plain
text.

Grammars are compiled into the application, so highlighting works offline
and identically everywhere.

## Injections

Documents that embed other languages highlight the embedded content with
the embedded language's grammar:

- Markdown fenced code blocks are colored by the language named on the
  fence (` ```rust ` and friends), and Markdown's own emphasis, links, and
  inline code come from a dedicated inline grammar.
- HTML `<script>` and `<style>` elements color as JavaScript and CSS.

## How it works

The division of labor follows the project's architecture rule:

- The **core** owns the parse. Each edit feeds the tree an exact
  description of what changed, and tree-sitter re-parses incrementally —
  keystroke-scale work no matter the file size. On request it runs the
  language's highlight query over a range and answers with *styled spans*:
  ranges plus indices into a style table.
- The **shell** owns the pixels. Spans are painted as TextKit rendering
  attributes — a color-only overlay that cannot invalidate text layout,
  so coloring never competes with typing.

The style table carries a color per system appearance, so switching
between light and dark mode recolors instantly, with palettes tuned for
each.

Very large documents (beyond a few megabytes) deliberately skip
highlighting; the editor itself stays fast at any size.

The palette itself is a theme — seven ship built in, and user themes
are JSON files; see [themes in the configuration](configuration.md).

Should a coloring artifact ever survive an edit, **View → Redraw**
(⌥⌘L, rebindable as `redraw`) rebuilds every visual layer from scratch:
base attributes, syntax colors, diagnostic marks, and the gutter.

## Not there yet

- Bold/italic style nuances — the overlay is currently color only.
- Manual language selection from the UI for files with odd extensions.
- Viewport-scoped queries for documents in the hundreds of kilobytes
  (currently they are colored whole or, past a cap, not at all).
