# Documents

A *buffer* is raw text; a *document* is a buffer plus everything that makes
it a file: an undo history, a dirty flag, a path, and an encoding. Editor
windows always work with documents.

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

## Not there yet

- Encodings beyond UTF-8 and Latin-1.
