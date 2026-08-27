# Textchum

[![Documentation](https://img.shields.io/badge/docs-perrito666.github.io%2Ftextchum-14b8a6)](https://perrito666.github.io/textchum/)
[![Latest release](https://img.shields.io/github/v/release/perrito666/textchum)](https://github.com/perrito666/textchum/releases/latest)
[![CI](https://github.com/perrito666/textchum/actions/workflows/ci.yml/badge.svg)](https://github.com/perrito666/textchum/actions/workflows/ci.yml)

A text editor in the spirit of TextMate: native, fast, and focused on
**editing and validating a vast range of file types** — not on being an
IDE.

Textchum is built as a portable compiled core (Rust) behind a fully native
shell — Swift + AppKit on macOS, with a young GTK4/libadwaita shell for
Linux — meeting at a C interface. The core owns the text; the shell owns
the platform.

## What it does today

- **Documents the core owns**: rope-backed buffers, undo/redo with
  coalescing, atomic saves, external-change detection, and session
  restore down to caret positions (`--fresh` to skip it).
- **Syntax highlighting** via tree-sitter across 16 languages —
  Makefiles and git commit messages included — with
  injections (Markdown fences, HTML script/style), incremental re-parse
  on every edit, and auto-indent on return.
- **Themes**: seven built in — including Molokai, Solarized, Dracula,
  and Gruvbox — each with light and dark palettes; user themes are JSON
  files, and `--emit-theme` writes a complete starter to recolor.
- **One language server per project**: opening files from two projects
  spawns two independent server instances, each scoped to its own root.
  Diagnostics, hover, completion, jump to definition, references,
  rename across files, formatting, and a document outline (⇧⌘O) —
  with crash restart, idle shutdown, a ctags fallback for projects
  without a server, and a debug log
  (`~/Library/Logs/Textchum/lsp.log`) that explains every decision.
- **Navigation**: a per-window drawer (open buffers grouped by project
  over the project's file tree), window tabs or separate windows with
  project-level split/gather, fuzzy file open (⌘T) and ripgrep-style
  project search (⇧⌘F) with stacked filters and smart case, a command
  palette (⇧⌘P), and a jump stack — Go Back/Go Forward across every
  jump, vim-jumplist style.
- **Markdown**: live split-pane preview with DOM-patch updates and
  scroll sync.
- **`chum`**: a terminal command (`chum +42 main.rs`) installable from
  the app menu.
- **Configuration** that is GUI-driven but plainly hand-editable JSON —
  broken files are never clobbered, unknown keys survive, and every
  shortcut is rebindable.

## Install

Grab `Textchum.app` from the
[releases](https://github.com/perrito666/textchum/releases) (unsigned:
right-click → Open on first launch; Apple Silicon, macOS 14+), or build
from source:

```sh
make run      # build the core + app and launch the editor
make app      # a double-clickable Textchum.app in dist/
make check    # what CI runs: tests, smoke test, header drift check
make docs     # build the documentation site (en/es/fr) into site/
```

On Linux (GTK4/libadwaita, at feature parity with the macOS shell):

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev \
  libwebkitgtk-6.0-dev libsoup-3.0-dev
make linux
```

To install it for the current user:

```sh
make install-linux
```

This installs the binary, desktop entry, and icon into `~/.local`. If it does not appear in your app launcher immediately, log out and back in.

Full documentation lives in [`docs/`](docs/index.md), built with MkDocs —
see [Getting started](docs/getting-started.md).

## The icon

The app icon is a tulip photographed by Horacio Duran in the flower
fields near Lisse, the Netherlands (52°19'54.1"N 4°37'25.9"E) on
21 April 2026 at 11:25.

## Contributing

[AGENTS.md](AGENTS.md) records how this codebase is worked on — where
the core/shell line falls, the principles the code already follows, how
changes are verified, and what a commit and its documentation are
expected to carry. It is written for anyone sending changes, human or
AI assistant.

## Contributors

Textchum is written by [Horacio Duran](https://perri.to), with
contributions from:

- [Juan Diaz](https://github.com/nueces) — Linux install instructions
  and packaging polish, and the save-as highlighting report

## License

[MIT](LICENSE)
