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
| `make playground` | Makes a throwaway project and profile, and opens the working copy on it. |
| `make docs` | Builds this documentation site into `site/`. |
| `make clean` | Removes all build products. |

Prefer not to build? Every `v*` tag publishes a
[GitHub release](https://github.com/perrito666/textchum/releases) with a
ready-made `Textchum.app` zip and `textchum-gtk` Linux tarballs
(x86_64 and arm64), each with its SHA-256. The app is not
code-signed, so on first launch right-click it and choose Open.

## The playground

Building tells you the editor compiles. It does not tell you what a
change looks like against a git repository with a remote, a file with
uncommitted edits, a nested project, a misspelling and four hundred
lines to scroll — so `make playground` makes one and opens the working
copy of the editor on it.

```
make playground              build it and open the editor on it
make playground KEEP=1       reuse the one already made
make playground OPEN=0       make it and say where it is, no editor
```

Everything lands under `build/playground`: the project on one side, the
editor's whole profile on the other. What is in the project:

- **Python and Rust**, the Rust crate nested inside the Python project,
  so manifest projects and per-project settings have two roots to tell
  apart.
- **History** — four commits, two authors, four different months — so
  Blame Line has something to say.
- **A working copy in the state one is usually in**: lines changed,
  lines added, lines gone, something staged, something untracked. The
  change gutter marks all of it.
- **A remote that goes nowhere**
  (`github.com/textchum-playground/playground`), so Copy Forge URL
  produces a URL.
- **A file with a syntax error** for whichever language server is
  installed, **prose with a misspelling** for the spell pass, and
  **four hundred lines** to scroll.

The profile is handed over with `--data-dir`, so the configuration,
themes, icon packs, session and server log of that run are all inside
it and the real ones are never opened.

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
is younger than the macOS app but no longer a toy: tabs (AdwTabView)
with focus-not-duplicate opens, a project file-tree sidebar (F9),
core-owned editing and undo, tree-sitter highlighting from the shared
theme table, in-file search (Ctrl+F), Find in Project (Ctrl+Shift+F, regex with
smart case, stacked line/file filters, and a says-what-it-did status
line), fuzzy Open Quickly
(Ctrl+P), an Open Files list grouped by project over the tree, the
language-server pool wired in (squiggled diagnostics with problem
counts, completion as you type, hover, jump to definition on F12,
server trouble as toasts), a live Markdown preview pane
(Ctrl+Alt+P), and a
preferences window (Ctrl+,) over the same `config.json` contract —
appearance, theme, editor settings, and language servers — defaults,
per-project overrides, and the workspace toggles —
stored at `~/.config/textchum/config.json`.

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev \
  libwebkitgtk-6.0-dev libsoup-3.0-dev
cargo build --release --manifest-path linux/Cargo.toml
linux/target/release/textchum-gtk notes.md
```

CI builds it and runs its headless smoke test on every push.

### Packages

Each release carries a `.deb` and an `.rpm` beside the tarball, and the
repository has a `PKGBUILD` and a flake for the two distributions that
build from source:

```sh
sudo apt install ./textchum_0.0.10_amd64.deb     # Debian, Ubuntu
sudo dnf install ./textchum-0.0.10-1.x86_64.rpm  # Fedora, RHEL, openSUSE
(cd packaging && makepkg -si)                    # Arch
nix profile install github:perrito666/textchum   # Nix
```

`make deb` and `make rpm` build the first two from a checkout;
`nix build` and `nix develop` cover the last, the dev shell bringing
the GTK toolchain and the optional tools with it.

What every one of them depends on is the four libraries the shell links
against — GTK 4, libadwaita, GtkSourceView 5 and WebKitGTK 6. hunspell,
Universal Ctags and git are recommendations rather than requirements:
each switches off exactly one feature (prose spell check, the Jump to
Definition fallback, and Copy Forge URL) and nothing else.

Language servers and formatters are deliberately in nobody's dependency
list. They are yours, in versions only you know, and the editor runs
whatever is on your `PATH`.

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
