# Configuration

Textchum's settings follow one principle: **the GUI is the comfortable way
to change them, and a plain JSON file is the always-available escape
hatch.** There is exactly one store — the file — and the Settings window
reads and writes it; nothing lives only inside the app.

## The Settings window

**Textchum → Settings…** (⌘,) edits the recognized settings:

- **Appearance** — follow the system (switching live when macOS does), or
  force light or dark.
- **Theme** — the syntax palette; see [Themes](#themes) below.
- **Open files in** — tabs of the current window (the default) or
  separate windows. With separate windows, each window's navigator lists
  only its own tab group's documents.
- **Font** — any fixed-pitch family installed on the system, or the
  platform's monospaced font.
- **Font size** — 6 to 72 points.
- **Tab width** — 1 to 16 columns.
- **Show line numbers** — the gutter, also togglable per session with
  View → Toggle Line Numbers (⇧⌘L).

Every change is applied to open editor windows immediately and written to
disk at the same moment. There is no Apply or Save button to forget.

## The file

Settings live in:

```
~/Library/Application Support/Textchum/config.json
```

A file edited by hand might look like:

```json
{
  "appearance": "dark",
  "editor": {
    "font_family": "JetBrains Mono",
    "font_size": 13,
    "tab_width": 4
  }
}
```

`appearance` accepts `"system"`, `"light"`, or `"dark"`; omitting it (the
default) follows the system.

Everything is optional — a missing file, a missing section, or a missing
key simply means the default. Writes are atomic (temporary file plus
rename), like every write Textchum does.

Two guarantees make hand editing safe:

- **Unknown keys survive.** The settings window rewrites only the keys it
  owns. Anything else in the file — your annotations, keys from a newer
  version — is preserved verbatim on every save.
- **Broken files are never clobbered.** If the file fails to parse,
  Textchum starts with default settings, tells you once at launch, and
  leaves the file exactly as you wrote it so you can fix it in any editor —
  including Textchum itself. Should you change a setting from the GUI while
  the file is broken, the unparseable original is first copied to
  `config.json.bak`, then replaced.

Out-of-range or mistyped values do not count as breakage: a `font_size` of
`4000` is clamped to the valid range, a `font_family` of `42` is ignored,
and the rest of the file works normally.

## Themes

The **Theme** picker in the General tab selects the syntax palette.
Three ship built in — **Textchum** (the default), **Textchum High
Contrast**, and **Graphite**, a muted near-monochrome. Every theme
carries a light and a dark palette in one file, so one theme serves
both appearance modes.

User themes are JSON files in:

```
~/Library/Application Support/Textchum/themes/
```

selected by file name (without `.json`); a file named after a built-in
overrides it. The fastest way to start one is to generate a complete
starter — every styled capture name, filled with the default palette —
and just change colors:

```bash
Textchum --emit-theme ~/Library/Application\ Support/Textchum/themes/Mine.json
```

Entries map tree-sitter capture names to styles:

```json
{
  "name": "Mine",
  "styles": {
    "keyword": {"light": "#AD3DA4", "dark": "#FC5FA3", "bold": true},
    "comment": {"light": "#707F8C", "dark": "#7F8C98", "italic": true}
  }
}
```

Colors are `#RRGGBB` or `#RRGGBBAA`. Anything omitted — a color, a
flag, a whole capture — keeps the default palette's value, so a theme
only needs to say what it changes. The escape-hatch rules match the
configuration's: a theme that fails to parse falls back to the default
with one warning and is never overwritten, and unknown keys survive.
Theme files are read at launch and when the selection changes.

## Projects

The Projects tab decides where a project starts and ends — the boundary
the navigator groups by and the language-server pool keys its instances
on. Both switches exist twice: as a default for every project, and per
project root. A row added with the path field (which completes directory
names as you type, and carries a Browse… button) overrides the defaults
for that root only.

- **Manifest projects** — normally the outermost repository wins:
  opening a file anywhere inside a repository makes the repository the
  project, however many `Cargo.toml` or `pyproject.toml` files sit in
  between. Switching this on splits a root at language manifests again,
  so nested modules become projects of their own.
- **Recursive config** — makes a root's per-project settings (its
  language-server commands, and these very switches) apply to the nested
  projects inside it, closest ancestor first. Useful for monorepos: one
  configuration at the top, many projects underneath.
- **Ctags fallback** — answers Jump to Definition from a Universal
  Ctags index when no language server is available; see
  [language servers](language-servers.md).

In the file, these live in a `workspace` section:

```json
{
  "workspace": {
    "manifest_projects": false,
    "recursive_config": false,
    "ctags_fallback": false,
    "projects": {
      "/Users/you/code/monorepo": {
        "manifest_projects": true,
        "recursive_config": true
      }
    }
  }
}
```

## Key shortcuts

Menu shortcuts are rebindable through a hand-edited `keys` section (no
UI yet): an object of action names to `modifiers+key` specs, applied at
launch.

```json
{
  "keys": {
    "openQuickly": "cmd+p",
    "goToBlockEnd": "ctrl+alt+down",
    "findInProject": "cmd+shift+g"
  }
}
```

Modifiers: `cmd`, `shift`, `alt`, `ctrl`. Keys: a character, or
`up`/`down`/`left`/`right`/`return`/`escape`/`space`/`tab`/`delete`.
Actions include `new`, `open`, `openQuickly`, `save`, `saveAs`, `close`,
`undo`, `redo`, `find`, `findAndReplace`, `findNext`, `findPrevious`,
`useSelectionForFind`, `findInProject`, `jumpToDefinition`,
`goToBlockStart`, `goToBlockEnd`, `toggleNavigator`, `togglePreview`,
`toggleLineNumbers`, `settings` — an unknown name is logged with the
full list. Go to Block Start/End (⌃⌥↑/⌃⌥↓ by default) jump over the
innermost multi-line syntax block around the caret, courtesy of the
same tree that powers highlighting.

## Not there yet

- Textchum does not yet watch the file while running; changes made in
  another editor apply on the next launch.
- Per-project overrides of the editor settings (font, tab width) —
  projects already carry their own detection and language-server
  settings, but not these.
