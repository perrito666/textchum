# Textchum

Textchum is a text editor for macOS in the spirit of TextMate: native, fast,
and focused on one job — **editing and validating a vast range of file
types** — rather than being an IDE. There is no run button, no debugger, and
no plugin marketplace on the roadmap; there is (or will be) syntax
highlighting for a lot of languages, and language-server-backed validation
that respects project boundaries.

## How it is built

Textchum is split into two halves:

- **The core** (`libtextchum`), written in Rust, owns everything about the
  text: buffers, edits, and the event stream that keeps the UI informed. It
  compiles to a static library with a plain C interface and knows nothing
  about macOS.
- **The shell**, written in Swift with AppKit, owns everything about the
  platform: windows, rendering, input, and menus. It never holds document
  state of its own — every edit is routed through the core.

This split keeps the interesting logic portable and headlessly testable,
while the user-facing layer stays fully native. The [architecture
page](architecture.md) explains the reasoning and the rules of the boundary.

## Current state

Textchum is young. What exists and works today:

- A Rust core exposing rope-backed text buffers over a C ABI, with
  byte-offset and UTF-16 based editing (the latter matching how AppKit and
  the Language Server Protocol address text).
- Tree-sitter syntax highlighting for fourteen languages, incremental on
  every edit, with language injections (Markdown fences, HTML
  script/style) and light/dark palettes — see
  [Highlighting](highlighting.md).
- Documents on top of buffers: open and save with encoding detection and
  atomic writes, undo/redo with typing coalescing, and dirty-state tracking
  anchored to the last save — see [Documents](documents.md).
- Language-server validation with **one server instance per project**:
  diagnostics as you type, marked in the text and counted in the window
  subtitle, from independent per-project server processes — see
  [Language servers](language-servers.md).
- A navigation drawer in every window: open documents grouped by project
  (nearest root marker — the grouping language servers will share), with
  the current project's folder tree below — see
  [The navigator](navigator.md).
- JSON-backed configuration with a GUI settings window writing through to
  the file — including the appearance choice (system/light/dark): hand
  edits survive, broken files fall back to defaults and are backed up
  rather than clobbered — see [Configuration](configuration.md).
- An asynchronous event channel from core worker threads to the UI, with a
  strict single-thread delivery contract.
- A macOS editor with multiple windows (and native window tabs), open/save
  panels, save prompts on close, find and replace with regular expressions,
  and live watching of open files for changes made by other programs; every
  text view is kept in lockstep with its core document through a
  synchronization protocol that refuses to let the two sides diverge.
- A headless smoke test that exercises the whole Swift ↔ core round trip —
  editing, undo, saving, reopening, events — used both by CI and by humans
  in a hurry.

What is planned next, roughly in order: more language-server features
(completion, hover, go-to-definition), and Markdown preview.

## Where to go from here

- [Getting started](getting-started.md) — build and run Textchum from source.
- [Architecture](architecture.md) — the core/shell split and its rules.
- [Documents](documents.md) — undo, dirty state, encodings, atomic saves.
- [Highlighting](highlighting.md) — languages, injections, and the theme.
- [Configuration](configuration.md) — the settings window and its JSON file.
- [The C boundary](ffi.md) — conventions of the interface between the two.
