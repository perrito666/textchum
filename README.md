# Textchum

A text editor for macOS in the spirit of TextMate: native, fast, and focused
on **editing and validating a vast range of file types** — not on being an
IDE.

Textchum is built as a portable compiled core (Rust) behind a fully native
shell (Swift + AppKit), meeting at a C interface. The core owns the text;
the shell owns the platform.

## Status

Early. Today: rope-backed buffers behind a C ABI, an async core→UI event
channel, and a minimal editor window whose text view is kept in verified
lockstep with a core buffer. Next up: documents (open/save/undo), syntax
highlighting, per-project language servers, Markdown preview.

## Quick start

```sh
make run      # build the core + app and launch the editor
make check    # what CI runs: tests, smoke test, header drift check
make docs     # build the documentation site (en/es/fr) into site/
```

Full documentation lives in [`docs/`](docs/index.md), built with MkDocs —
see [Getting started](docs/getting-started.md).

## The icon

The app icon is a tulip photographed by Horacio Duran in the flower
fields near Lisse, the Netherlands (52°19'54.1"N 4°37'25.9"E) on
21 April 2026 at 11:25.

## License

MIT
