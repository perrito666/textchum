# Configuration

Textchum's settings follow one principle: **the GUI is the comfortable way
to change them, and a plain JSON file is the always-available escape
hatch.** There is exactly one store — the file — and the Settings window
reads and writes it; nothing lives only inside the app.

## The Settings window

**Textchum → Settings…** (⌘,) edits the recognized settings:

- **Appearance** — follow the system (switching live when macOS does), or
  force light or dark.
- **Open files in** — tabs of the current window (the default) or
  separate windows. With separate windows, each window's navigator lists
  only its own tab group's documents.
- **Font** — any fixed-pitch family installed on the system, or the
  platform's monospaced font.
- **Font size** — 6 to 72 points.
- **Tab width** — 1 to 16 columns.

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

## Not there yet

- Textchum does not yet watch the file while running; changes made in
  another editor apply on the next launch.
- Per-project overrides.
