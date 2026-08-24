# Architecture

Textchum is two programs that meet at a C interface:

```
┌──────────────────────────────────────────────┐
│ Shell — Swift, AppKit                        │
│  windows · rendering · input · menus         │
└───────────────▲──────────────┬───────────────┘
                │ C calls      │ event callback
┌───────────────┴──────────────▼───────────────┐
│ Core — Rust, libtextchum (static library)    │
│  buffers · edits · events                    │
│  (soon: syntax, projects, language servers)  │
└──────────────────────────────────────────────┘
```

The division of labor follows one rule of thumb: anything that answers *"what
is the text, and what do we know about it?"* belongs in the core; anything
that answers *"how does it look and feel on this OS?"* belongs in the shell.
The core never draws. The shell never parses.

## Why a compiled core behind a native shell

- **The hard problems are platform-independent.** Ropes, incremental
  parsing, protocol clients — none of it cares about AppKit. Keeping it in a
  plain library makes it testable headlessly (`cargo test` covers the core
  with no UI in sight) and portable to other platforms later.
- **The user-facing layer should be boringly native.** Text input on macOS
  is deep — IME, dead keys, dictation, accessibility. Using real AppKit views
  means inheriting all of it instead of reimplementing it.
- **A C ABI is the widest possible boundary.** Swift consumes C headers
  natively; so does everything else that might one day host the core.

## The source-of-truth rule

The single most important invariant in the codebase: **the core buffer owns
the document; anything the UI holds is a display cache.**

Concretely, in today's editor window:

1. AppKit reports every impending text change — typing, paste, drop, undo —
   through one delegate method, as a UTF-16 range plus replacement string.
2. The shell applies that exact edit to the core buffer *first*.
3. Only if the core accepts it does the view proceed with its own change. A
   rejection (which would indicate a bug) refuses the view edit too, so the
   two sides can only move together.
4. Debug builds additionally assert byte-equality of both sides after every
   change.

Positions cross the boundary in the two units the ecosystem actually uses:
byte offsets (the core's native unit) and UTF-16 code units (AppKit's and
LSP's native unit). The core does all conversions; the shell never counts
code points.

## Threading contract

Simple rules, strictly kept:

- The shell calls into the core **only from the main thread**.
- The core owns all worker threads and delivers events through **one**
  callback invoked from **one** dedicated dispatch thread — never from the
  caller's thread, never concurrently with itself.
- The Swift wrapper (`TextchumKit`) hops events to the main actor before the
  app sees them, so application code lives entirely on the main actor.

This costs a little parallelism at the boundary and buys the absence of a
whole category of races. Work that benefits from parallelism happens *inside*
the core, behind the single-threaded interface.

## Layers on the Swift side

| Layer | Responsibility |
|---|---|
| `CTextchum` | The generated C header exposed as a Clang module. No code. |
| `TextchumKit` | Safe Swift API: classes with deterministic ownership, `NSRange`-based editing, typed events on the main actor. The only place pointers appear. |
| `Textchum` | The application: windows, views, menus. Ordinary Swift, no FFI. |

The same layering is expected of any future shell: a thin binding, a safe
idiomatic wrapper, then the app.
