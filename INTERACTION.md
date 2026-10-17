# How Textchum behaves

These are the rules every piece of the interface follows, in both
shells. They exist because the bugs that made the editor feel wrong
were never in what a feature did: they were a balloon that swallowed
a click, a balloon that took the keyboard, an answer shown after the
user had moved on, a list where Return did nothing. Each feature had
been built on its own, and each got those things wrong in its own way.

The rules are numbered so an issue or a review can point at one
("breaks 1.2"). A change that has to break a rule says so in its
issue and argues for it there; a rule that turns out wrong is changed
here first, not quietly worked around.

## 1. The keyboard belongs to the text

**1.1 Nothing takes the keyboard that was not asked for.** Hover
balloons, a finding's message, the signature of a call, the
completion list, inlay hints, notices, the chord window: none of them
ever becomes key or first responder. They are windows or views that
cannot take focus, not windows that hand focus back afterwards —
handing it back is a race, and it was lost.

**1.2 What a command opens for typing gives the keyboard back to the
same place.** A picker, the find bar, the rename field take the
keyboard because the user asked for them. When they close — by
Return, by Escape, by a click elsewhere — the keyboard returns to the
same view with the same caret and selection it had.

**1.3 Asking again goes inside.** Pressing the command that showed a
passive thing while it is up moves the keyboard into it: Show
Documentation pressed with the documentation already showing lets it
be scrolled and selected with the keyboard, and Escape comes back out.

**1.4 A press reaches what it was aimed at.** Nothing that closes
when the text is clicked consumes that click. The click closes it and
lands, so a double-click on a word with a balloon over it selects the
word.

## 2. Escape backs out one level

**2.1** Each Escape closes the topmost transient thing and nothing
else, in this order: the completion list; a balloon; the signature;
the marks of a finished search; extra carets. With none of those up,
Escape does nothing.

**2.2** Escape never loses work, never closes a document or a window,
and never moves the caret.

## 3. Lists open on the first choice

**3.1** Every list opens with its first choosable row selected, and
Return takes it — whether anything was typed or not.

**3.2** The arrows move the selection from the filter field, without
leaving it; the selection is always scrolled into view.

**3.3** Typing filters every list, including ones that show no filter
field; the match is fuzzy and the best match comes first.

**3.4** A double-click is Return. Return in a list never reaches the
text behind it.

**3.5** A list of places shows the place: the selected row's code is
previewed beside the list, so the right one can be told before going
there.

## 4. Passive help waits, and stays out of the way

**4.1** Hover help appears only when the pointer has rested on a
symbol or a finding, never over blank space, punctuation or comments.

**4.2** It never opens under the pointer, never covers the caret's
line, and never covers what is being typed.

**4.3** It stays while the pointer is on the text it is about or
inside it, so it can be reached, scrolled and copied from, and closes
a beat after the pointer leaves both. It appears once: it does not
open and close in a loop while the pointer moves along a word.

**4.4** It closes the moment the text is pressed in or typed into: it
was about the text as it was.

## 5. An answer is about a moment

**5.1** Every request to a server or a tool remembers what it asked
about: the document version, the caret, the word. An answer that
arrives after the user has moved past that is dropped, not shown.

**5.2** An answer never moves the text the caret is in while the user
is typing, and never scrolls the view. Anything that changes layout —
inlay hints, a folded region, an inserted import — is applied when
typing pauses, and what was there stays until its replacement is in:
a hint does not vanish and come back between keystrokes.

**5.3** The editor never waits for an answer. The only exception is a
save chain the user configured to run before writing, and that has a
time limit and says when it was hit.

**5.4** Work in the background says what it is doing where the user
already looks — the status bar — and stops saying it when it is done.

## 6. A failure explains itself, without a dialog

**6.1** A command that has nothing to do says so next to the caret, in
a short hint that goes away at the next key or move: "No usages
found", "Put the caret on a name first". Not a beep alone, and not an
alert to be dismissed.

**6.2** A dialog is for a decision that can lose work or cannot be
undone — unsaved changes, reverting, a file changed on disk, a save
chain that failed — and for choosing a file. A report is never a
dialog.

