# Textchum — Planning Document

A modern take on TextMate for macOS: a fast, native text editor whose job is to **edit and
validate a vast number of file types**. Syntax highlighting and language intelligence (LSP)
are first-class; run/debug/IDE machinery is explicitly out of scope.

## 1. Product goals

**In scope**
- Fast native macOS editor (AppKit/SwiftUI), TextMate-like in spirit: lightweight, keyboard-friendly, project-aware.
- Syntax highlighting for a large set of languages.
- LSP support with **one server instance per project group**: opening files from three
  different Python projects spawns three independent `pyright`/`pylsp` instances, each with
  its own root and workspace state.
- Validation surfaced prominently: diagnostics, hover, completion, go-to-definition, rename, formatting.
- Markdown authoring in three tiers of priority:
  1. plain text editing with highlighting (must have),
  2. text + live preview pane (should have),
  3. WYSIWYG/hybrid rendering (nice to have, last).

**Out of scope (deliberately)**
- Run/debug/tasks, terminals, VCS UIs, plugin marketplaces, remote development.
- Cross-platform UI at first. The architecture keeps the door open (see §2), but no
  Linux/Windows work is planned in these phases.

## 2. Architecture: compiled core + native shell (the Ghostty model)

Like Ghostty (`libghostty` + Swift app), Textchum is split into:

```
┌──────────────────────────────────────────────┐
│ macOS app (Swift, AppKit + SwiftUI)          │
│  windows/tabs, text view & rendering, input, │
│  find UI, preview pane, preferences          │
└───────────────▲──────────────┬───────────────┘
                │ C ABI        │ callbacks (events)
┌───────────────┴──────────────▼───────────────┐
│ libtextchum (compiled core, one static lib)  │
│  buffers (rope) · undo · file I/O            │
│  syntax engine (tree-sitter) · themes        │
│  project/workspace model                     │
│  LSP client pool (per-project instances)     │
│  markdown pipeline · search                  │
└──────────────────────────────────────────────┘
```

**Division of labor rule of thumb:** anything that answers "what is the text and what do we
know about it?" lives in the core. Anything that answers "how does it look and feel on this
OS?" lives in the shell. The core never draws; the shell never parses.

### Core language: Rust (recommended), Zig as the documented alternative

Requirements: compiled, produces a static library with a plain C ABI that Swift can link,
good concurrency for LSP process management, and — critically for this product — leverage
for the hard subsystems.

