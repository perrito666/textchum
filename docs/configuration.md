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
- **File icons** — a VS Code icon pack for the file tree; see
  [File icons](#file-icons) below.
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
documentation popover off (`true`, the default, keeps it on).
`editor.new_files_in` places fresh documents in a `"tab"` of the
frontmost window's group (the default) or a `"window"` of their own.
`editor.mark_occurrences` (`true` by default) marks the other places
the selected word appears; `editor.occurrences_case_sensitive` and
`editor.occurrences_whole_word` decide what counts as one, both `true`
by default.

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

### Somewhere else, for a run

`--data-dir <path>` keeps everything Textchum owns under one directory
for that run — the configuration, themes, icon packs, the session and
the language-server log — instead of the usual places:

```bash
Textchum --data-dir ~/scratch-profile
```

That is a whole profile made for the occasion and thrown away
afterwards, with the real one never opened; `make playground` uses it,
and so does anything else that must not touch your settings. On Linux a
run with its own profile is its own process, since handing the files to
an instance already running would open them in that instance's profile.

`--config <path>` is the narrower version: it points at one
configuration file, and the session follows it.

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

### Importing one from another editor

**Textchum → Import Theme** brings colours over from VS Code or
TextMate. Pick a theme file, or a folder holding several — a VS Code
extension directory (its `package.json` says what it contributes) or a
TextMate bundle (its themes live in `Themes/`). Everything found is
imported, and the first is put on.

Both editors describe colour by **TextMate scope**, so importing is a
matter of translating scopes to Textchum's capture names:
`entity.name.function` becomes `function`, `keyword.control.loop`
becomes `repeat`. Scopes stop where captures keep going — no theme
colours `if` differently from `while` — so a capture the source never
named takes its colour from the one it is a special case of, in either
direction: a theme that says `keyword` colours every kind of keyword,
and one that says only `constant.numeric` colours the whole constant
family.

Two things are worth knowing before the colours look wrong:

- **A theme fills one appearance.** Both editors write a theme for a
  light background or a dark one; Textchum's carry both. An import
  fills the side the source declares and leaves the other at the
  default palette, and says which side it filled. Importing a dark
  theme while the editor is in light appearance changes nothing
  visible.
- **Scopes with nowhere to go are named.** Anything the source coloured
  that no capture answers to is listed after the import. Those colours
  are unused.

The result is an ordinary theme file in the themes folder, editable
like any other.

## File icons

The file tree draws an icon per row. Without a pack that is whatever
the desktop offers for the file's type, which knows Python from
Markdown and stops not much further along — and has never heard of a
file called `Dockerfile`.

**Settings → General → File icons** takes a **VS Code icon pack**. The
packs already seen are on the list, split between the ones imported
here and the ones opened where they lie; *System icons* is the way
back.

**Import…** copies the pack into Textchum's own folder —
`~/Library/Application Support/Textchum/icons/` on macOS,
`~/.local/share/textchum/icons/` on Linux — so moving or deleting the
original cannot take the icons away. **Open…** points at a pack where
it sits and remembers it, which is right for one you maintain
yourself. Either takes the icon theme's JSON file or the extension
folder holding it (its `package.json` says which file). **Delete**
removes an imported pack; a pack opened from elsewhere belongs to
whoever put it there, so it can only be dropped from the list.

The choice is a path in `config.json`, and the packs opened from
elsewhere are remembered beside it:

```json
{
  "icon_pack": "~/packs/material-icon-theme/dist/material-icons.json",
  "icon_packs": ["~/packs/material-icon-theme/dist/material-icons.json"]
}
```

A pack whose folder is gone drops off the list rather than sitting
there to fail when chosen. A pack that cannot be read is reported once
and the tree keeps the system's icons.

Lookup follows VS Code's, most specific first:

1. The whole file name (`Dockerfile`, `cargo.toml`), lowercased.
2. The longest extension that matches: `component.test.ts` tries
   `test.ts` before `ts`.
3. The language Textchum decided the file is — which is also how a
   language set by hand in **File Properties** reaches the icon.
4. The pack's own default.

A pack's `light` section overrides any of those on a light background,
one lookup at a time, so a pack that only redraws a handful keeps the
rest.

Two things are left out. **Folder icons**: the tree draws its own.
**Font-based icons** — the `fontCharacter` definitions Seti and its
descendants use — need the icon font installed and a text run where an
image goes; a pack with nothing else is refused with that as the
reason, rather than loaded to draw nothing.

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

Several dictionaries can apply at once — name them separated by commas.
A word any of them knows is spelled correctly, which is what a document
that switches languages mid-paragraph needs:

```json
{ "editor": { "spell": "en_US, es_ES" } }
```

`editor.spell_words` is your own list: project names, acronyms, and
everything no dictionary ships with. Right-clicking a misspelling
offers replacements, **Add to Dictionary**, which writes the word here,
and **Ignore**, which accepts it until the editor quits. The list is
also editable in Settings.

```json
{ "editor": { "spell_words": ["SBX", "Textchum"] } }
```

On Linux the same settings ride hunspell: install `hunspell` plus a
dictionary package (`hunspell-es`, `hunspell-en-us`, …) and the marks
appear; `"auto"` follows `$LANG`, and the dictionaries hunspell can
find are listed beside the field in Preferences.

## Autosave

Off by default. `editor.autosave` is a number of seconds; the clock
restarts with every keystroke, so the save happens once typing stops
rather than in the middle of a sentence.

```json
{ "editor": { "autosave": 30 } }
```

Two things it deliberately does not do. It never saves a document that
has no name — there is nowhere to put it, and inventing one is not the
editor's decision. And it does not run save preprocessors: a formatter
reflowing the line you are still writing is not a favour, so explicit
saves remain the place for that.

## Key shortcuts

Settings ▸ Keyboard has them: a profile, and every command with the
shortcut it answers to, editable in place.

**Profiles.** People arrive from another editor with its shortcuts in
their fingers, so the three those editors are known for ship with the
build — Visual Studio Code, Sublime Text and IntelliJ IDEA. A profile
names the commands it moves and leaves the rest alone, so picking one
changes what that editor is known for and nothing else. `keys_profile`
holds the choice; empty is Textchum's own bindings.

Changing one shortcut on top of a profile keeps the profile: the change
is an override, and **Reset changes** drops all of them. **Save as
profile** turns what is in force into a profile of your own — the way
to modify a bundled preset, since those ship with the build. Saved
profiles live in `key_profiles`, and one that reuses a bundled name
replaces it.

The file spells the overrides as a `keys` section: an object of action
names to `modifiers+key` specs, applied over the profile.

```json
{
  "keys": {
    "openQuickly": "cmd+p",
    "goToBlockEnd": "ctrl+alt+down",
    "findInProject": "cmd+shift+g"
  }
}
```

Modifiers: `cmd`, `shift`, `alt`, `ctrl` — `cmd` is Command on macOS
and Ctrl on Linux, so one profile means the same thing on both. Keys: a
character, `f1` to `f20`, or
`up`/`down`/`left`/`right`/`return`/`escape`/`space`/`tab`/`delete`.
Actions include `new`, `open`, `openQuickly`, `save`, `saveAs`, `close`,
`undo`, `redo`, `find`, `findAndReplace`, `findNext`, `findPrevious`,
`useSelectionForFind`, `findInProject`, `jumpToDefinition`,
`findReferences`, `codeActions`, `renameSymbol`, `formatDocument`, `runPreprocessors`,
`documentOutline`,
`goBack`, `goForward`,
`blameLine`, `goToLine`,
`goToBlockStart`, `goToBlockEnd`, `toggleNavigator`, `togglePreview`,
`toggleLineNumbers`, `toggleHover`, `showHover`, `togglePathDisplay`,
`redraw`, `commandPalette`, `serverStatus`, `newWithFormat`, `revealInTree`, `reopenClosed`,
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

## Adding a project

The Projects tab lists the roots of the open documents, so a project is
added by picking it rather than by typing its path. **Copy settings
from** starts the new entry from one already configured — its servers,
save commands, flags and editor overrides, all of it — which is what a
second service in the same layout needs. The same choice sits on each
project's own row, for copying onto one that already exists.

An empty override field shows what it inherits, so a blank box says
what applies rather than leaving you to go and look.

A configured root whose directory is gone is marked *missing*, and
**Remove missing** forgets every one of them: nothing will ever match
those entries again.

## Languages the build does not know

Textchum's colouring comes from tree-sitter grammars compiled into the
build. A language it does not carry can be named in `languages`, with
the grammar as a compiled library and its highlights query as a file:

```json
{
  "languages": {
    "dockerfile": {
      "grammar": "~/.local/share/textchum/grammars/libtree-sitter-dockerfile.dylib",
      "highlights": "~/.local/share/textchum/grammars/dockerfile/highlights.scm",
      "extensions": ["dockerfile"],
      "filenames": ["Dockerfile", "Containerfile"]
    }
  }
}
```

`aliases`, `filenames` and `injections` are optional, and so is
`symbol`: the constructor is `tree_sitter_<name>` unless it is named,
with dashes and dots turned into underscores. A grammar built for a
different tree-sitter is refused by its ABI number rather than trusted
and crashed on, and a name the build already knows is replaced by the
configured one — which is how a dated built-in grammar gets fixed
without waiting for a release.

Building one, from a grammar's own repository:

```bash
cc -O2 -fPIC -shared -I src -o libtree-sitter-NAME.dylib src/parser.c src/scanner.c
```

(`.so` on Linux, and drop `src/scanner.c` when the grammar has none.)
An entry that cannot be loaded costs that one language: the editor says
what went wrong and carries on.

## Project records

A file remembers how it is split, where each view was looking, what is
folded, and what it was told it is when its name does not say. That is
data about the file, so it lives with the project instead of in
`config.json`: one record per project root, JSON like everything else.

```json
{
  "version": 1,
  "root": "/work/engine",
  "files": {
    "src/parser.rs": {
      "views": 2,
      "dividers": [0.45],
      "folds": [[12, 48]],
      "language": "rust",
      "places": [{"caret": 812, "scroll": 240.0}]
    }
  }
}
```

Records are kept in the profile, beside the session and the themes, so
a run pointed at a scratch profile writes its own. **Keep each
project's state with the checkout** puts the record at
`<root>/.tchum` instead, for a layout that travels with the clone —
the choice is global, since a per-project answer would have to be
recorded centrally to be found.

The sweep runs at launch on a thread of its own: it forgets the records
of projects that are no longer there, and those not written for longer
than the keep window (90 days by default; zero keeps them until they
are forgotten by hand). **Forget records at launch** turns it off, and
**Manage…** beside the records folder lists what exists, with what each
record is about and when it was last written, to be forgotten one at a
time or in a sweep.

## Not there yet

- Nothing at the moment — file an itch when one appears.
