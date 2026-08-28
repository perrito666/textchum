# Syntax highlighting

Textchum highlights code with [tree-sitter](https://tree-sitter.github.io):
every document with a recognized language keeps a real parse tree, updated
incrementally on each edit, and coloring is computed from that tree — not
from regular expressions.

## Languages

Detection is by file extension — or by exact file name, for the files
whose identity *is* their name: `Makefile` (and `*.mk`), and git's
message files (`COMMIT_EDITMSG`, `MERGE_MSG`, `TAG_EDITMSG`), so commit
messages written through `chum --wait` arrive colored. It runs on open
and on the first save of an untitled document. Currently recognized:
Rust, Python, Go, C, JavaScript, JSON, Bash, Make, git commit messages,
HTML, CSS, TOML, YAML, Swift, Zig, and Markdown. The window subtitle
shows the active language; unrecognized files simply stay plain text.
The navigator's file rows carry the type's own Finder icon when macOS
genuinely differentiates it — and a small badge in the language's
conventional color otherwise. The distinction matters: a default
handler (an IDE, say) stamps its *own* document icon on every type it
claims, identical everywhere, so an icon shared across types counts as
generic and the badge wins.

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

Colouring follows the viewport: the visible stretch plus a generous
margin is what gets queried and painted, repainted as you scroll. A
megabyte file is coloured as cheaply as a small one, and there is no
size past which colour silently stops — only the core's parse ceiling,
beyond which a document is plain text by design.

A theme's **bold and italic** are honoured too. Colour rides TextKit's
rendering attributes, which never disturb layout; the typographic
traits are applied as fonts, which is why they are painted for the
visible stretch rather than the whole document. Monospaced faces keep
their advance width across weights, so nothing reflows.

## Not there yet

- Queries limited to the visible stretch for documents of several
  hundred kilobytes: today they are coloured whole, or past a ceiling
  not at all.
