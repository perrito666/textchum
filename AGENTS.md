# Working on Textchum

Notes for anyone sending changes — human or AI assistant. They are not
style trivia; they are the handful of decisions that keep this codebase
coherent, written down so a newcomer can follow them without guessing.

## The shape of the thing

A portable compiled **core** in Rust owns the text; **shells** own their
platform. macOS (Swift + AppKit) reaches the core through a C ABI;
Linux (GTK4 + libadwaita) links the crates directly.

The division is not decoration, and it decides where your change goes:

- **Anything both shells would otherwise duplicate belongs in the
  core** — document state, undo, syntax, search, configuration
  resolution, Markdown rendering, project detection. If you find
  yourself writing the same logic twice in two languages, you are
  writing it in the wrong place.
- **Anything platform-shaped belongs in the shell** — menus, windows,
  panels, key handling, file dialogs, spell checking (each platform has
  its own engine).
- **The core never decides where files live.** Config and session paths
  are platform conventions; shells pass them in.

## Principles the code already follows

**Every edit goes through one choke point.** The core owns the
document; the shell routes each change through a single place and
debug builds assert the two sides stay byte-identical afterwards. If
you find a second path that mutates text, that is a bug, not a
shortcut.

**Failures explain themselves.** "No language server is running for
this document" beats a beep; "save the file so a python server can
attach, or add a save preprocessor for python" beats "no". When
something cannot work, say what would make it work.

**Degrade, never block.** A missing language server, a broken config, a
formatter that is not installed — each of these makes one feature
unavailable and nothing else. A broken `config.json` is preserved,
backed up, and reported; the app runs on defaults meanwhile.

**Configuration is a plain JSON file the GUI edits.** There is exactly
one store. Settings windows read and write the same file a person can
edit by hand, unknown keys survive round trips, and writes are atomic.
Adding a setting means: core getter/setter with a total default, FFI
pair, both shells' UI, and the docs.

**Per-project settings replace defaults, never append.** Hide globs,
preprocessor chains, server commands: a project entry is the whole
answer for that project. Keep it that way; "sometimes merged" is
impossible to reason about.

**Parity is the default.** A feature that lands on one shell should
land on the other, or the gap gets written down in `PLAN_linux.md`.
Both shells share the core's behaviour, not just its data.

**Nothing calls out to the tool it is imitating.** Hugo support does
not run `hugo`; the Markdown preview does not shell out. Read the
syntax, render honestly, and show a placeholder where executing
something would be a lie.

## Verifying

`make check` is what CI runs: core tests, the headless smoke test, and
a check that the generated C header is not stale. Run it before you
push.

- **Core logic gets a unit test.** Rust tests live beside the code they
  cover.
- **Anything crossing the FFI or the shells gets smoke coverage.**
  `make smoke` drives a real app end to end without a window server;
  the GTK shell has the same via `--smoke-test`.
- **UI changes get looked at.** The app has hidden `--debug-panel`
  hooks (files, grep, settings, palette, outline, hover, complete,
  status, about, …) and a `--config <path>` flag precisely so a screen
  can be opened and screenshotted without touching a real profile.
  Never point a test run at the real configuration.
- **Do not claim something works because it compiles.** Run it, look at
  it, and if you could not verify a piece, say so.

## Sending a change

- **Open an issue first for anything non-trivial**, then fix it, then
  close the issue with what you found. The issue is where the
  reasoning lives; the commit is where the change lives.
- **Commit messages are prose.** Say what changed and *why it was
  wrong before* — the second half is the part future readers need.
  Present tense, no bullet lists of files, no attribution trailers or
  mentions of the tool that helped you write it.
- **Comments explain constraints, not mechanics.** Write down the thing
  the next reader cannot see: why a borrow is dropped before a call,
  why an order matters, which upstream bug a workaround exists for.
  Never narrate what the line already says.
- **Documentation is part of the change.** The site is written in
  English, Spanish, and French; a user-visible feature updates all
  three. Docs describe what the editor does today — no phase numbers,
  no roadmaps, no mention of how the code was produced.
- **Say what is missing.** Every feature here ships with an honest
  "not there yet" line where one applies. A known gap named in the
  docs is worth more than a claim that has to be walked back.

## House rules

- Third-party GitHub Actions are pinned to a commit SHA.
- The main branch takes reviewed, green pull requests.
- Releases are tagged `vX.Y.Z`; the macOS build is signed and
  notarized, and the release notes are written by hand rather than
  generated from commit subjects.
