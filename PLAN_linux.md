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

1. ~~File watching + Revert to Saved.~~ Done: `GFileMonitor` per
   pathed page — clean buffers follow the disk silently, dirty ones
   get a Reload toast, the app's own saves are recognized and ignored,
   and Revert to Saved (Ctrl+Alt+R) confirms before discarding edits.
2. ~~Session restore.~~ Done: `~/.local/state/textchum/session.json`
   (same hand-readable shape as the Mac's), written eagerly on opens,
   closes, and quit; `--fresh` skips it once.
3. ~~Rebindable keys.~~ Done: the `keys` section's action names and
   `modifiers+key` specs map onto the win.* actions at startup (`cmd`
   lands on Ctrl).
4. ~~The jump stack.~~ Done: Go Back (Alt+Left) / Go Forward
   (Alt+Right) with the same clear-forward-on-new-jump contract;
   definitions, search results, and outline picks leave the trail.
5. ~~LSP breadth.~~ Done: References (Shift+F12), Rename (F2, open
   pages edit in place, unopened files rewritten on disk), Format
   (Ctrl+Shift+I), Document Outline (Ctrl+Shift+O), and the debug log
   in `~/.local/state/textchum/lsp.log`. Hover has parity too: Pango
   Markdown rendering, symbol gating (identifiers outside comments
   only), the `editor.hover` toggle in Preferences, and Show
   Documentation for Symbol (Ctrl+Alt+H).
6. ~~Replace in file.~~ Done: the search bar grew a replace row
   (Replace / All) and match-case, regex, and whole-word toggles.
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
11. **Editing niceties.** Auto-indent on return is in (GtkSourceView's
    own), and Redraw (Ctrl+Alt+L) too; block start/end navigation is
    still missing.
12. ~~Theme files.~~ Done: JSON files in `~/.config/textchum/themes/`
    join the built-ins in Preferences, named by file stem.
13. **Ctags fallback.** Not implemented; depends on nothing
    macOS-specific.
14. **Recent files.** No recent-documents menu.
15. ~~Window subtitle detail.~~ Done: encoding · size · language ·
    problems, same as the Mac.
16. **Save preprocessors.** Done in behavior: the shared config
    section drives the same stdin→stdout chains ({path}/{filename}
    placeholders included) before every save, with a
    save-without-preprocessing escape dialog and Run Save
    Preprocessors (Ctrl+Alt+F). Still config-file-only — Preferences
    has no editing UI for the chains yet.
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
