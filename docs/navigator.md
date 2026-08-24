# The navigator

Every editor window carries a navigation drawer on its left (toggle with
**⌘0**, or View → Toggle Navigator). It has two stacked panes.

## Open buffers, grouped by project

The top pane lists the open documents of **this window's tab group** —
files opened as tabs share one list, while separate windows keep separate
worlds (whether files open as tabs or windows is a
[setting](configuration.md)). Documents are grouped by the **project**
they belong to. A file's project is the nearest ancestor
directory that looks like a project root — a version-control directory
(`.git`, `.hg`, `.svn`) or a build/manifest file (`Cargo.toml`, `go.mod`,
`package.json`, `pyproject.toml`, `Package.swift`, `build.zig`,
`Makefile`, …). Nearest wins: in a monorepo, a file inside a crate with
its own `Cargo.toml` belongs to that crate, not to the repository root.
Files outside any project gather under **Other**.

This is the same notion of "project" the rest of Textchum uses (and the
one language servers will be scoped by), so the drawer doubles as a
truth-teller: if a file is grouped somewhere surprising, that is exactly
how the rest of the application sees it too.

The current window's document is bold; documents with unsaved changes
show a dot. Clicking a document brings its window to the front.

## The project tree

The bottom pane shows the folder tree of the current document's project,
from its root. Clicking a file opens it — or brings its window forward if
it is already open. Documents without a project (the **Other** group)
show no tree.

Hidden files are not listed.

## Not there yet

- Rename / reveal-in-Finder actions on tree entries.
- Respecting `.gitignore` in the tree.
- A manual "this file belongs to that project" override for cases where
  the marker heuristic guesses wrong.
