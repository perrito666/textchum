# Language servers

Textchum validates code through the
[Language Server Protocol](https://microsoft.github.io/language-server-protocol/),
with one defining behavior: **one server instance per project**.

## One instance per project

Server processes are keyed by *(server, project root)*, using the same
notion of project as [the navigator](navigator.md): the nearest ancestor
directory with a root marker. Open files from two different Rust projects
and two independent `rust-analyzer` processes run, each initialized with
its own root, each seeing only its own project's files. Cross-project
leakage — diagnostics from one workspace bleeding into another, an index
built over your whole home directory — cannot happen by construction.

Files outside any project get a per-directory instance, so loose files
never join someone else's workspace either.

## What you see

- Findings arrive as you type (sent in debounced batches) and mark the
  offending text: red for errors, orange for warnings, blue for notes.
- The window subtitle counts them ("2 errors, 1 warning").
- **Completion as you type**: suggestions appear after identifier
  characters and `.`, filtered as you keep typing — ↑/↓ to choose,
  ⏎ or ⇥ to accept, ⎋ to dismiss, ⌃Space to ask explicitly.
- Resting the mouse over a symbol shows the server's **hover**
  documentation in a popover.
- **Jump to Definition** (⌃⌘J) goes to the symbol under the caret —
  across files, opening or fronting the target as needed.
- **Find References** (⇧⌘R) lists every use of the symbol under the
  caret in a floating panel — ↑/↓ to move, ⏎ to jump.
- **Rename Symbol…** (⌃⌘R) renames across the whole workspace: open
  windows edit in place (undo works per window), files nobody has open
  are rewritten on disk.
- **Format Document** (⌥⇧⌘F) reformats through the server, keeping tabs
  if the document indents with tabs and spaces otherwise.
- A missing server is reported once, with the command that installs it;
  everything else about the editor keeps working without it.

## Servers

Textchum finds servers on `PATH` — it does not install them:

| Language | Server | Install |
|---|---|---|
| Rust | rust-analyzer | `rustup component add rust-analyzer` |
| Python | pyright | `npm install -g pyright` |
| Go | gopls | `go install golang.org/x/tools/gopls@latest` |
| C | clangd | Xcode CLT, or `brew install llvm` |
| JavaScript | typescript-language-server | `npm install -g typescript-language-server typescript` |
| Swift | sourcekit-lsp | ships with the Xcode toolchain |
| Zig | zls | `brew install zls` |
| Bash | bash-language-server | `npm install -g bash-language-server` |

## Choosing servers yourself

Settings → Language Servers overrides which command serves a language —
for every project (a *default*) or for a single project root. Project
entries win over defaults; unlisted languages use the table above. The
entries live in `config.json` under `"lsp"`, with the file's usual
hand-editing guarantees:

```json
{
  "lsp": {
    "defaults": {"python": "pylsp"},
    "projects": {"/work/projA": {"python": "pyright-langserver --stdio"}}
  }
}
```

Changes apply to servers started afterwards; the tab's **Restart Servers
Now** retires running instances and respawns them under the new
configuration.

## When there is no server

Two safety nets cover the no-server case:

- **The ctags fallback.** With **Ctags fallback** enabled in
  Settings → Projects (as a default or per project, like every project
  flag), Jump to Definition is answered from a
  [Universal Ctags](https://ctags.io) index of the project whenever no
  language server is available — and whenever a running server has no
  answer. The index is built on first use and refreshed as you keep
  jumping; ctags knows names, not semantics, so it is a fallback, not a
  replacement.
- **The debug log.** Every decision on the road from "file opened" to
  "server running" — the resolved project root, which server was chosen
  and why, spawn failures with the exact `PATH` searched, and every
  status transition — is appended to:

  ```
  ~/Library/Logs/Textchum/lsp.log
  ```

  Each server's own error output (stderr) is captured there too, so a
  server that exits during startup leaves its complaint on record — a
  command missing its transport flag (pyright's `--stdio`, say) is
  diagnosed in one glance, and the log notes outright when a custom
  command omits arguments the built-in registry knows are required.
  When a project mysteriously has no language support, this file names
  the missing piece.

One classic cause deserves a note: apps launched from Finder used to
inherit macOS's minimal `PATH`, which contains none of the places
language servers actually live (Homebrew, npm, cargo, go). Textchum now
adopts the login shell's `PATH` at startup — plus a few conventional
tool directories — so a server that works from the terminal works from
the Dock too.

## Under the hood

The client lives in the core, behind the same boundary as everything
else: JSON-RPC over stdio, an initialize handshake before any document
traffic, and full-document synchronization (incremental sync is a later
optimization). Server messages are handled off the UI thread and reach
the interface through the core's single event channel; a wedged server
process gets a bounded grace period at shutdown and is then killed, so
quitting Textchum can never hang on a misbehaving server. The whole
protocol path is exercised in CI against a scripted server.

## Not there yet

- Snippet placeholders in completions are flattened to plain text.
- ⌘-click as an alternative trigger for Jump to Definition.
- Markdown rendering in hover popovers (they show the raw text).
- Automatic restart of crashed servers (a crash is reported; reopening
  the file starts a fresh instance).
- Idle shutdown of unused instances, and a server-status panel.
