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
  documentation in a popover, with the Markdown servers send rendered
  — code blocks monospaced, emphasis and inline code styled. It only
  triggers over identifiers (never whitespace or comments), can be
  switched off in View ▸ Hover Documentation (or Settings), and
  **Show Documentation for Symbol** (⌃⌘H) asks for the symbol under
  the caret on demand — even with mouse hover off.
- **Jump to Definition** (⌃⌘J, or ⌘-click) goes to the symbol under
  the caret — across files, opening or fronting the target as needed.
- **Find References** (⇧⌘R) lists every use of the symbol under the
  caret in a floating panel — ↑/↓ to move, ⏎ to jump. Code comes
  first, tests after, each under a heading with a count: what calls
  this is the question, and what checks it is the follow-up. Which
  files are tests is a convention rather than a fact — a `tests`
  directory, a `parser_test.go`, a `Button.test.ts`, a
  `ParserTests.swift` — so the rule is a cautious one, and `latest.rs`
  is not a test. A Rust `#[cfg(test)] mod tests` inside an ordinary
  file is listed as code, which is what its path says. A result that
  is all one or all the other gets no headings.
- **A marked line can be read.** Resting the pointer on an underlined
  stretch shows what the server said, and **Show Diagnostic for Line**
  (⌃⌘E, Ctrl+Alt+E on Linux) says the same for the caret's line — the
  caret is usually at the end of the line being fixed rather than
  inside the mark, so the line is what it answers about. The message
  names its severity, because an underline says only that something is
  wrong and a warning should not read like an error. No round trip:
  the finding is already in hand.
- **Rename Symbol…** (⌃⌘R) renames across the whole workspace: open
  windows edit in place (undo works per window), files nobody has open
  are rewritten on disk.
- **Format Document** (⌥⇧⌘F) asks the server first and falls back to
  the save-preprocessor chain — so formatting works on untitled
  documents and languages without a server, whenever a chain is
  configured.
- **Format Document** (⌥⇧⌘F) reformats through the server, keeping tabs
  if the document indents with tabs and spaces otherwise.
- **Document Outline** (⇧⌘O) lists the file's symbols — nesting shown
  by indentation, fuzzy-filterable — and ⏎ jumps to the selection.
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

An entry's command is editable in place — fix a typo or add a missing
`--stdio` right in the row, press ⏎ or click away, no delete-and-re-add.
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
  replacement. It must be *Universal* Ctags (`brew install
  universal-ctags`) — the `ctags` macOS ships in `/usr/bin` is a
  different, much older program that cannot emit the JSON index this
  reads. Textchum looks past that one to find a real Universal Ctags
  further along your `PATH`.
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

Instances also look after themselves: a server that **crashes**
mid-session is restarted automatically with backoff (1 → 2 → 4 → 8
seconds; four failures in a row and it stays down until a restart or a
configuration change), and an instance **no open document has needed
for five minutes** is shut down — the next open starts a fresh one.

- Snippet completions expand and walk. The first placeholder comes
  back selected, so typing replaces it; ⇥ moves to the next stop and
  ⇧⇥ back to the previous one; a placeholder written more than once
  mirrors the one being typed. Reaching the end, pressing ⎋, or
  clicking outside gives the keys back.
- **View ▸ Language Server Status** lists the running instances and
  the session's recent status transitions, refreshed live, with a
  pointer to the full log.
