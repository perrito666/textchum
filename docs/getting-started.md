# Getting started

Textchum currently builds and runs on macOS only.

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
| `make docs` | Builds this documentation site into `site/`. |
| `make clean` | Removes all build products. |

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
