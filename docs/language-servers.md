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

- References, rename, formatting.
- Snippet placeholders in completions are flattened to plain text.
- ⌘-click as an alternative trigger for Jump to Definition.
- Markdown rendering in hover popovers (they show the raw text).
- Automatic restart of crashed servers (a crash is reported; reopening
  the file starts a fresh instance).
- Idle shutdown of unused instances, and a server-status panel.
