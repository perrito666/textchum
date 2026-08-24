# Search

Two ways to find things beyond the current file, sharing one rule:
**the scope is a visible, editable path.** Both panels show exactly where
they look — the current document's project by default — and widening the
search is literally editing that path (up to `~` or `/` if you like).
Search never silently looks somewhere you did not expect.

Both walks are gitignore-aware, skip hidden files, and cap file sizes,
courtesy of ripgrep's own engine embedded in the core — not a subprocess.

## Open Quickly (⌘T)

Type fragments of a file name — `editwc` finds
`EditorWindowController.swift` — with fzf-style fuzzy matching and
ranking. ↑/↓ move the selection, ⏎ opens (fronting the window if the
file is already open), ⎋ closes. An empty query browses the scope
alphabetically.

## Find in Project (⇧⌘F)

The query is a regular expression; results stream in as
`path:line: text`. ⏎ jumps straight to the matching line. Results are
capped (200) to stay instant; refine the pattern rather than scrolling.

Case follows the **smart-case** rule ripgrep made familiar: an all
lowercase query matches any case, while a query containing an uppercase
letter is matched exactly. So `todo` finds `TODO`, and `TODO` finds
only `TODO`.

A line under the results says what the search did — "18 matches in 4
files · 812 searched", "No matches in 812 files searched", or the
reason nothing could be searched at all (a scope that does not exist,
one where everything is ignored, or a bad pattern, quoted). An empty
result is never mute, so a mistyped pattern or a wrong scope announces
itself instead of looking like an absence of matches.

## Stacked filters

Under the Find in Project query, **＋ Add Filter** stacks refinements:

- **line contains / line excludes** — the matching line's text;
- **file contains / file excludes** — the hit's file path.

Filters are case-insensitive substrings and combine with *and*: lines
with `foo` where `bar` also appears, but not in files with `test` in the
name, is the query `foo` plus `line contains bar` plus
`file excludes test`. File excludes prune whole files before they are
even opened, so filtered searches stay as fast as plain ones.

## Not there yet

- Replace across files.
- Explicit case/whole-word toggles in the panel (smart case covers the
  common case, and the pattern itself can express both).
- Persisted search history.
