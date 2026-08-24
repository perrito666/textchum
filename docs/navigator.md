# The navigator

Every editor window carries a navigation drawer on its left (toggle with
**⌘0**, or View → Toggle Navigator). It has two stacked panes.

## Open buffers, grouped by project

The top pane lists the open documents of **this window's tab group** —
files opened as tabs share one list, while separate windows keep separate
worlds (whether files open as tabs or windows is a
[setting](configuration.md)). Documents are grouped by the **project**
they belong to, resolved in this order:

1. the nearest `.textchum.json` — the explicit, human-placed override;
2. the **outermost version-control root** (`.git`, `.hg`, `.svn`): a
   repository is one project no matter how many nested manifests it
   contains — a Python package in a subfolder belongs to the repo, and
   nested repositories resolve to the outermost one;
3. outside version control, the nearest build/manifest file
   (`Cargo.toml`, `go.mod`, `package.json`, `pyproject.toml`,
   `Package.swift`, `build.zig`, `Makefile`, …).

Files outside any project gather under **Other**. Step 2's
repository-wins rule can be relaxed per project — the **Manifest
projects** switch in [Settings → Projects](configuration.md) splits a
root at its language manifests again, for repositories that are really
several projects in a trenchcoat.

Rows show bare filenames — until two open files share one, in which
case each shows just enough trailing path to tell them apart (tab
titles follow suit). The button at the top of the list — or
View → Toggle Path Display (⌥⌘T) — switches every row to its path from
the project root while it is on; deliberately not remembered across
launches — it is a quick look, not a mode.

Right-clicking a **project's header** offers window arrangement for the
whole group: **Split into New Window** pulls the project's documents out
into a window of their own (as tabs of it), and **Gather Into** is a
submenu of destinations — This Window, or any other open window (its
tab group, really) — that adopts the project's documents there as tabs. The divider between
the buffer list and the folder tree is one shared position — dragging
it in any tab moves it in all of them, and it is remembered with the
session.

Right-clicking a buffer row or a tree entry offers the file's location
in every useful spelling: the bare name, the path relative to the
project root, the absolute path, and — inside a git repository with a
remote — the file's URL on its forge, speaking GitHub's, GitLab's, and
Forgejo's URL shapes natively. The same items act on the front tab from
**File → Copy Path**.

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

Expanded folders are shared state: open a folder in one tab and it is
open in every tab (and in any window showing the same project).

Hidden files are not listed.

## Not there yet

- Rename / reveal-in-Finder actions on tree entries.
- Respecting `.gitignore` in the tree.
- A manual "this file belongs to that project" override for cases where
  the marker heuristic guesses wrong.
