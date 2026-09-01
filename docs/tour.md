# A tour

Every screen Textchum has, in the order you would meet them, on both
shells: the macOS app on the left and the GTK one on the right. The
screenshots come from a small fictional project — *Harbor*, a port
broker that exists only so these pictures have something honest to
show — and each follows your own light or dark setting.

The two shells are the same editor over the same core, so the pictures
mostly differ in what the platform contributes: window furniture,
where a panel is drawn, and which font the system hands over.

## The window

One document per window, tabs by default, and a navigator drawer with
two halves: the open buffers grouped by project on top, that project's
file tree below.

<div class="shots" markdown>
<figure markdown>
[![The editor window: sidebar with open buffers and the project tree, a Rust file with syntax highlighting and line numbers (macOS)](images/editor.png#only-light)](images/editor.png)
[![The editor window: sidebar with open buffers and the project tree, a Rust file with syntax highlighting and line numbers (macOS)](images/editor-dark.png#only-dark)](images/editor-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![The editor window: sidebar with open buffers and the project tree, a Rust file with syntax highlighting and line numbers (Linux)](images/editor-gtk.png#only-light)](images/editor-gtk.png)
[![The editor window: sidebar with open buffers and the project tree, a Rust file with syntax highlighting and line numbers (Linux)](images/editor-gtk-dark.png#only-dark)](images/editor-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

The title bar carries the facts about the document — encoding, size,
language, and the problem count once a language server has an opinion.
The tree follows along: switching tabs expands the path to the current
file and highlights it.

Right-clicking in the text opens the editor's own menu: Jump to
Definition, Find References, Rename Symbol, the diagnostics of the
line, Blame Line, Format Document and File Properties, beside cut,
copy, paste and the spelling suggestions. Those commands act on the
character that was clicked rather than on the caret, which the click
leaves where it was. What the document has no use for is not there —
no server running means no Find References, no findings means no
diagnostics rows.

## Folding

**Fold** (⌘[, Ctrl+[ on Linux) closes the block that opens on the
caret's line; **Fold All** (⌥⌘[, Ctrl+Alt+[) closes every block that is
not inside one already closed, and **Unfold All** (⌘], Ctrl+]) opens
them again. A closed block shows its opening line with an ellipsis
after it.

The blocks come from the same tree the colouring uses, and the folds
belong to the document: closing a function in one view closes it in
every view of that file.

## Columns

A window is a row of columns. A column shows one file at a time and
holds one or more views of it, stacked.

**New Column** (⌘\\, Ctrl+\\ on Linux) puts a column beside this one,
showing the same file until it is given another; **Close Column**
(⇧⌘\\, Ctrl+Shift+\\) takes one away. **Second View** (⌥⌘\\,
Ctrl+Alt+\\) stacks another view of the column's file under the first,
and **Close View** (⇧⌥⌘\\, Ctrl+Alt+Shift+\\) removes it. **Next Pane**
(⌥⌘`, Ctrl+Alt+`) moves the keyboard through them.

Each view scrolls on its own, which is the point: reading the top of a
file while editing the bottom of it. A column owns the file it shows,
so changing its tab moves every view in it to the new file.

Both sides are one document. There is one history and one save, so an
edit on either side is the same edit, and neither view can be a stale
copy of the other. Both toolkits are built for this — a text buffer
that several views share — and what does not come free is the
colouring, since on macOS that lives on the layout rather than the
text; each view gets painted.

## Language servers

Diagnostics arrive as tinted marks in the text and a count in the
title bar. Nothing about the editor waits on the server: it attaches
when it can, and says so when it cannot.

<div class="shots" markdown>
<figure markdown>
[![A warning from the language server marked in the text, counted in the title bar (macOS)](images/diagnostics.png#only-light)](images/diagnostics.png)
[![A warning from the language server marked in the text, counted in the title bar (macOS)](images/diagnostics-dark.png#only-dark)](images/diagnostics-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![A warning from the language server marked in the text, counted in the title bar (Linux)](images/diagnostics-gtk.png#only-light)](images/diagnostics-gtk.png)
[![A warning from the language server marked in the text, counted in the title bar (Linux)](images/diagnostics-gtk-dark.png#only-dark)](images/diagnostics-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

Resting the pointer on a symbol shows the server's documentation, with
the Markdown it sends rendered — code blocks monospaced, emphasis
styled. ⌃⌘H asks for the symbol under the caret instead, which works
with mouse hover switched off.

<div class="shots" markdown>
<figure markdown>
[![Hover documentation over a function, showing a rendered signature and prose (macOS)](images/hover.png#only-light)](images/hover.png)
[![Hover documentation over a function, showing a rendered signature and prose (macOS)](images/hover-dark.png#only-dark)](images/hover-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Hover documentation over a function, showing a rendered signature and prose (Linux)](images/hover-gtk.png#only-light)](images/hover-gtk.png)
[![Hover documentation over a function, showing a rendered signature and prose (Linux)](images/hover-gtk-dark.png#only-dark)](images/hover-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Go to Line** (⌘L, Ctrl+L on Linux) takes a number, or the whole
`src/main.rs:412:8` pasted straight out of a build log — the file name
and the trailing noise are ignored, the line is centred, and Go Back
returns to where reading was interrupted.

A **change bar** runs down the left of the gutter, saying which lines
differ from the file as it stands in git: a green stripe for a line
that is new, blue for one that reads differently, and a red wedge on
the boundary where lines were deleted — deleted lines occupy no height,
so a stripe would have nothing to cover. It follows the buffer rather
than the file on disk, so it is right before you save, and it is
recomputed once typing settles. A file with no committed version, or
one outside a repository, gets no marks rather than every line claimed
as new.

The bar compares against the last commit unless told otherwise. Deep in
a feature branch that answer goes quiet — everything committed on the
branch counts as unchanged — so `editor.git_marks: "branch"` compares
against the commit the branch grew from instead, and the bar shows
everything the branch touches. When git does not name a default branch,
`editor.merge_base_branches` lists the names to try, most likely first;
both settings can be overridden per project. **Go → Changed in Branch**
(⌃⌘T, Ctrl+Alt+P on Linux) lists the branch's files — the pull
request's files, read from git alone — behind the same fuzzy filter as
Open Quickly.

**Blame Line** (⌃⌘B, Ctrl+Alt+B on Linux) asks git who last touched the
line under the caret: the commit, the author and when they wrote it,
the subject and the message body — where the reasoning usually is — and
the file's name at the time if it has been renamed since. The commit is
one button away from the clipboard, which is most of what the answer is
for. A line typed since the last commit says so rather than borrowing
somebody else's.

It asks with the buffer's text, not the file on disk, so an unsaved
edit above the caret cannot quietly shift the answer onto the
neighbouring line.

In a line's **leading whitespace**, two keys mean something more than
usual. Backspace deletes back to the previous tab stop rather than one
space at a time, and Tab lines the line up with the nearest non-blank
line above it — pressing it again, once already level, goes one level
deeper. Anywhere else in the line both keys are themselves: it is the
position that decides, not a mode, which is what keeps them from
surprising anyone. A tab-indented line is left to its tab character,
which is already one press per level.

With text selected, typing an opening bracket or quote — `(`, `[`, `{`,
`'`, `"`, `` ` `` — wraps the selection in the pair instead of replacing
it. What was wrapped stays selected, so pressing another one wraps that
in turn: `[`, `(` and `{` over `hello` give `[({hello})]`. Typing
anything else replaces the selection as it always did.

The thin bar under the editor answers what a look at the text cannot:
where the caret is, whether the file indents with tabs or spaces and by
how much, what language it is treated as, and its encoding. The
indentation and the language are clickable and open File Properties,
where those choices are made.

Scrolled deep into a long body, the first line of each enclosing
construct stays pinned at the top of the view — the `class` line and
the `def` line while a Python method scrolls — and clicking a pin goes
to it. The pins cost rows, so `editor.context_lines: false` (or the
switch in Settings) turns them off.

The caret's line carries a faint tint, so the caret can be found in a
long file at a glance. Right-clicking in the text — or **File → Copy
Path** — offers the caret's line in the shapes that get pasted: **Path
and Line** (`path:line`, which a terminal opens straight to) and
**Forge URL for Line**, the file's forge URL with the line fragment
spelled the way that forge does.

Completions appear as you type after identifier characters and `.`;
↑/↓ choose, ⏎ or ⇥ accept, ⎋ dismisses. A snippet arrives with its
first placeholder selected, so typing replaces it; ⇥ moves to the next
placeholder and ⇧⇥ back, and one written twice mirrors as you type.
The last ⇥ leaves the caret where the snippet asked for it and hands
the keys back.

<div class="shots" markdown>
<figure markdown>
[![The completion popup listing members with their types (macOS)](images/completion.png#only-light)](images/completion.png)
[![The completion popup listing members with their types (macOS)](images/completion-dark.png#only-dark)](images/completion-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![The completion popup listing members with their types (Linux)](images/completion-gtk.png#only-light)](images/completion-gtk.png)
[![The completion popup listing members with their types (Linux)](images/completion-gtk-dark.png#only-dark)](images/completion-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**⇧⌘O** lists the file's symbols, filterable from the keyboard.

<div class="shots" markdown>
<figure markdown>
[![The document outline panel, listing a struct and its methods (macOS)](images/outline.png#only-light)](images/outline.png)
[![The document outline panel, listing a struct and its methods (macOS)](images/outline-dark.png#only-dark)](images/outline-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![The document outline panel, listing a struct and its methods (Linux)](images/outline-gtk.png#only-light)](images/outline-gtk.png)
[![The document outline panel, listing a struct and its methods (Linux)](images/outline-gtk-dark.png#only-dark)](images/outline-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**View ▸ Language Server Status** answers "is my server alive?" — what
runs where, and the session's recent transitions, refreshed live.

<div class="shots" markdown>
<figure markdown>
[![The language server status panel listing one running instance and its status transitions (macOS)](images/server-status.png#only-light)](images/server-status.png)
[![The language server status panel listing one running instance and its status transitions (macOS)](images/server-status-dark.png#only-dark)](images/server-status-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![The language server status panel listing one running instance and its status transitions (Linux)](images/server-status-gtk.png#only-light)](images/server-status-gtk.png)
[![The language server status panel listing one running instance and its status transitions (Linux)](images/server-status-gtk-dark.png#only-dark)](images/server-status-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

## Finding things

**⌘T** opens files by fuzzy name within the project. The scope is
walked once and matched in memory, so typing stays instant; the status
strip says how many of how many files matched, and which keys do what
— **⏎ searches, ⌘⏎ opens**, so refining a query never opens a file by
accident.

<div class="shots" markdown>
<figure markdown>
[![Open Quickly: a fuzzy query, one matching path, and the status strip naming the keys (macOS)](images/open-quickly.png#only-light)](images/open-quickly.png)
[![Open Quickly: a fuzzy query, one matching path, and the status strip naming the keys (macOS)](images/open-quickly-dark.png#only-dark)](images/open-quickly-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Open Quickly: a fuzzy query, one matching path, and the status strip naming the keys (Linux)](images/open-quickly-gtk.png#only-light)](images/open-quickly-gtk.png)
[![Open Quickly: a fuzzy query, one matching path, and the status strip naming the keys (Linux)](images/open-quickly-gtk-dark.png#only-dark)](images/open-quickly-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**⇧⌘F** searches contents with a regular expression, with stacked
filters that refine the results by line text or file path. The status
line always says what the search did — matches, files searched, or why
nothing was read.

<div class="shots" markdown>
<figure markdown>
[![Find in Project: regex results with a file filter applied (macOS)](images/find-in-project.png#only-light)](images/find-in-project.png)
[![Find in Project: regex results with a file filter applied (macOS)](images/find-in-project-dark.png#only-dark)](images/find-in-project-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Find in Project: regex results with a file filter applied (Linux)](images/find-in-project-gtk.png#only-light)](images/find-in-project-gtk.png)
[![Find in Project: regex results with a file filter applied (Linux)](images/find-in-project-gtk-dark.png#only-dark)](images/find-in-project-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**⇧⌘P** is the command palette: every menu action, fuzzy-searchable,
with its shortcut alongside.

<div class="shots" markdown>
<figure markdown>
[![The command palette listing menu actions and their shortcuts (macOS)](images/palette.png#only-light)](images/palette.png)
[![The command palette listing menu actions and their shortcuts (macOS)](images/palette-dark.png#only-dark)](images/palette-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![The command palette listing menu actions and their shortcuts (Linux)](images/palette-gtk.png#only-light)](images/palette-gtk.png)
[![The command palette listing menu actions and their shortcuts (Linux)](images/palette-gtk-dark.png#only-dark)](images/palette-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

## Markdown and prose

Markdown documents open with a live preview beside the text, and the
prose spell checker — off until you pick a dictionary — marks
misspellings in purple, distinct from diagnostics. In code it looks
only at comments; identifiers are never flagged.

<div class="shots" markdown>
<figure markdown>
[![A Markdown document with its rendered preview beside it (macOS)](images/preview.png#only-light)](images/preview.png)
[![A Markdown document with its rendered preview beside it (macOS)](images/preview-dark.png#only-dark)](images/preview-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![A Markdown document with its rendered preview beside it (Linux)](images/preview-gtk.png#only-light)](images/preview-gtk.png)
[![A Markdown document with its rendered preview beside it (Linux)](images/preview-gtk-dark.png#only-dark)](images/preview-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

<div class="shots" markdown>
<figure markdown>
[![Misspellings marked in prose, with the rendered preview alongside (macOS)](images/spell-check.png#only-light)](images/spell-check.png)
[![Misspellings marked in prose, with the rendered preview alongside (macOS)](images/spell-check-dark.png#only-dark)](images/spell-check-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Misspellings marked in prose, with the rendered preview alongside (Linux)](images/spell-check-gtk.png#only-light)](images/spell-check-gtk.png)
[![Misspellings marked in prose, with the rendered preview alongside (Linux)](images/spell-check-gtk-dark.png#only-dark)](images/spell-check-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

## Settings

Settings are a plain JSON file that the window edits; the file is the
escape hatch, and it is watched, so an edit in another editor applies
at once.

<div class="shots" markdown>
<figure markdown>
[![Settings, General tab: appearance, theme, placement, font, and the editor toggles (macOS)](images/settings-general.png#only-light)](images/settings-general.png)
[![Settings, General tab: appearance, theme, placement, font, and the editor toggles (macOS)](images/settings-general-dark.png#only-dark)](images/settings-general-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Settings, General tab: appearance, theme, placement, font, and the editor toggles (Linux)](images/settings-general-gtk.png#only-light)](images/settings-general-gtk.png)
[![Settings, General tab: appearance, theme, placement, font, and the editor toggles (Linux)](images/settings-general-gtk-dark.png#only-dark)](images/settings-general-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Projects** decides how project roots are found, what the tree hides,
and which editor settings a root overrides.

<div class="shots" markdown>
<figure markdown>
[![Settings, Projects tab: detection toggles, hide patterns, and per-project overrides (macOS)](images/settings-projects.png#only-light)](images/settings-projects.png)
[![Settings, Projects tab: detection toggles, hide patterns, and per-project overrides (macOS)](images/settings-projects-dark.png#only-dark)](images/settings-projects-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Settings, Projects tab: detection toggles, hide patterns, and per-project overrides (Linux)](images/settings-projects-gtk.png#only-light)](images/settings-projects-gtk.png)
[![Settings, Projects tab: detection toggles, hide patterns, and per-project overrides (Linux)](images/settings-projects-gtk-dark.png#only-dark)](images/settings-projects-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

Hidden names are glob patterns, edited one per line, with a menu that
adds a named preset in one click.

<div class="shots" markdown>
<figure markdown>
[![The hide editor open as a popover, one pattern per line, with the Add preset menu (macOS)](images/hide-globs.png#only-light)](images/hide-globs.png)
[![The hide editor open as a popover, one pattern per line, with the Add preset menu (macOS)](images/hide-globs-dark.png#only-dark)](images/hide-globs-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![The hide editor open as a popover, one pattern per line, with the Add preset menu (Linux)](images/hide-globs-gtk.png#only-light)](images/hide-globs-gtk.png)
[![The hide editor open as a popover, one pattern per line, with the Add preset menu (Linux)](images/hide-globs-gtk-dark.png#only-dark)](images/hide-globs-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Presets** edits those named sets the same way. They start as
built-ins; edit any of them and your list takes over, so a preset you
delete stays deleted until you restore the built-ins. This screen and
the next have no picture from the GTK shell because they are not
screens there: presets sit inside Projects, and preprocessors inside
Language Servers.

![Settings, Presets tab: named glob sets, each editable one pattern per
line](images/settings-presets.png#only-light)
![Settings, Presets tab: named glob sets, each editable one pattern per
line](images/settings-presets-dark.png#only-dark)

**Language Servers** overrides which command serves a language, for
every project or for one root.

<div class="shots" markdown>
<figure markdown>
[![Settings, Language Servers tab: default and per-project server commands (macOS)](images/settings-servers.png#only-light)](images/settings-servers.png)
[![Settings, Language Servers tab: default and per-project server commands (macOS)](images/settings-servers-dark.png#only-dark)](images/settings-servers-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Settings, Language Servers tab: default and per-project server commands (Linux)](images/settings-servers-gtk.png#only-light)](images/settings-servers-gtk.png)
[![Settings, Language Servers tab: default and per-project server commands (Linux)](images/settings-servers-gtk-dark.png#only-dark)](images/settings-servers-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Preprocessors** runs formatters before every save: one command per
line, each reading the document on standard input and writing it back
on standard output.

![Settings, Preprocessors tab: per-language command chains](images/settings-preprocessors.png#only-light)
![Settings, Preprocessors tab: per-language command chains](images/settings-preprocessors-dark.png#only-dark)

## Small things

**⇧⌘N** starts a new document in a chosen language, filtered from the
keyboard, so highlighting works before the first save.

<div class="shots" markdown>
<figure markdown>
[![The New with Format picker, filtering the language list (macOS)](images/new-with-format.png#only-light)](images/new-with-format.png)
[![The New with Format picker, filtering the language list (macOS)](images/new-with-format-dark.png#only-dark)](images/new-with-format-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![The New with Format picker, filtering the language list (Linux)](images/new-with-format-gtk.png#only-light)](images/new-with-format-gtk.png)
[![The New with Format picker, filtering the language list (Linux)](images/new-with-format-gtk-dark.png#only-dark)](images/new-with-format-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

And the About panel says which build you are running — a real version,
even for a local build.

<div class="shots" markdown>
<figure markdown>
[![The About panel showing the build version, author, repository, and license (macOS)](images/about.png#only-light)](images/about.png)
[![The About panel showing the build version, author, repository, and license (macOS)](images/about-dark.png#only-dark)](images/about-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![The About panel showing the build version, author, repository, and license (Linux)](images/about-gtk.png#only-light)](images/about-gtk.png)
[![The About panel showing the build version, author, repository, and license (Linux)](images/about-gtk-dark.png#only-dark)](images/about-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>
