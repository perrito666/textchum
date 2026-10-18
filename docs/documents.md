# Documents

A *buffer* is raw text; a *document* is a buffer plus everything that makes
it a file: an undo history, a dirty flag, a path, and an encoding. Editor
windows always work with documents.

## New files

**⌘N** opens an untitled plain-text buffer; **File → New with Format**
lists every known language, so a new file can be highlighted before it
has a name (the language stays until the first save names the file,
which re-detects from the extension as usual). Save As for an untitled
document starts in the folder of whatever file was frontmost when it
was created — a new file usually belongs to the project you were just
in — and suggests the chosen language's extension.

## Typing

Return auto-indents: the new line inherits the current line's leading
whitespace, and goes one level deeper when the line ends (before the
caret) with an opener — `{`, `[`, `(`, or a `:`. The extra level speaks
the document's own dialect: tabs in a tab-indented file, spaces at the
configured tab width otherwise. A line with nothing to inherit gets a
plain newline, so the feature is invisible until it helps.

## Undo and redo

The undo history lives in the core, not in AppKit's `NSUndoManager`. Every
edit is recorded as an invertible operation; undo pops the newest record,
applies its inverse, and reports the resulting change to the window, which
replays it on screen. Because the history sits behind the same interface as
everything else, it can never miss an edit — there is no second path into
the text.

Records coalesce so undo moves in human-sized steps:

- **Typing runs** merge: consecutive insertions, each starting exactly where
  the previous one ended, become one undo step.
- **Deletion runs** merge the same way, for both backspacing and forward
  deleting.
- A **newline** ends a run on either side, so undo stops at line
  granularity.
- **Moving the caret** (click, arrow keys) ends the current run; the next
  keystroke starts a fresh step.

Compound operations record as explicit groups: Replace All rewrites every
match but undoes as a single step, and a reload from disk (below) is one
step too.

A rename or a code action from a language server is one step in each
file it changed. When it changed more than one, Undo in any of them
takes the whole of it back: in every open document whose newest step it
still is, and in the files that were not open, which the editor wrote
itself and kept a copy of. Redo makes it again everywhere. A file
changed since — by hand, or by another program — is left as it is, and
the notification area says which, along with how many files were
undone. This holds for the session: after a restart each document's own
history starts afresh.

## Several carets

**Add Next Occurrence** (⌃G, Alt+J on Linux) selects the word at the
caret; pressed again, it adds the next match of what is selected as a
selection of its own, round the end of the file — whole words only when
the selection is a whole word. **Add Caret Above** and **Below** (⌃⇧↑ and
⌃⇧↓, Alt+Shift+Up and Down on Linux) add a caret on the next line up or
down, in the first caret's column or at the line's end when it is
shorter. All three are in the Edit menu and rebindable as
`addNextOccurrence`, `addCaretAbove` and `addCaretBelow`.

With several carets up, typing, pasting, Backspace, Delete, Return and
Tab happen at every one, as one undo step; the arrows, Home and End move
every caret, and with Shift extend every selection. Carets that meet
become one. Escape, once nothing else is up for it to close, leaves only
the newest caret; so does a click, or any other command, which is about
one caret.

Not there yet: a column of carets is made one line at a time, not by
dragging; Return and Tab at several carets insert a plain line break and
tab, without the indentation one caret gets; word moves and other
commands leave the newest caret only; and on Linux an input method's
composed text goes to the newest caret only.

## Folding

