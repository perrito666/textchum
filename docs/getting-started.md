# Getting started

Textchum's home platform is macOS; an experimental Linux shell over the
same core builds too — see [Linux](#linux-experimental) below.

## Prerequisites

- macOS 14 or newer.
- The Swift toolchain, version 6 or newer. The Xcode Command Line Tools are
  enough (`xcode-select --install`); the full Xcode application is not
  required.
- A Rust toolchain (stable). The easiest install is [rustup](https://rustup.rs).
- `make`, which ships with the Command Line Tools.

## Build and run

```sh
git clone https://github.com/perrito666/textchum
cd textchum
make run
```

`make run` builds the Rust core as a static library, generates the C header,
builds the Swift application against it, and launches the editor.

Other useful targets:

| Target | What it does |
|---|---|
| `make build` | Builds core and app without launching. |
| `make test` | Runs the Rust test suite. |
| `make smoke` | Builds everything, then runs the headless smoke test. |
| `make check` | Everything CI runs: tests, smoke test, header drift check. |
| `make app` | Builds a double-clickable `Textchum.app` (with icon) into `dist/`. |
| `make docs` | Builds this documentation site into `site/`. |
| `make clean` | Removes all build products. |

Prefer not to build? Every `v*` tag publishes a
[GitHub release](https://github.com/perrito666/textchum/releases) with a
ready-made `Textchum.app` zip (and its SHA-256). The app is not
code-signed, so on first launch right-click it and choose Open.

## Repository layout

```
textchum/
├── core/                    Rust workspace
│   ├── textchum-core/         the editor core (buffers, events)
│   └── textchum-ffi/          C ABI over the core; generates textchum.h
├── macos/                   Swift package
│   └── Sources/
│       ├── CTextchum/         the generated C header as a Clang module
│       ├── TextchumKit/       safe Swift wrapper over the C interface
│       └── Textchum/          the AppKit application
├── docs/                    this documentation (MkDocs)
└── Makefile                 the entry point for every task above
```

## The `chum` command

**Textchum → Install chum Command…** (or, from a checkout,
`make install-cli` — honors `PREFIX`, default `/usr/local`) installs a
small terminal command that talks to the running app; the menu route
asks for administrator rights only when `/usr/local/bin` needs them:

```sh
chum notes.md                # open (tab or window per your settings)
chum +42 src/main.rs         # open with the caret on line 42
chum -w big.md               # force a separate window
chum -t a.rs +7 b.rs         # several files, tabs, one with a line
chum --wait draft.md         # block until the window is closed
```

`--wait` is what tools that spawn an editor and read the file afterwards
need — save, close the window, and the caller resumes:

```sh
git config --global core.editor "chum --wait"
```

Closing without saving leaves the file untouched, which git reads as an
aborted commit — the same gesture as `:q!`. If Textchum quits (or is
gone), waiting chums are released rather than left hanging.

It works through the `textchum://` URL scheme, so the app bundle
(`make app`) must have been launched at least once to register it.

## Linux (experimental)

The same core drives a native GTK4/libadwaita shell, linked as a Rust
crate rather than through the C header (both sides are Rust there). It
is young — one window per file, core-owned editing and undo, a header-bar menu listing everything available, tree-sitter
highlighting from the shared theme table, open/save, in-file search
(Ctrl+F), project-wide fuzzy open (Ctrl+P), and the language-server
pool wired in: diagnostics as squiggles with a problem count in the
title, jump to definition (F12), and server trouble surfaced as toasts
— but it is the real
architecture, not a port: the sync protocol and its debug assertions are
the macOS ones translated.

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev
cargo build --release --manifest-path linux/Cargo.toml
linux/target/release/textchum-gtk notes.md
```

CI builds it and runs its headless smoke test on every push.

## Building the documentation

The documentation is a [MkDocs](https://www.mkdocs.org) site using the
Material theme, with English, Spanish and French builds. It is fully static:
host the generated `site/` directory with any web server.

```sh
python3 -m venv .docs-venv
.docs-venv/bin/pip install -r docs/requirements.txt
.docs-venv/bin/mkdocs serve    # live-reloading preview at localhost:8000
.docs-venv/bin/mkdocs build    # static site into site/
```

`make docs` wraps the same steps.

## Troubleshooting

- **`xcodebuild` errors about a "command line tools instance"** — harmless;
  Textchum does not use `xcodebuild`. Build with `make` (which drives
  `swift build`).
- **Linker cannot find `-ltextchum`** — the Rust core has not been built yet.
  Run `make core` (or any `make` target that includes it) before invoking
  `swift build` by hand.
