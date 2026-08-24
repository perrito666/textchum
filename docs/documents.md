# Documents

A *buffer* is raw text; a *document* is a buffer plus everything that makes
it a file: an undo history, a dirty flag, a path, and an encoding. Editor
windows always work with documents.

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

## Not there yet

- Detecting external changes to open files (edit the file elsewhere and
  Textchum will not notice yet).
- Encodings beyond UTF-8 and Latin-1.
- Reopening windows and documents from the previous session.