**6.3** The message names what would make it work ("save the file so
a server can attach"), as `AGENTS.md` already says.

## 7. Every jump is remembered

**7.1** Anything that moves the caret to another file, or out of
sight in the same one — a definition, a usage, a search result, the
outline, Go to Line, a symbol, the next finding, a block start or end
— records where it came from. Go Back returns there exactly: caret,
selection and scroll.

**7.2** Edits leave a trail of their own: the last place something was
changed is one command away, and the one before it is the same
command again.

**7.3** A jump lands with context above the target — about a third of
the way down the view — and never at an edge. The view moves there in
a short animation, so the eye can follow where it went; the animation
never delays the next key.

**7.4** The files opened lately and the places visited lately are each
one command away, most recent first.

## 8. One action, one name, everywhere

**8.1** Every command has one name, and that name is the same in the
menu, the command palette, the chord window, the shortcut settings and
the documentation. The palette shows each command's shortcut.

**8.2** Everything the mouse can do has a command, and every command
is in a menu.

**8.3** Both shells do the same thing under the same name. What
differs is the platform's own: Command on macOS where Linux has Ctrl.

## 9. One undo per intention

**9.1** Typing a run of text is one step; accepting a completion and
the import it brought is one step; a rename or a code action is one
step in each document it touched.

**9.2** Undo puts the caret and selection back where the edit was, and
scrolls there if it is out of sight.

**9.3** What the editor does on its own — formatting before a save, a
server's edit — is never merged into a step the user typed.

## 10. Selection is predictable

**10.1** A selection extended with the keyboard keeps its anchor: the
opposite arrow gives back what it took.

**10.2** A double-click selects a name as the language spells it,
underscores included; a triple-click selects the line.

**10.3** Selection can grow and shrink by the code's own structure —
a name, its expression, the statement, the block — one step at a
time.

**10.4** Text that is drawn but is not in the document — inlay hints —
cannot be selected, and the caret steps over it.

## 11. Never slower than typing

**11.1** A keystroke is on screen in the next frame. Anything that
costs more is cut to the viewport, deferred to a pause, or moved off
the main thread. `TEXTCHUM_DEBUG_TIMERS` is how a change shows it kept
to this.

**11.2** Nothing walks the whole document on a keystroke.

## 12. What can be done is shown

**12.1** A line where the server has a fix or a refactoring to offer
shows a mark in the gutter; it does not have to be guessed at.

**12.2** The next and the previous finding are one command each. They
wrap around the document, and landing on a finding shows its message
as passive help.

**12.3** The completion list shows each item's type, the
documentation of the selected one beside the list, and the import an
item will add. Items are ranked by how well they match what was typed,
then by the server's order, then by what was chosen before.

## 13. Asking again starts where it was left

**13.1** A picker opens with what was typed into it last this session
already in its field, selected: the first key typed replaces it, and
Return runs it again. The command palette, Open Quickly, Find in
Project, Go to Symbol, Go to Line — each remembers its own.

**13.2** With nothing typed, what was chosen recently comes first,
marked as recent: the last commands run in the palette, the last files
opened in Open Quickly, the last symbols jumped to. What is offered
for one place only — the code actions at the caret — has no recent
list, since last time's place is not this one.

**13.3** The options a dialog was left with are the options it opens
with: match case, whole word and regular expression in the find bar
and in Find in Project, the scope and the filters of a search, the
format a new document was last given.

**13.4** A file dialog starts where the work is. Open starts in the
folder last opened from in this project, or the project's root, or
the folder of the document in front; Save As starts in the document's
own folder, or for a document never saved, in the folder last saved
into from this project. Never the home folder while a project is open.

**13.5** A query lasts the session. What describes how the user works
rather than what they were looking for — recent commands and files,
the options of 13.3, the folders of 13.4 — is kept in the project's
record and survives a restart; with no project, in the editor's own.

**13.6** A memory is a start, never a constraint. A remembered query is
selected so it is replaced as typing begins; a remembered folder is
where the dialog opens, not where it is kept; and nothing remembered
is ever acted on without the user pressing something.

## Checking a change against these

A change to the interface says which rules it touches, and its smoke
coverage checks them: who has the keyboard after the thing closes,
what Return and Escape do, whether a stale answer is dropped. Some of
it can only be seen with real input — a click while a balloon is up, a
double-click, a pointer that rests — and `make feel` checks that on
the running app, on macOS: it drives `dist/Textchum.app` on a fixture
with the scripted server and reports each rule it reads back as kept,
broken, or known to be broken until its issue lands. What it does not
check is looked at on the running app before the change is called
done.

## Where the code stands

What breaks these rules today, each to be fixed under its own issue:

- **6.2** A few reports that run to several paragraphs are still
  dialogs on macOS: the result of installing the `chum` command, of
  installing a grammar and of importing a theme, and the note that
  Universal Ctags is missing. They wait for a notice that can carry
  details.
- **3.5** Open Quickly, Find in Project and Go to Symbol show no
  preview of the place a row goes to.
- **7.1** Find Next and Go to Block Start/End move the caret without
  recording the jump.
- **7.2, 7.3, 7.4** There is no last-edit command, no recent places
  list, and jumps land without animation, often at an edge.
- **10.3** Selection does not grow or shrink by structure.
- **12.1, 12.2, 12.3** No gutter mark for fixes, no next/previous
  finding command, completion ranked by the server's order alone and
  without documentation beside it.
- **13.2** Go to Symbol lists no recent symbols with nothing typed.
- **13.4** Open starts wherever the system last left its dialog, not in
  the project; on Linux it starts with no folder at all. Save As
  already remembers the folder an untitled document was last saved
  into, but app-wide, not per project.
- **13.3** The find bar's and Find in Project's toggles, the search
  filters and the last format of a new document are not remembered.
- **2.1** On Linux, Escape in the text does not close the find bar,
  which belongs to the window rather than the document.
- **Linux** Inlay hints are gathered after the line, not inside it
  (see `PLAN_linux.md`).
