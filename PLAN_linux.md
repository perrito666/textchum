# Linux shell — delta against macOS

The GTK4/libadwaita shell (`linux/`) rides the same core as the macOS
app and already covers the daily-driver set. This file tracks exactly
what separates the two shells, so the gap is a checklist rather than a
feeling. Architecture notes live in `PLAN.md` (§Phase 6); this is the
feature ledger.

## At parity

- Core-owned documents: the edit choke point, byte-identical debug
  assertions, undo/redo replayed from the core's history.
- tree-sitter highlighting from the shared style table, all languages
  including the name-detected ones (Makefile, git commit messages).
- Tabs (AdwTabView) with focus-not-duplicate opens; the drawer's both
  halves — open buffers grouped by project over the project file tree.
- Open/save with dirty marks; language + problem counts in the title.
- Find in Project: regex, smart case, stacked line/file filters, the
  says-what-it-did status line. Open Quickly over the core's matcher.
- LSP through the shared pool (one instance per project, crash restart,
  idle shutdown): diagnostics as squiggles with counts, completion as
  you type, hover, jump to definition with not-working-says-why toasts.
- Markdown preview pane (reload-on-settle; macOS DOM-patches).
- Preferences over the same `config.json`: appearance, theme, font
  size, tab width, line numbers, server defaults, per-project server
  overrides, and the workspace toggles (manifest projects, recursive
  config) as defaults and per-root.
- Everything reachable from the primary menu with its shortcut shown.

## Missing on Linux (the actual delta)

Ordered roughly by how much daily pain each gap causes.

1. **File watching + Revert to Saved.** The Linux shell does not watch
   open files at all: external changes go unnoticed until a manual
   reload that also does not exist yet. macOS follows the disk silently
   when clean, prompts when dirty, and has ⌥⌘R. `GFileMonitor` is the
   tool; the core's `reload()` does the rest.
2. **Session restore.** No `session.json` on Linux: no reopened files,
   no caret positions, no `--fresh`. The state format is shared and
   hand-readable; only the save/restore plumbing is missing.
3. **Rebindable keys.** The configuration's `keys` section is ignored;
   accelerators are hardcoded. Map the existing action names onto
   `set_accels_for_action` at startup.
4. **The jump stack.** No Go Back/Go Forward; definition jumps and
   search results leave no trail.
5. **LSP breadth.** References, rename, formatting, and the document
   outline are macOS-only; the pool methods exist in the linked crate,
   so each is a panel/action away. Hover on Linux also lags the Mac's:
   no Markdown rendering, no symbol gating (it fires over whitespace
   and comments), no on/off toggle, and no show-at-caret command. The
   LSP debug log is also unwired
   (`textchum_lsp::log::set_path` is never called — point it at
   `~/.local/state/textchum/lsp.log`).
6. **Replace in file.** The search bar finds; it does not replace, and
   has no regex/whole-word toggles (macOS uses the native find bar).
7. **Command palette.** Menu actions exist and are named; the
   fuzzy-searchable panel over them does not.
8. **The `chum` story.** `GApplication` already gives single-instance
   file opening over D-Bus (`textchum-gtk file.txt` from anywhere), but
   there is no `+line` handling, no `--wait` for `GIT_EDITOR`, and no
   installable wrapper script.
9. **Navigator polish.** No filename disambiguation, no path-display
   toggle, no copy name/relative/absolute/forge-URL menus, no project
   split/gather between windows, and tree rows use generic icons — no
   language badges or system type icons.
10. **New-file ergonomics.** No New with Format, no Save As directory
    seeding from the frontmost file.
11. **Editing niceties.** No auto-indent on return (shell-side on
    macOS), no block start/end navigation, no Redraw command.
12. **Theme files.** Only the built-in themes are selectable; the user
    theme JSON files and `--emit-theme` output are not read (the parser
    is in the core — wiring only).
13. **Ctags fallback.** Not implemented; depends on nothing
    macOS-specific.
14. **Recent files.** No recent-documents menu.
15. **Window subtitle detail.** macOS shows encoding · size · language
    · problems; Linux shows language · problems.
16. **Save preprocessors.** The `preprocessors` config section (chains
    of stdin→stdout formatters, defaults + per-root) is macOS-only:
    the resolution lives in the shared core, so Linux needs only the
    process-spawning half and a Run Save Preprocessors action.
17. **Prose spell check.** macOS scopes the system checker to comments
    and prose documents (`editor.spell`). GTK land would use
    libspelling/enchant over the same comment-span logic.
18. **Packaging.** No `.desktop` file, no icon, no Flatpak, no release
    artifact — the only install is `make linux` from a checkout.

## Behavioral differences that are choices, not gaps

- **Markdown preview updates** reload the whole document after edits
  settle; macOS patches the DOM and syncs scroll. Upgrade when it
  itches.
- **Windows vs tabs**: macOS offers open-in-tab-or-window as a setting
  with per-window drawers; Linux is one workbench per window with tabs,
  and New Window makes another workbench. The Linux model is simpler
  and fits AdwTabView; revisit only if someone misses the setting.
- **Toasts vs alerts**: server trouble is a toast on Linux, a one-time
  alert on macOS. Toasts are the better fit for GNOME; keep.

## Linux-only advantages worth keeping

- Single-instance + open-over-D-Bus for free from `GApplication`.
- `AdwTabView` was designed for editors: drag-reorder, overview, and
  pinning come with it (drag-reorder just works; the rest unexposed).
- The gutter is a first-class GtkSourceView API — none of the macOS
  sibling-view scar tissue.
- The shell links the core as a crate: no FFI ceremony, no header to
  drift.

## Suggested order

File watching (1) and session restore (2) make it trustworthy; keys
(3), jump stack (4), and LSP breadth (5) make it comfortable; the rest
is polish in whatever order irritation dictates. Packaging (18) last —
ship it to more people only once it deserves them.
