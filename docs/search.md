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

## Not there yet

- Replace across files.
- Case/whole-word toggles in the panel (the pattern itself can express
  both).
- Persisted search history.
