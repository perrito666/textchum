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
    "tab_width": 4,
    "hover": false
  }
}
```

`appearance` accepts `"system"`, `"light"`, or `"dark"`; omitting it (the
default) follows the system. `editor.hover` switches the mouse-rest
documentation popover off (`true`, the default, keeps it on). `editor.hover` switches the mouse-rest
documentation popover off (`true`, the default, keeps it on).

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
Seven ship built in: **Textchum** (the default), **Textchum High
Contrast**, **Graphite** (a muted near-monochrome), and the classics —
**Molokai**, **Solarized**, **Dracula**, and **Gruvbox**. Every theme
carries a light and a dark palette in one file, so one theme serves
both appearance modes (the dark-born classics pair their canonical
palette with a contrast-adjusted light one; Solarized and Gruvbox use
their genuine light palettes).

User themes are JSON files in:

```
~/Library/Application Support/Textchum/themes/
```

selected by file name (without `.json`); a file named after a built-in
overrides it. **Textchum → Open Themes Folder** opens (and creates)
this directory. The fastest way to start one is to generate a complete
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

## Save preprocessors

Formatters and fixers can run automatically before every save, per
language — for every project or for one root, exactly like language
servers. Each entry is a chain: one command per line, run in order,
where every command reads the document on standard input and writes
the whole document back on standard output (the `-` convention most
formatters follow). If a link in the chain fails — non-zero exit,
empty output, or a hang past ten seconds — nothing is applied, the
error (with the tool's stderr) is shown, and the save asks whether to
proceed unprocessed.

```json
{
  "preprocessors": {
    "defaults": {
      "python": ["ruff check --fix -", "black -"],
      "go": ["gofmt"]
    },
    "projects": {
      "/work/site": { "javascript": ["prettier --stdin-filepath {filename}"] }
    }
  }
}
```

`{path}` and `{filename}` anywhere in a command expand to the
document's absolute path and bare name — for tools that read stdin but
infer their behavior from the name, like Prettier's `--stdin-filepath`.
An untitled document offers `Untitled` plus its language's extension.

A project entry replaces the default chain for that language, never
appends to it. The Settings window edits the same section under
Language Servers, and **Edit ▸ Run Save Preprocessors** (⌃⌥⌘F, action
name `runPreprocessors`) runs the chain on demand without saving —
formatting through your tools instead of the language server's
formatter. The result lands as one edit, so ⌘Z undoes it.

## Spell checking

Prose gets the system spell checker — the same dictionaries every Mac
app shares — scoped to where prose actually lives: comments in code,
and the whole document in Markdown, git commit messages, and plain
text. Identifiers and string literals are never flagged. Misspellings
carry a purple tint, distinct from the red/orange/blue of diagnostics.

Pick the language in Settings ▸ General ▸ "Spell check prose" — Off
(the default), Automatic by content, or a specific dictionary — or set
`editor.spell` by hand: `"auto"` or a spelling identifier like
`"en_US"` or `"es"`. The dictionaries available are the ones enabled
in System Settings ▸ Keyboard ▸ Text Input (macOS ships dozens; add
more there and they appear in the picker).

```json
{ "editor": { "spell": "auto" } }
```

On Linux the same setting rides hunspell: install `hunspell` plus a
dictionary package (`hunspell-es`, `hunspell-en-us`, …) and the marks
appear; `"auto"` follows `$LANG`.

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
`findReferences`, `renameSymbol`, `formatDocument`, `runPreprocessors`,
`documentOutline`,
`goBack`, `goForward`,
`goToBlockStart`, `goToBlockEnd`, `toggleNavigator`, `togglePreview`,
`toggleLineNumbers`, `toggleHover`, `showHover`, `togglePathDisplay`,
`redraw`, `commandPalette`, `serverStatus`,
`settings` — an unknown name is
logged with the full list. And when a shortcut escapes memory entirely,
the **Command Palette** (⇧⌘P) fuzzy-searches every menu action by name
and runs the selection. Go to Block Start/End (⌃⌥↑/⌃⌥↓ by default) jump over the
innermost multi-line syntax block around the caret, courtesy of the
same tree that powers highlighting.

## Live reload

The file is watched while Textchum runs: edit `config.json` in another
editor and the change applies the moment it lands — appearance, theme,
fonts, key bindings, server table, everything, including the open
Settings window. The app's own saves are recognized and ignored, and a
file that momentarily fails to parse falls back to defaults without
being overwritten, exactly like at launch.

## Per-project editor settings

A project root can override the editor's font family, font size, and
tab width for every window inside it — the Projects tab's rows carry
the three fields (empty means "inherit the general value"), and the
file spells it as an `editor` object on the workspace entry:

```json
{
  "workspace": {
    "projects": {
      "/work/legacy": { "editor": { "tab_width": 8, "font_size": 12 } }
    }
  }
}
```

## Not there yet

- Nothing at the moment — file an itch when one appears.