**Fold** (Ctrl+[) collapses the block the caret's line opens; the line
that says what the block is stays, and everything after it up to the
end of the block is hidden. **Fold All** (Ctrl+Alt+[) does every block,
outermost first, and **Unfold All** (Ctrl+]) gives them all back. The
blocks come from the same tree the colouring uses.

One fold per opening line, and the widest when several blocks start
there — `impl Item {` opens both the impl and its body, and folding the
impl is what was meant. A block that would hide a single line is not
offered: in a brace language that line is the closing brace, and
elsewhere it saves one line in exchange for an affordance on every
other one.

Folding changes what you see and nothing else. The text is untouched,
so what gets saved is what was there.

**On macOS this is not available yet.** TextKit 2 lays out one line per
text element and has no attribute that says to skip one: an element
handed back empty still takes a line, and an element withheld from the
enumeration is laid out correctly by a bare layout manager but renders
as nothing through the view's viewport controller. Both were measured
rather than assumed.

## Transformations

**Edit ▸ Transform** acts on the selection, or on the whole document
when nothing is selected — which is what the operation is about when no
part of it was singled out:

- Upper, lower, title and inverted case.
- Sort lines, sort lines reversed, remove duplicate lines.
- Join lines, trim trailing whitespace.
- Convert line endings to LF or CRLF.

An operation over lines is given whole lines: a selection that starts
mid-line grows to the boundaries around it first, because sorting half
a line is not something anyone asked for. Text that came in with CRLF
goes out with CRLF, unless the conversion is the point. The transformed
stretch stays selected, so a second one can follow without selecting it
again, and the whole thing is one undo step.

Title case starts each word and lowers the rest of it, with an
apostrophe counting as part of the word: `don't be well-known` becomes
`Don't Be Well-Known`. Removing duplicates keeps the first of each and
leaves the order alone. Joining drops each line's own indentation,
which was there to sit under the line above.

## Find and replace

**⌘F** opens the native find bar (**⌥⌘F** with the replace field, **⌘G** /
**⇧⌘G** next and previous match, **⌘E** searches the selection). The bar's
options menu offers substring, whole-word, and **regular expression**
matching. Replacements are ordinary edits: they flow through the core,
land in the undo history — Replace All as one step — and mark the document
dirty like typing would.

## External changes

Textchum watches each open document's file. If another program changes it:

- a **clean** document follows the disk silently — the window simply shows
  the new content;
- a **dirty** document asks: keep your unsaved changes, or reload from
  disk. Reloading discards the buffer in favor of the file, but the reload
  is itself one undo step, so ⌘Z brings your version back (and marks the
  document dirty again, as it then differs from disk).

A file that disappears from disk is left alone: the buffer stays, and
saving recreates the file.

**Revert to Saved** (File menu, ⌥⌘R, rebindable as `revertToSaved`)
is the manual version of the same reload: throw the buffer away and
take the disk's word for it, with one confirmation when there are
unsaved changes (and one Undo to take it back). It exists for the rare
external change the watcher misses — delete-and-replace flows like a
git checkout.

## Local history

Every save keeps the version it wrote, apart from the file, and the
first save of a file also keeps the version it replaced — so a save
that should not have happened, a formatter that went too far, or work
thrown away by a checkout can be brought back. **File → Local
History…** (rebindable as `showLocalHistory`) lists the versions kept
of the file in front, newest first, each with the lines restoring it
would add and remove; the selected one is shown beside the list as the
difference from the text now. Choosing one puts it in the editor as one
edit: Undo takes it back, and saving keeps it, as a new version.

Versions live in a `history` folder among the editor's own data
(`~/Library/Application Support/Textchum/history` on macOS,
`~/.local/share/textchum/history` on Linux, or under `--data-dir`), one
folder per file. A version the same as the newest is not kept twice. Up
to 100 versions are kept per file, for 30 days; a file over 4 MB is not
kept.

Not there yet: only saves are kept, not the text between them; a
deleted file's versions cannot be opened from the editor, though they
stay in the folder until their 30 days are up; and how long they are
kept is not a setting.

## Dirty state

A document knows the exact point in its history where it was last saved, so
*dirty* means "the current state differs from the saved one" — not "an edit
happened at some point". Editing and then undoing back to the save point
leaves a clean document, and the window's close button loses its dot
accordingly. If new edits make the saved state unreachable (you undid past
it and typed something else), the document counts as dirty until the next
save, as it must.

Closing a dirty window, or quitting with dirty windows open, asks the usual
question: save, don't save, or cancel.

## Files and encodings

Textchum decodes on open and re-encodes on save:

- Valid **UTF-8** loads as UTF-8. A leading BOM is stripped in memory,
  remembered, and written back on save.
- Anything else is decoded as **ISO-8859-1** (Latin-1), which maps every
  byte to a character and therefore cannot fail. Saves re-encode to Latin-1;
  if an edit introduced characters Latin-1 cannot hold, the save silently
  promotes the file to UTF-8 — nothing can be lost in that direction — and
  the window's subtitle reflects the new encoding.

Line endings are never normalized: what was read is what is written, whether
that is `\n` or `\r\n`.

The current encoding is always visible in the window subtitle, next to the
document size.

## Saves are atomic

A save writes the whole document to a temporary file in the target's
directory, flushes it, and renames it over the target. A crash mid-save can
never leave a truncated file, and other programs watching the file see the
old content or the new — never a mixture.

## Session restore

Relaunching Textchum reopens the files you had open, each with its caret
and scroll position, fronting the one you were in. The state is a plain
JSON file (`session.json`, next to the configuration), written
continuously — not just at quit — so a crash loses at most a moment of
position, never the file list. Files that no longer exist are skipped.

To start without memory (handy when chasing a bug): launch with
`--fresh`, hold ⇧ while the app starts, or delete `session.json` —
any of the three is a complete reset.

Closing a tab is not final either: **Reopen Closed Tab**
(⇧⌘T on macOS, Ctrl+Shift+T on Linux) brings back the last one, caret
included, and repeats back through the recent ones. Only saved
documents are remembered — an untitled buffer has nothing to reopen
from, and reopening it empty would be a lie.

## Not there yet

- Encodings beyond UTF-8 and Latin-1.
