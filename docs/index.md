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
- An asynchronous event channel from core worker threads to the UI, with a
  strict single-thread delivery contract.
- A minimal macOS app: one window, one editable text view, kept in lockstep
  with a core buffer through a synchronization protocol that refuses to let
  the two sides diverge.
- A headless smoke test that exercises the whole Swift ↔ core round trip,
  used both by CI and by humans in a hurry.

What is planned next, roughly in order: real document handling (open, save,
encodings, undo), syntax highlighting, per-project language servers, and
Markdown preview.

## Where to go from here

- [Getting started](getting-started.md) — build and run Textchum from source.
- [Architecture](architecture.md) — the core/shell split and its rules.
- [The C boundary](ffi.md) — conventions of the interface between the two.
