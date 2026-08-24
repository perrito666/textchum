# The C boundary

The shell and the core meet at a single C header, `textchum.h`. It is
generated from the Rust source by [cbindgen](https://github.com/mozilla/cbindgen)
during every core build, and committed so shell-side tooling works without a
Rust toolchain. CI fails if a build leaves the header out of date, so it can
never drift from the code.

## Conventions

Every function in the interface follows the same small set of rules.

**Opaque handles.** The core's types (`TcApp`, `TcBuffer`) are opaque
structs; callers hold pointers, pass them back, and release them with the
matching `tc_*_free` function. Nothing is ever allocated by the caller on the
core's behalf.

**UTF-8 in, explicit lengths.** Strings passed *into* the core are
`(pointer, length)` pairs of UTF-8 bytes — no nul terminators required, no
encodings other than UTF-8. Strings returned *by* the core are nul-terminated
UTF-8 owned by the core; release them with `tc_string_free`.

**Two position units.** Functions address text either in UTF-8 byte offsets
(the core's native unit) or in UTF-16 code units (suffixed `_utf16`), because
that is what `NSRange` and the Language Server Protocol count in. The core
does the conversion; callers use whichever unit they naturally have.

**Transactional failure.** Fallible calls return `bool`. `false` means the
input was validated, rejected, and **nothing changed** — an out-of-bounds
offset, a mid-character byte position, invalid UTF-8. Callers can always
treat failure as "the operation did not happen".

**Panics do not cross.** Every entry point catches Rust panics and converts
them into the function's failure value. A bug in the core cannot unwind into
Swift stack frames.

**One thread in, one thread out.** Calls into the core must come from a
single thread. Events flow the other way through the callback registered
with `tc_app_new`, invoked on a single core-owned dispatch thread. The
callback's job is to marshal to the shell's UI thread; `TextchumKit` does
exactly that and nothing else.

## The event channel

Some information originates inside the core (today: pong replies used to
verify the channel; next: diagnostics from language servers, highlight
invalidations). It reaches the shell as a `TcEvent` — a `kind` discriminant
plus event payload — delivered to the registered callback.

Shells must tolerate unknown `kind` values: a newer core emitting an event an
older shell does not understand is forward compatibility, not an error.

`tc_app_free` blocks until queued events have been delivered and guarantees
the callback is never invoked afterwards, which is what makes teardown safe
to write on the shell side.

## Current surface

| Function | Purpose |
|---|---|
| `tc_version` | Core version as a static string. |
| `tc_app_new` / `tc_app_free` | Create/destroy a core instance and its event channel. |
| `tc_app_ping` | Request an async pong; exercises the event path. |
| `tc_buffer_new` / `tc_buffer_free` | Create/destroy a text buffer. |
| `tc_buffer_insert` | Insert UTF-8 at a byte offset. |
| `tc_buffer_delete` | Delete a byte range. |
| `tc_buffer_replace_utf16` | Replace a UTF-16 code unit range — the shape of an AppKit edit. |
| `tc_buffer_text` | Copy out the full contents. |
| `tc_buffer_len_bytes` / `tc_buffer_len_utf16` | Lengths in both units. |
| `tc_string_free` | Release a core-returned string. |

The surface is deliberately small and grows only when a shell feature needs
it. Bulk data (future: highlight spans, diagnostics) will cross as compact
structs or serialized payloads rather than one call per item.

## How Swift consumes it

The `CTextchum` target wraps the header as a Clang module, so Swift imports
it like any library. `TextchumKit` then translates raw calls into idiomatic
Swift — classes with `deinit`-based ownership, `NSRange` parameters, thrown
errors for rejected operations, and typed events delivered on the main
actor. Application code never touches a pointer.