- **Rust — recommended.** The ecosystem hands us most of the core: `ropey` (rope buffer),
  `tree-sitter` (first-party Rust bindings, incremental parsing, highlight queries),
  `lsp-types` (the whole LSP protocol, typed), `serde_json` + a small JSON-RPC framing
  layer, `tokio` or plain threads for server process I/O. `cargo` builds a `staticlib`;
  `cbindgen` generates the C header; a SwiftPM package wraps it. This is a proven path
  (Zed's core is Rust; several Swift apps embed Rust cores).
- **Zig — viable, more build-your-own.** Best-in-class C interop and cross-compilation, and
  tree-sitter (a C library) embeds trivially. But the rope, the LSP client, JSON-RPC, and
  protocol types would all be hand-written. Choose Zig only if the language itself is a
  goal; it likely costs Phase 1–3 an extra 30–50%.
- **Go — rejected.** `-buildmode=c-archive` works, but the runtime comes along (GC, signal
  handlers that historically clash with Cocoa), cgo callback overhead is real on hot paths
  (per-keystroke calls cross the boundary), and shipping Go inside a Mac app bundle is
  swimming upstream. Its strengths (network services) aren't what this project needs.

**Decision to confirm in Phase 0:** default is Rust. Phase 0's walking skeleton is small
enough that building it twice (Rust and Zig) is a legitimate option if you want the
comparison to be empirical rather than argued.

### The FFI boundary

- One C header (`textchum.h`), generated from the core (`cbindgen` for Rust).
- **Handle-based API**: opaque pointers for `tc_app`, `tc_buffer`, `tc_workspace`. No
  structs with internals exposed; versioned functions.
- **Calls in** are synchronous and cheap (apply edit, query highlight spans for a line
  range, request completion). **Events out** (diagnostics arrived, highlights updated,
  server crashed) go through a single registered callback with a tagged event + payload
  (serialized as a small C struct or msgpack blob); the Swift side marshals to the main queue.
- **Threading contract:** the shell calls the core only from the main thread; the core owns
  all worker threads (parsers, LSP I/O) and never calls the event callback from more than
  one dispatching thread. This one rule prevents 90% of FFI misery.
- Strings crossing the boundary are UTF-8 with explicit lengths; positions are (line, UTF-16
  code unit) at the boundary to match both LSP and AppKit conventions, with the core
  handling conversions from its internal byte offsets.

### Text rendering strategy (the riskiest UI decision)

Two-stage plan:

1. **Start on TextKit 2** (`NSTextView` + `NSTextLayoutManager`). The core rope is the
   single source of truth; `NSTextStorage` is a synchronized cache. All edits — typed,
   pasted, or programmatic — are routed through the core first, which returns the applied
   delta the shell replays into text storage. This gives us native input handling (IME,
   dictation, emoji picker, accessibility, kill ring) for free, which is a mountain of work
   to reimplement.
2. **Keep a custom Core Text renderer as the escape hatch**, not a goal. Only build it if
   TextKit 2 measurably fails on large files or highlight churn. The architecture supports
   it because the shell only ever asks the core "give me styled spans for lines N..M".

Known risk: the rope↔NSTextStorage sync protocol is a classic source of subtle bugs.
Mitigation: a single choke point (`applyEdit`) with debug-mode checksum assertions comparing
both sides after every edit.

## 3. Subsystem designs

### 3.1 Buffers and documents

- Rope-backed buffers (`ropey`), byte-indexed internally with line/UTF-16 conversion utilities.
- Undo/redo as edit-delta stacks with grouping (typing coalescing, one group per command).
- File I/O in the core: encoding detection (UTF-8 default, latin-1/UTF-16 fallback),
  newline preservation, atomic save (write-temp-then-rename), external-change detection via
  FSEvents (shell detects, core reconciles).
- Large-file posture: highlighting and LSP disable themselves above configurable thresholds;
  the editor itself must stay responsive on 100 MB files.

### 3.2 Syntax highlighting

- **Engine: tree-sitter.** Incremental parsing (keystroke-time re-parse), highlight/injection
  /locals queries, and a huge existing grammar collection. Grammars come from crates.io
  packages (cargo compiles their C sources — zero build machinery, covers the mainstream
  languages); a vendoring manifest/pipeline joins later for the long tail, and dynamic
  loading of grammar `.dylib`s is an option if binary size becomes a problem.
- **Injections** are required early because Markdown depends on them (fenced code blocks
  highlight in their own language), as do HTML/JS/CSS.
- **Themes** map tree-sitter capture names (`@keyword`, `@string`, …) to colors/weights,
  defined in JSON — see §3.7 for the full theme/appearance design.
- **Long tail:** tree-sitter covers the important ~60 languages. If coverage of exotic types
  matters later, an optional TextMate-grammar fallback engine (e.g. `syntect`-style) can sit
  behind the same "styled spans" interface — noted as a stretch item, not planned work.

### 3.3 Projects and the LSP instance pool (the differentiating feature)

The workspace model exists chiefly to serve the "one LSP per project group" requirement.

- **Project detection:** when a file opens, the core walks up from it looking for root
  markers (`.git`, `pyproject.toml`, `go.mod`, `Cargo.toml`, `package.json`, …, per-language
  configurable). The nearest match defines the file's *project root*. No markers → the file
  is a *loose file* grouped under its directory.
- **Instance pool:** LSP instances are keyed by `(server_id, project_root)`. Opening
  `~/work/projA/x.py` and `~/work/projB/y.py` yields two `pyright` processes, each
  initialized with its own `rootUri` and workspace configuration. Instances ref-count their
  open documents and shut down after a configurable idle period.
- **Manual override:** the UI exposes each buffer's resolved (server, root) assignment and
  lets the user re-root a file or pin a directory as a project — auto-detection will
  sometimes guess wrong (monorepos, symlinks) and must be correctable, not magical.
- **Server registry:** declarative config mapping language → server command, args, root
  markers, initialization options. Ship curated defaults (pyright, rust-analyzer, gopls,
  typescript-language-server, clangd, zls, …); user config extends/overrides in JSON.
  Textchum does **not** install servers in v1 — it finds them on PATH and gives actionable
  "server not found, install with …" messages.
- **Client scope for v1:** initialize/shutdown, didOpen/didChange (incremental)/didSave/
  didClose, publishDiagnostics, completion (+resolve), hover, definition, references,
  rename, documentSymbol, formatting. Explicitly later: workspace symbols, code actions,
  semantic tokens (tree-sitter covers coloring), inlay hints.
- **Resilience:** servers crash. The pool restarts with backoff, surfaces status per
  instance in the UI (a status item listing live servers, their roots, and memory), and
  never lets a dead server block editing.

### 3.4 Markdown

1. **Tier 1 (Phase 2):** tree-sitter-markdown highlighting with injected fenced-code
   languages. Nothing special — Markdown is just another language.
2. **Tier 2 (Phase 4):** split-pane live preview in a `WKWebView`. The core parses to an AST
   (`pulldown-cmark` or `comrak`), emits HTML with source-line anchors; the shell does
   debounced re-render with DOM patching (morphdom-style) to avoid flicker, plus two-way
   scroll sync via the anchors. Styling via bundled CSS themes matching editor themes.
3. **Tier 3 (Phase 6, stretch):** hybrid "reveal markup at the caret" rendering (à la
   Typora/Obsidian live preview) — decorations over the real source, never a lossy
   rich-text conversion. This is the hardest rendering work in the project and stays last;
   if TextKit 2 attributes + inline attachments can't express it cleanly, it may motivate
   the custom renderer.

### 3.5 Shell (Swift app)

- AppKit backbone: `NSWindow`/`NSDocument`-style lifecycle, the editor as an AppKit view.
  SwiftUI for chrome where it's strong: preferences, sidebars, status popovers, command
  palette.
- **Tabs are a requirement**, not a nicety: many buffers per window is the normal working
  mode. Start with native `NSWindow` tabbing (free, Mission Control friendly); if it fights
  the sidebar model below (native tabs are one-window-per-tab, so each tab would carry its
  own sidebar), replace with an in-window tab bar over a single editor pane, driven by the
  same open-buffer list the navigator shows.
- **File navigation drawer** (toggleable left sidebar), two stacked panes:
  1. **Open buffers, grouped by project.** The top pane lists every open buffer in the
     window, grouped under the project they belong to (project = the workspace model's
     root for that file — git repo root or root-marker directory, the same grouping that
     scopes LSP instances; loose files gather under a "Other" group). Click to switch to
     that buffer; the dirty dot and filename mirror the tab/window state.
  2. **Folder tree of the selected buffer's project.** Below a split, the bottom pane
     shows the directory tree the selected buffer lives in, scoped to its project root
     (typically the enclosing git repo). Selecting a different buffer above retargets this
     tree to that buffer's project. Gitignore-aware listing; click to open files as new
     buffers in the same window. Thin by design: open/rename/reveal-in-Finder, not a file
     manager.
  - The drawer is a view over core-owned state (open documents + workspace roots + a
    directory listing API); the shell contributes only the outline views and the split.
    Buffer→project grouping therefore needs the Phase 3 workspace model — until then the
    tree scopes to the enclosing git repo as a stopgap.
- Command palette (⇧⌘P-style).
- Diagnostics UI: underlines + gutter marks, a per-buffer issues list, jump-to-next-error.
- Recent files: the standard File → Open Recent menu.
- **Content search "in and around the file" (⇧⌘F), ripgrep-style.** Search starts scoped
  to the current file's surroundings — its project root, or its directory for loose files —
  and the scope is a **path shown in an editable field**: widening the search is literally
  editing the path (up to `~` or `/` if you like). Engine in the core, in the spirit of
  ripgrep rather than shelling out to it: ripgrep's own crates (`grep-searcher`,
  `grep-regex`, the `ignore` walker), so results are gitignore-aware, fast, and parallel.
  Results stream into a list; clicking jumps to the file/line.
