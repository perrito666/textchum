# Search

Two ways to find things beyond the current file, sharing one rule:
**the scope is a visible, editable path.** Both panels show exactly where
they look — the current document's project by default — and widening the
search is literally editing that path (up to `~` or `/` if you like).
Search never silently looks somewhere you did not expect.

Being a path, the scope completes like one on macOS: typing forward
offers the folders that finish the component under the caret, `~`
included. A scope that is not there says so, and names the nearest
folder that is — usually one typo away from the one that was meant.
Blanks around a pasted path are ignored.

Both walks are gitignore-aware, skip hidden files, and cap file sizes,
courtesy of ripgrep's own engine embedded in the core — not a subprocess.

## Search Everywhere (⇧⌘A)

One box for files, symbols, commands and settings (Ctrl+Shift+A on
Linux). What is typed is matched against all four at once, and each
kind of answer comes in a group of its own, the group with the best
match first: typing `save` puts the Save commands above a `save.md`,
typing `main` puts `main.rs` above a command that mentions it. A
group shows a few rows, enough to choose from with the next group still
in view. Files are the project's, matched by name first; symbols are
the language server's, and arrive when it answers; commands are the
menus'; settings are the editor's on/off switches, each listed with
whether it is on. With nothing typed it lists the files opened lately
and the commands run lately. The selected file or symbol shows its code
beside the list. ⏎ opens a file or a symbol, runs a command, and flips a
setting — saved, and in effect at once, as if changed in Settings.


Type fragments of a file name — `editwc` finds
`EditorWindowController.swift` — with fuzzy matching and ranking. ↑/↓ move the selection, ⏎ opens (fronting the window if the
file is already open), ⎋ closes. An empty query browses the scope
alphabetically. The same stacked filters as Find in Project apply
here too — every filter kind refines the matched paths — and the
status line says how many matches the filters pruned.

**⏎ opens the selected row once the rows answer what was typed**; while
a search is still owed — the query changed a moment ago — it searches
instead, so refining a query never opens a file by accident. ⌘⏎
(Ctrl+⏎ on Linux) opens regardless. ↑/↓ move the selection, ⎋ closes,
and a double-click opens too. The status strip spells this out. The
panel can be widened, and keeps its size; in a row the directory gives
way first, losing its head, so the file name is always readable.

Under the list, the selected row's place is shown: a file's first
lines in Open Quickly, a match's line among the lines around it in
Find in Project, a symbol's declaration in Go to Symbol — read from
the open document when the file is open, which may be ahead of the
disk. The divider between the two can be dragged.

The scope is walked once when the panel opens, then matched in memory
— typing stays instant on a real repository, and the file count in the
status line tells you what is being searched. A scope with more files
than the walk limit says so, so "narrow the scope" is advice rather
than a guess.

## Find in Project (⇧⌘F)

The query is the text to find, exactly as typed — what gets pasted into
a search is code, and code is full of `(`, `.` and `[`. The **`.*`**
toggle beside the query reads it as a regular expression instead, and
stays as you left it until the editor quits. Results stream in as
`path:line: text`. ⏎ jumps straight to the matching line. Results are
capped (200) to stay instant; refine the pattern rather than scrolling.

Case follows the **smart-case** rule ripgrep made familiar: an all
lowercase query matches any case, while a query containing an uppercase
letter is matched exactly. So `todo` finds `TODO`, and `TODO` finds
only `TODO`.

A line under the results says what the search did — "18 matches in 4
files · 812 searched", "No matches in 812 files searched", or the
reason nothing could be searched at all (a scope that does not exist,
one where everything is ignored, or a bad pattern, quoted). An empty
result is never mute, so a mistyped pattern or a wrong scope announces
itself instead of looking like an absence of matches.

## Stacked filters

Under the Find in Project query, **＋ Add Filter** stacks refinements:

- **line contains / line excludes** — the matching line's text;
- **file contains / file excludes** — the hit's file path.

Filters are case-insensitive substrings and combine with *and*: lines
with `foo` where `bar` also appears, but not in files with `test` in the
name, is the query `foo` plus `line contains bar` plus
`file excludes test`. File excludes prune whole files before they are
even opened, so filtered searches stay as fast as plain ones.

## The selected word

Selecting a whole word marks the other places it appears in the
visible text — a grey tint, distinct from the selection and from the
find bar's matches, so the three stay apart. Selecting part of a word,
or a stretch spanning several, marks nothing: those selections were
made for some other reason.

⎋ puts the marks away and leaves the selection, so the word is still
there to act on. Editing or selecting elsewhere asks the question
again. The search covers what is on screen, which is what a mark can
be seen on, so a long file costs what a short one does.

Two settings in Settings ▸ Editor decide what counts: whether `Item`
is `item`, and whether `item` inside `items` is one. Both are off by
default — a name that differs in case is a different name, and the
word was selected as a word.

## The jump stack

Every jump — a search result, Jump to Definition, a reference, an
outline entry, a `chum` open, Go to Line, the next finding — records
where it left from. So do Find Next and Go to Block Start/End when
where they go is out of sight; a step to a match already on screen is
not a jump. **Go Back** (⌃⌘←) walks those origins; **Go Forward**
(⌃⌘→) retraces. The history is rewritten from the current point:
jumping somewhere new discards the forward trail. Positions survive
edits as line/column, so going back after a change lands close, not
lost.

**Go to Last Edit** (⇧⌘⌫, Ctrl+Shift+Backspace on Linux) goes to where
something was last changed, in whichever open file that was; pressed
again with the caret still there, it goes to the change before, and so
back. A burst of typing on a few lines is one place, loading or
reloading a file is none, and the places move with the text as it is
edited around them. Each is a jump, so Go Back returns. The trail is
kept for the session: a file closed takes its places with it.

**Recent Files** (⌘E, Ctrl+E on Linux) lists the files opened lately in
the project in front, most recent first, without the one in front: ⌘E
and Return goes back to the file before. **Recent Places** (⌥⌘E,
Ctrl+Alt+Shift+E on Linux) lists where the caret has been — the places
jumps left from and the places edited, in any file — most recent first,
one entry per place, each with the code around it beside the list.
Both filter as you type. Places, like the edit trail, last the session.

A jump to a line already on screen leaves the view where it is. One
out of sight moves the view, in a short animation, so the line sits a
third of the way down, with what comes before it in view; a far jump
starts the motion a screen short of the target, so it shows which way
it went without sweeping through the whole file. With reduced motion
asked for in the system settings, the view moves at once.

## Not there yet

- Search Everywhere lists only the settings that are on or off; a tab
  width or a theme is chosen in Settings. Two taps of Shift do not open
  it.
- Replace across files.
- Explicit case/whole-word toggles in the panel (smart case covers the
  common case, and the pattern itself can express both).
- Persisted search history.
- Editing the scope in the GTK shell, whose panels search the current
  project only; path completion comes with it.