- **Fuzzy file finding (⌘T) with the same scope criteria.** Same editable root path, same
  ignore-aware walk; fzf-style scoring via an embedded matcher (e.g. the `nucleo` crate —
  fzf's spirit, in-process) rather than a subprocess. Type to filter, return to open.
  Both features share the "scope = a visible, editable path" principle so search never
  silently looks somewhere the user did not expect.

### 3.6 Configuration

- JSON in `~/Library/Application Support/Textchum/` (`config.json`, `themes/`, `servers.json`) —
  GUI-managed, hand-editable as the escape hatch (broken files are preserved and
  backed up, unknown keys survive GUI saves) —
  read by the core, with a Preferences UI writing through. Per-project overrides via
  `.textchum.json` at the project root (tab width, server choice).

### 3.7 Appearance & themes

- **Appearance mode is a user choice**: `system` (default — follow macOS and switch live
  when the system does), `light`, or `dark`. Stored in `config.json`
  (`"appearance": "system" | "light" | "dark"`), settable from the Settings window, applied
  app-wide immediately (windows, chrome, and syntax palette together).
- **Themes are JSON files** in `~/Library/Application Support/Textchum/themes/`, selected
  by name in `config.json`. A theme defines the syntax style table — capture name →
  {color, bold, italic} — with a light and a dark palette in one file, so one theme serves
  both appearance modes. Editor chrome colors (background, caret, selection) join the
  format when the editor grows beyond system colors.
- **Sane defaults ship built in**: the current default palette plus a small curated set
  (at minimum a high-contrast pair). Built-ins are compiled into the core and selectable
  like user themes; user files with the same name override built-ins.
- **A vanilla theme generator** bootstraps new themes: `Textchum --emit-theme <file>`
  (headless, like `--smoke-test`) writes a complete starter theme — every styled capture
  name enumerated with the default palette's values — so making a theme is "generate,
  open, change colors", never "guess the schema".
- **Same escape-hatch rules as configuration**: a theme that fails to parse falls back to
  the default with one warning, is never overwritten, and unknown keys survive tooling.

Scheduling: appearance mode lands with the configuration work (small); the theme file
format, built-in set, selection UI, and generator land in Phase 5 alongside the other
customization polish.

## 4. Repository layout & build

```
textchum/
├── PLAN.md
├── justfile                 # single entry point: just build / run / test / xcode
├── core/                    # Rust workspace
│   ├── textchum-core/       #   buffers, syntax, workspace, markdown
│   ├── textchum-lsp/        #   LSP client + instance pool
│   └── textchum-ffi/        #   C ABI surface (staticlib) + cbindgen config
├── grammars/                # manifest + vendored tree-sitter grammars
├── include/                 # generated textchum.h (checked in for Xcode)
└── macos/
    ├── Textchum.xcodeproj
    ├── TextchumKit/         # Swift wrapper: safe Swift API over the C ABI
    └── Textchum/            # the app: views, windows, preview, prefs
```

- Build flow: `just build` → cargo builds `libtextchum.a` (arm64 + x86_64, lipo'd) +
  regenerates the header → `xcodebuild` links it. Xcode run-script phase invokes cargo so
  plain Xcode development also works.
- Testing: core logic tested in Rust (rope ops, LSP framing against a scripted fake server,
  project-root resolution tables). Swift side gets XCTest for the sync protocol and FFI
  wrapper. A tiny headless harness drives the C API directly as an integration layer.
- CI (GitHub Actions, macOS runner): core tests, FFI header drift check, app build.

## 5. Phases

Each phase ends with something usable; exit criteria are the definition of done.

### Phase 0 — Decisions & walking skeleton (~1–2 weeks)
Prove the architecture end to end before writing real features.
- Repo scaffolding, `justfile`, CI.
- Core exposes `tc_version()` plus a toy buffer (create, insert, read back) over the C ABI;
  event callback round-trip demonstrated (core timer thread → Swift main queue).
- Swift app: one window, text view showing buffer contents fetched from the core.
- Confirm the language decision (optionally by also spiking the skeleton in Zig).
- **Exit:** typed text round-trips Swift → core → Swift; `just build && just run` works from
  a clean checkout; CI green.

### Phase 1 — A real editor, no intelligence (~4–6 weeks)
- Rope buffers, full edit pipeline through the core, undo/redo with coalescing.
- Open/save (encodings, atomic writes, external-change detection), multiple documents,
  window tabs, dirty-state + save prompts.
- The TextKit 2 sync protocol hardened (checksum assertions, fuzz test firing random edits
  at both sides).
- In-buffer find/replace with regex. Basic preferences (font, tab width).
- **Exit:** daily-drivable for plain-text notes; 100 MB file opens and scrolls smoothly;
  edit-sync fuzz test passes 1M operations.

### Phase 2 — Syntax highlighting (~3–4 weeks)
- tree-sitter integrated: incremental parse on edit, styled spans queried per visible range,
  async re-highlight events for off-screen changes.
- Grammar build pipeline; initial set (~15): Python, Rust, Go, Zig, C/C++, Swift, JS/TS,
  JSON, YAML, TOML, HTML, CSS, Markdown (+inline), Bash.
- Injections working (Markdown fences, HTML script/style).
- Theme engine, light/dark, system-appearance following.
- **Exit:** keystroke latency in a highlighted 10k-line file indistinguishable from Phase 1;
  Markdown with mixed-language fences highlights correctly.

### Phase 3 — Projects & LSP (~6–8 weeks, the heart of the project)
- Workspace model: root detection, loose-file grouping, manual re-rooting UI.
- First version of the **file navigation drawer** (§3.5): open buffers grouped by the
  workspace model's projects on top, the selected buffer's project folder tree below.
  It doubles as the workspace model's debug view — the grouping the drawer shows is the
  grouping the LSP pool keys on, so a wrong tree is visible before a wrong server is.
- Settle the tab question: native window tabs vs in-window tab bar (§3.5), decided by how
  tabs and the per-window drawer coexist in practice.
- LSP client (JSON-RPC over stdio, incremental didChange) and the **instance pool** keyed by
  `(server, root)` with idle shutdown, crash restart + backoff, and a server-status UI.
- Features in order: diagnostics → hover → completion → go-to-definition → references →
  rename → formatting → document symbols.
- Server registry + curated defaults; "server missing" guidance.
- **Exit — the acceptance test is the original requirement:** open files from two separate
  Python projects and observe two pyright instances, each reporting project-correct
  diagnostics and cross-file navigation confined to its own root. Kill a server process
  manually; editing continues and the pool recovers.

### Phase 4 — Markdown preview (~2–3 weeks)
- Split-pane WKWebView preview: core-emitted HTML with source anchors, debounced patch
  updates, bidirectional scroll sync, preview CSS themes.
- **Exit:** typing in a large README updates the preview within ~100 ms without flicker or
  scroll jumps.

### Phase 5 — Breadth & polish (~4–6 weeks, then ongoing)
- Grammar set to ~40 languages; server defaults to ~12.
- Content search (⇧⌘F, ripgrep crates) and fuzzy file-open (⌘T, nucleo) with the shared
  editable-scope-path design (§3.5); command palette; document outline (LSP symbols).
- Themes (§3.7): JSON theme format, built-in curated set, selection in Settings, and the
  `--emit-theme` vanilla theme generator.
- Navigation drawer polish: rename/reveal actions, gitignore-aware filtering options,
  drag to reorder buffer groups; `.textchum.json` per-project config.
- **Session restore: reopen where you left off.** Quitting remembers the open files and
  relaunching restores them — including each document's caret position and scroll
  offset, window/tab arrangement, and which window was frontmost. State lives in a plain
  JSON file next to the configuration (`session.json`), written atomically on quit and on
  window close, following the same escape-hatch rules as everything else: hand-readable,
  never a cache you cannot inspect. Files that no longer exist at restore time are
  skipped silently; per-file positions are also remembered for files reopened later via
  Open Recent, so "continue where I left off" works per document, not just per session.
  **Opening without memory is a first-class path, for debugging:** launch with
  `--fresh` (or hold ⇧ at launch) to ignore the saved session, and since the state is
  one deletable JSON file, `rm session.json` is always a complete reset — the app must
  start correctly from nothing, and that path is exercised by tests.
- Performance pass (startup time budget: < 300 ms to first window), crash reporting, app
  icon, notarized DMG/Sparkle or TestFlight distribution.
- **Exit:** you've stopped opening other editors for day-to-day work; a v0.1 build is
  installable by someone else from a DMG.

### Phase 6 — Stretch
- Hybrid/WYSIWYG Markdown (§3.4 tier 3).
- Custom Core Text renderer if TextKit 2 has hit measured limits.
- TextMate-grammar fallback for exotic languages; semantic tokens; code actions.
- A second platform shell (the core is already portable; a Linux GTK shell would be the test).

**Very-far stretch (explicitly last, after everything above):**
- **Images.** Opening an image file shows it instead of failing or spewing bytes. If macOS
  provides native editing machinery worth adopting (the `PhotosUI`/markup-style editors or
  whatever the OS then offers), use it; otherwise images are strictly **read-only** viewers
  — Textchum is not growing an image editor of its own. Shell-only feature: the core's
  involvement stops at "this is not text".
- **Binary files as hex dump.** Files that decode as neither UTF-8 nor Latin-1 text open in
  a hex-dump mode (offset · bytes · ASCII gutter, the classic layout). Read-only first;
  byte-level editing only if it ever earns its keep. Hex rendering is a presentation of the
  raw bytes, so this one does touch the core: the document layer learns to hold "bytes
  without a text decoding" and the shell renders the dump. Also the natural fallback UI
  when someone opens a 2 GB blob by accident.

## 6. Top risks & mitigations

| Risk | Mitigation |
|---|---|
| Rope↔NSTextStorage desync corrupts text | Single edit choke point; debug checksums; fuzz test from Phase 1 |
| TextKit 2 performance/quirks on big files | Measure early (Phase 1 exit criterion); custom renderer as planned escape hatch |
| FFI churn slows iteration | Handle-based API, one event callback, header generated not hand-written; grow the surface slowly |
| LSP servers are individually quirky (pyright vs gopls init options) | Per-server config in the registry; scripted fake server for protocol tests; integration tests against 2–3 real servers |
| Grammar build pipeline (C compilation, ABI versions) gets brittle | Vendor pinned revisions; build in CI; static linking first |
| WYSIWYG Markdown scope creep | Fenced into Phase 6; tiers 1–2 satisfy the stated priority ("text first") |
| Solo-project fatigue | Every phase ends usable; Phase 1 already replaces a plain-text editor |

## 7. Immediate next steps

1. Confirm core language (default: Rust) — or green-light the dual Phase 0 spike.
2. `git init`, scaffold the repo layout above, commit this plan.
3. Build the Phase 0 walking skeleton.
