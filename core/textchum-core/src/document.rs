//! Documents: a buffer plus everything that makes it a file.
//!
//! A [`Document`] owns a [`Buffer`] and layers on the concerns an editor has
//! beyond raw text: an undo/redo history, dirty-state tracking, a path, an
//! encoding, and durable saves. All editing goes through the document so the
//! history can never miss an edit.
//!
//! ## Encoding policy
//!
//! Files are decoded on open and re-encoded on save:
//!
//! * Valid UTF-8 loads as UTF-8; a leading BOM is stripped and remembered,
//!   and written back on save.
//! * Anything that is not valid UTF-8 is decoded as ISO-8859-1 (Latin-1),
//!   which maps every byte to a character and therefore cannot fail. Saves
//!   re-encode to Latin-1; if an edit introduced characters outside Latin-1,
//!   the save silently promotes the file to UTF-8 (nothing can be lost in
//!   that direction) and the document's encoding is updated to match.
//!
//! Line endings are not normalized in either direction: what was read is
//! what is written.
//!
//! ## Saves are atomic
//!
//! Saving writes to a temporary file in the target's directory and renames
//! it over the target, so a crash mid-save can never leave a half-written
//! file. The rename also means a save is all-or-nothing from the point of
//! view of other processes watching the file.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, BufferError};
use crate::fsutil::write_atomically;
use crate::history::{EditRecord, History};
use crate::snippet::{self, Region, Session};
use crate::syntax::{self, HighlightSpan, SyntaxState, SYNTAX_MAX_BYTES};

/// The on-disk encoding of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8WithBom,
    Latin1,
}

impl Encoding {
    /// Human-readable name, suitable for a status bar.
    pub fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8WithBom => "UTF-8 with BOM",
            Self::Latin1 => "ISO-8859-1",
        }
    }
}

/// Errors from document I/O and editing.
#[derive(Debug)]
pub enum DocumentError {
    /// An underlying buffer operation rejected its input.
    Buffer(BufferError),
    /// Reading or writing the file failed.
    Io { path: PathBuf, source: std::io::Error },
    /// The document has no path yet; use `save_as`.
    NoPath,
}

impl fmt::Display for DocumentError {
    /// In the interface language: these reach the user as they are, so
    /// they are prose and not identifiers. The operating system's own
    /// message comes through untranslated, which is the one part
    /// neither shell can do anything about.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Buffer(e) => write!(f, "{e}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::NoPath => write!(f, "{}", crate::i18n::tr("document has no file path")),
        }
    }
}

impl std::error::Error for DocumentError {}

impl From<BufferError> for DocumentError {
    fn from(e: BufferError) -> Self {
        Self::Buffer(e)
    }
}

/// An edit the core just performed on itself (via undo or redo), expressed
/// in UTF-16 code units so a shell can replay it verbatim on its display
/// cache: replace `start..end` with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedEdit {
    pub start_utf16: usize,
    pub end_utf16: usize,
    pub text: String,
}

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// A text document: buffer, history, path, and encoding.
pub struct Document {
    buffer: Buffer,
    history: History,
    path: Option<PathBuf>,
    encoding: Encoding,
    syntax: Option<SyntaxState>,
    /// The snippet being filled in, if one is. Its regions are kept
    /// current by [`Document::mutate_buffer`], which every edit passes
    /// through, so they mean the same thing after an edit as before it.
    snippet: Option<Session>,
    /// An expansion handed to the shell to insert, waiting for
    /// [`Document::begin_snippet`] to say where it landed.
    pending_snippet: Option<snippet::Expansion>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// Creates an empty, pathless UTF-8 document. Like any editor's untitled
    /// window, it counts as clean until the first edit.
    pub fn new() -> Self {
        let mut history = History::default();
        history.mark_saved();
        Self {
            buffer: Buffer::new(),
            history,
            path: None,
            encoding: Encoding::Utf8,
            syntax: None,
            snippet: None,
            pending_snippet: None,
        }
    }

    /// Opens the file at `path`, decoding per the module's encoding policy.
    pub fn open(path: &Path) -> Result<Self, DocumentError> {
        let bytes = std::fs::read(path).map_err(|source| DocumentError::Io {
            path: path.to_owned(),
            source,
        })?;
        let (text, encoding) = decode(&bytes);
        let mut doc = Self {
            buffer: Buffer::from_str(&text),
            history: History::default(),
            path: Some(path.to_owned()),
            encoding,
            syntax: None,
            snippet: None,
            pending_snippet: None,
        };
        doc.history.mark_saved();
        doc.detect_language();
        Ok(doc)
    }

    /// Picks the syntax language from the file extension, if any.
    fn detect_language(&mut self) {
        let language = self
            .path
            .as_deref()
            .and_then(crate::syntax::languages::by_path);
        if let Some(language) = language {
            self.set_language(Some(language.spec.name));
        }
    }

    /// Sets (or clears, with `None`) the syntax language. Returns false if
    /// the name is unknown or the document exceeds the syntax size cap.
    pub fn set_language(&mut self, name: Option<&str>) -> bool {
        let Some(name) = name else {
            self.syntax = None;
            return true;
        };
        if self.buffer.len_bytes() > SYNTAX_MAX_BYTES {
            self.syntax = None;
            return false;
        }
        let Some(language) = crate::syntax::languages::by_name(name) else {
            return false;
        };
        self.syntax = SyntaxState::new(language, self.buffer.rope());
        self.syntax.is_some()
    }

    /// The active syntax language name, if any.
    pub fn language_name(&self) -> Option<&'static str> {
        self.syntax.as_ref().map(|s| s.language().spec.name)
    }

    /// The document rendered as an HTML fragment, for the live preview.
    /// `None` unless the document's language is markdown.
    pub fn markdown_html(&self) -> Option<String> {
        if self.language_name() != Some("markdown") {
            return None;
        }
        Some(crate::markdown::to_html(&self.buffer.text()))
    }

    /// The UTF-16 bounds of the innermost multi-line syntax block
    /// containing the position — for go-to-block-start/end navigation.
    /// `None` for plain text or positions outside any block.
    pub fn block_bounds(&self, position: usize) -> Option<(usize, usize)> {
        let syntax = self.syntax.as_ref()?;
        let (byte, _) = self.buffer.utf16_range_to_bytes(position, position).ok()?;
        let range = syntax.block_at(self.buffer.rope(), byte)?;
        Some((
            self.buffer.byte_to_utf16(range.start).ok()?,
            self.buffer.byte_to_utf16(range.end).ok()?,
        ))
    }

    /// Every stretch that can be folded, as `(first line, last line)`
    /// with the lines zero-based: folding hides everything after the
    /// first line, up to and including the last.
    ///
    /// Empty for plain text, which has no structure to fold.
    pub fn fold_ranges(&self) -> Vec<(usize, usize)> {
        let Some(syntax) = self.syntax.as_ref() else {
            return Vec::new();
        };
        syntax.fold_ranges(self.buffer.rope())
    }

    /// The folds as JSON — `[{"start": 4, "end": 9}, …]` — for shells
    /// that reach the core through the C ABI.
    pub fn fold_ranges_json(&self) -> String {
        let items: Vec<serde_json::Value> = self
            .fold_ranges()
            .into_iter()
            .map(|(start, end)| serde_json::json!({"start": start, "end": end}))
            .collect();
        serde_json::Value::Array(items).to_string()
    }

    /// Styled spans over the UTF-16 code unit range `start..end`, in
    /// application order (later spans win where they overlap). Empty for
    /// plain-text documents.
    pub fn highlights(
        &self,
        start: usize,
        end: usize,
    ) -> Result<Vec<HighlightSpan>, BufferError> {
        let Some(syntax) = &self.syntax else {
            return Ok(Vec::new());
        };
        let (start_byte, end_byte) = self.buffer.utf16_range_to_bytes(start, end)?;
        let mut spans = syntax.highlights(self.buffer.rope(), start_byte..end_byte);
        // Hugo shortcodes are template calls inside prose, which no
        // Markdown grammar models — scanned and painted over the
        // prose spans, so `{{< figure >}}` reads as a call, not text.
        let template_ranges = match self.language_name() {
            // Shortcodes are template calls inside prose.
            Some("markdown") => Some(
                crate::hugo::shortcodes(&self.buffer.text())
                    .into_iter()
                    .map(|call| call.range)
                    .collect::<Vec<_>>(),
            ),
            // A layout is HTML with template actions all through it.
            Some("gotmpl") => Some(crate::hugo::template_actions(&self.buffer.text())),
            _ => None,
        };
        if let (Some(ranges), Some(style)) =
            (template_ranges, crate::syntax::theme::resolve("function"))
        {
            for range in ranges {
                if range.end <= start_byte || range.start >= end_byte {
                    continue;
                }
                let (Ok(from), Ok(to)) = (
                    self.buffer.byte_to_utf16(range.start),
                    self.buffer.byte_to_utf16(range.end),
                ) else {
                    continue;
                };
                spans.push(crate::syntax::HighlightSpan {
                    start_utf16: from,
                    end_utf16: to,
                    style,
                });
            }
        }
        Ok(spans)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    pub fn len_bytes(&self) -> usize {
        self.buffer.len_bytes()
    }

    pub fn len_utf16(&self) -> usize {
        self.buffer.len_utf16()
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Replaces the UTF-16 code unit range `start..end` with `text`,
    /// recording the edit in the history.
    pub fn replace_utf16(
        &mut self,
        start: usize,
        end: usize,
        text: &str,
    ) -> Result<(), DocumentError> {
        let (start_byte, end_byte) = self.buffer.utf16_range_to_bytes(start, end)?;
        let old = self.buffer.slice_bytes(start_byte, end_byte)?;
        self.mutate_buffer(start, end, text)?;
        self.history.record(EditRecord {
            start_byte,
            old,
            new: text.to_owned(),
        });
        Ok(())
    }

    /// The single choke point for buffer mutation: performs the replacement
    /// and keeps the syntax tree in sync with an incremental re-parse.
    fn mutate_buffer(&mut self, start: usize, end: usize, text: &str) -> Result<(), BufferError> {
        // tree-sitter wants the edit described in bytes and (row, column)
        // points; start/old-end come from the text before the mutation,
        // new-end from after.
        let edit_geometry = if self.syntax.is_some() {
            let (start_byte, old_end_byte) = self.buffer.utf16_range_to_bytes(start, end)?;
            Some((
                start_byte,
                old_end_byte,
                syntax::point_at(self.buffer.rope(), start_byte),
                syntax::point_at(self.buffer.rope(), old_end_byte),
            ))
        } else {
            None
        };

        self.buffer.replace_utf16(start, end, text)?;

        // Fold the edit into the snippet's live regions. One that the
        // edit cut across cannot be shifted into meaning anything, so
        // the session ends here rather than pointing at the wrong text.
        if let Some(session) = self.snippet.as_mut() {
            if !session.adjust(start, end, text.encode_utf16().count()) {
                self.snippet = None;
            }
        }

        if let Some((start_byte, old_end_byte, start_position, old_end_position)) = edit_geometry {
            let new_end_byte = start_byte + text.len();
            let rope = self.buffer.rope();
            if let Some(syntax_state) = self.syntax.as_mut() {
                syntax_state.apply_edit(
                    rope,
                    tree_sitter::InputEdit {
                        start_byte,
                        old_end_byte,
                        new_end_byte,
                        start_position,
                        old_end_position,
                        new_end_position: syntax::point_at(rope, new_end_byte),
                    },
                );
            }
        }
        Ok(())
    }

    /// Expands an LSP snippet body for an insertion at `at`, returning
    /// the text to insert. Nothing is inserted here: a shell keeps its
    /// own copy of the text and puts the string in through the same path
    /// as anything the user types, so the two never diverge. The stops
    /// are held until [`Self::begin_snippet`] says where the text
    /// landed.
    pub fn expand_snippet(&mut self, at: usize, body: &str) -> String {
        let expansion = {
            let variables = self.snippet_variables(at);
            snippet::expand(body, &variables)
        };
        let text = expansion.text.clone();
        self.pending_snippet = Some(expansion);
        text
    }

    /// Starts a tabstop session over the expansion last returned by
    /// [`Self::expand_snippet`], now sitting at `origin`. Returns the
    /// region the caret should take: the first stop's placeholder,
    /// selected so typing replaces it, or where `$0` asked for the caret
    /// when there is no stop to walk. `None` when nothing is pending.
    ///
    /// A session already running is replaced, not nested: a completion
    /// accepted inside a placeholder is the new thing being filled in.
    pub fn begin_snippet(&mut self, origin: usize) -> Option<Region> {
        let expansion = self.pending_snippet.take()?;
        let inserted_len = expansion.text.encode_utf16().count();
        self.snippet = Session::begin(&expansion, origin, inserted_len);
        Some(match self.snippet.as_ref() {
            Some(session) => session.current_region(),
            // No stops, or only `$0`: `$0` is where the caret goes, and
            // the end of the insertion is where it goes without one.
            None => expansion
                .stops
                .iter()
                .find(|stop| stop.number == 0)
                .and_then(|stop| stop.regions.first())
                .map(|region| Region {
                    start: region.start + origin,
                    end: region.end + origin,
                })
                .unwrap_or(Region {
                    start: origin + inserted_len,
                    end: origin + inserted_len,
                }),
        })
    }

    /// Expand, insert and begin in one step, for callers that hold no
    /// display copy of the text to keep in step.
    pub fn insert_snippet(
        &mut self,
        start: usize,
        end: usize,
        body: &str,
    ) -> Result<Region, DocumentError> {
        let text = self.expand_snippet(start, body);
        self.snippet = None;
        self.replace_utf16(start, end, &text)?;
        Ok(self
            .begin_snippet(start)
            .unwrap_or(Region { start, end: start }))
    }

    /// Whether a snippet is being filled in, and Tab therefore belongs
    /// to it.
    pub fn snippet_active(&self) -> bool {
        self.snippet.is_some()
    }

    /// The stop the caret is on.
    pub fn snippet_region(&self) -> Option<Region> {
        self.snippet.as_ref().map(Session::current_region)
    }

    /// Moves to the next stop, or back to the previous one. Returns the
    /// region to select. Reaching `$0`, or running off the end, gives
    /// the keys back: the session is over and the caret lands where the
    /// snippet said it should.
    pub fn snippet_advance(&mut self, forward: bool) -> Option<Region> {
        let session = self.snippet.as_mut()?;
        let (region, finished) = session.advance(forward);
        if finished {
            self.snippet = None;
        }
        Some(region)
    }

    /// Tells the session where the caret went. A caret outside the
    /// snippet has moved on, and the session ends with it — clicking
    /// elsewhere should not leave Tab captured.
    pub fn snippet_caret_moved(&mut self, position: usize) {
        if self
            .snippet
            .as_ref()
            .is_some_and(|session| !session.contains_caret(position))
        {
            self.snippet = None;
        }
    }

    /// Ends the session, wherever it had got to. Escape, and anything
    /// else that means the snippet is done being a snippet.
    pub fn cancel_snippet(&mut self) {
        self.snippet = None;
    }

    /// Copies the stop just typed in to the other places that carry the
    /// same number, so `${1:name}` written twice stays one name.
    /// Returns the edits performed, to be replayed on the shell's copy
    /// of the text **in array order**; empty when there was nothing to
    /// mirror, which is the common case.
    pub fn snippet_sync(&mut self) -> Vec<AppliedEdit> {
        let Some((source, mut targets)) = self
            .snippet
            .as_mut()
            .and_then(Session::take_mirror)
        else {
            return Vec::new();
        };
        let Ok((start_byte, end_byte)) = self.buffer.utf16_range_to_bytes(source.start, source.end)
        else {
            return Vec::new();
        };
        let Ok(text) = self.buffer.slice_bytes(start_byte, end_byte) else {
            return Vec::new();
        };

        // Back to front, so each target's offsets are still the ones
        // measured before any of these edits were applied.
        targets.sort_by_key(|region| std::cmp::Reverse(region.start));
        if let Some(session) = self.snippet.as_mut() {
            session.set_mirroring(true);
        }
        let mut edits = Vec::new();
        for target in targets {
            let unchanged = self
                .buffer
                .utf16_range_to_bytes(target.start, target.end)
                .and_then(|(from, to)| self.buffer.slice_bytes(from, to))
                .is_ok_and(|current| current == text);
            if unchanged {
                continue;
            }
            if self.replace_utf16(target.start, target.end, &text).is_ok() {
                edits.push(AppliedEdit {
                    start_utf16: target.start,
                    end_utf16: target.end,
                    text: text.clone(),
                });
            }
        }
        if let Some(session) = self.snippet.as_mut() {
            session.set_mirroring(false);
        }
        edits
    }

    /// The snippet variables this document can answer: the ones about
    /// the file it is, and where in it the snippet went. Anything else
    /// is left to its default.
    fn snippet_variables(&self, at: usize) -> impl Fn(&str) -> Option<String> {
        let path = self.path.clone();
        let line = self
            .buffer
            .utf16_range_to_bytes(at, at)
            .map(|(byte, _)| self.buffer.rope().byte_to_line(byte))
            .unwrap_or(0);
        move |name: &str| match name {
            "TM_FILENAME" => path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            "TM_FILENAME_BASE" => path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|n| n.to_string_lossy().into_owned()),
            "TM_DIRECTORY" => path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|n| n.to_string_lossy().into_owned()),
            "TM_FILEPATH" => path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "TM_LINE_INDEX" => Some(line.to_string()),
            "TM_LINE_NUMBER" => Some((line + 1).to_string()),
            _ => None,
        }
    }

    /// How many lines the document has, counted the way a reader
    /// counts them: a file ending in a newline has not gained a line by
    /// doing so, though the rope opens one to hold what comes next.
    pub fn len_lines(&self) -> usize {
        let rope = self.buffer.rope();
        let lines = rope.len_lines();
        if lines > 1 && rope.char(rope.len_chars() - 1) == '\n' {
            lines - 1
        } else {
            lines
        }
    }

    /// The UTF-16 offset of a one-based `line` and `column`, clamped to
    /// what is there: a line past the end is the last line, and a
    /// column past the end of its line is that line's end. Someone
    /// typing 9999 means the end of the file, and refusing them the
    /// jump teaches nothing.
    pub fn offset_for_line(&self, line: usize, column: usize) -> usize {
        let rope = self.buffer.rope();
        // Ropey counts a trailing newline as opening one more line,
        // which has nothing on it; the last line worth landing on is
        // the last one with content.
        let last = rope.len_lines().saturating_sub(1);
        let index = line.saturating_sub(1).min(last);
        let start = rope.line_to_char(index);
        let mut offset: usize = rope.char_to_utf16_cu(start);
        let wanted = column.saturating_sub(1);
        let mut seen = 0usize;
        for character in rope.line(index).chars() {
            if seen >= wanted || character == '\n' || character == '\r' {
                break;
            }
            offset += character.len_utf16();
            seen += 1;
        }
        offset
    }

    /// Ends the current undo coalescing run. Shells call this when the
    /// user's attention moves — caret jumps, focus changes — so the next
    /// keystroke starts a fresh undo step.
    pub fn break_undo_group(&mut self) {
        self.history.break_group();
    }

    /// Starts an explicit edit group: every edit until
    /// [`Self::end_edit_group`] undoes as one step. Used for compound
    /// operations like replace-all.
    pub fn begin_edit_group(&mut self) {
        self.history.begin_group();
    }

    /// Commits the open edit group.
    pub fn end_edit_group(&mut self) {
        self.history.end_group();
    }

    /// Undoes the newest step. Returns the edits the caller must replay on
    /// its display cache **in the given order**; empty if there was nothing
    /// to undo.
    pub fn undo(&mut self) -> Vec<AppliedEdit> {
        // A step's records were applied first-to-last; inverting them
        // last-to-first walks back through the exact intermediate states,
        // so each record's byte range is valid when its turn comes.
        let inverses: Vec<(usize, usize, String)> = match self.history.pop_undo() {
            Some(group) => group
                .iter()
                .rev()
                .map(|r| (r.start_byte, r.new.len(), r.old.clone()))
                .collect(),
            None => return Vec::new(),
        };
        inverses
            .into_iter()
            .map(|(start, remove_len, insert)| self.apply_recorded(start, remove_len, insert))
            .collect()
    }

    /// Redoes the most recently undone step; same contract as
    /// [`Self::undo`].
    pub fn redo(&mut self) -> Vec<AppliedEdit> {
        let replays: Vec<(usize, usize, String)> = match self.history.pop_redo() {
            Some(group) => group
                .iter()
                .map(|r| (r.start_byte, r.old.len(), r.new.clone()))
                .collect(),
            None => return Vec::new(),
        };
        replays
            .into_iter()
            .map(|(start, remove_len, insert)| self.apply_recorded(start, remove_len, insert))
            .collect()
    }

    /// Re-reads the document from its file, replacing the buffer contents.
    ///
    /// The replacement is recorded as a single undoable step (so an
    /// unwanted reload is one ⌘Z away), after which the document counts as
    /// clean — it matches the disk again. Returns the edit for the shell to
    /// replay; a no-op edit (empty range, empty text) means file and buffer
    /// already agreed.
    pub fn reload(&mut self) -> Result<AppliedEdit, DocumentError> {
        let path = self.path.clone().ok_or(DocumentError::NoPath)?;
        let bytes = std::fs::read(&path).map_err(|source| DocumentError::Io {
            path: path.clone(),
            source,
        })?;
        let (text, encoding) = decode(&bytes);
        self.encoding = encoding;

        if text == self.buffer.text() {
            self.history.mark_saved();
            return Ok(AppliedEdit {
                start_utf16: 0,
                end_utf16: 0,
                text: String::new(),
            });
        }

        let end_utf16 = self.buffer.len_utf16();
        self.replace_utf16(0, end_utf16, &text)?;
        self.history.mark_saved();
        Ok(AppliedEdit {
            start_utf16: 0,
            end_utf16,
            text,
        })
    }

    /// Replaces `remove_len` bytes at `start_byte` with `insert`, reporting
    /// the change in UTF-16 units. History records always describe ranges
    /// that exist in the current text, so the conversions cannot fail.
    fn apply_recorded(&mut self, start_byte: usize, remove_len: usize, insert: String) -> AppliedEdit {
        let start_utf16 = self
            .buffer
            .byte_to_utf16(start_byte)
            .expect("history record start must be a valid boundary");
        let end_utf16 = self
            .buffer
            .byte_to_utf16(start_byte + remove_len)
            .expect("history record end must be a valid boundary");
        self.mutate_buffer(start_utf16, end_utf16, &insert)
            .expect("history record must describe a valid range");
        AppliedEdit {
            start_utf16,
            end_utf16,
            text: insert,
        }
    }

    /// Saves to the document's path. Fails with [`DocumentError::NoPath`]
    /// for untitled documents.
    pub fn save(&mut self) -> Result<(), DocumentError> {
        let path = self.path.clone().ok_or(DocumentError::NoPath)?;
        self.save_to(&path)
    }

    /// Saves to `path` and adopts it as the document's path. An untitled
    /// document gains a syntax language from its new extension.
    pub fn save_as(&mut self, path: &Path) -> Result<(), DocumentError> {
        self.save_to(path)?;
        self.path = Some(path.to_owned());
        if self.syntax.is_none() {
            self.detect_language();
        }
        Ok(())
    }

    fn save_to(&mut self, path: &Path) -> Result<(), DocumentError> {
        let text = self.buffer.text();
        let (bytes, encoding) = encode(&text, self.encoding);
        write_atomically(path, &bytes).map_err(|source| DocumentError::Io {
            path: path.to_owned(),
            source,
        })?;
        self.encoding = encoding;
        self.history.mark_saved();
        Ok(())
    }
}

/// Decodes file bytes per the encoding policy.
fn decode(bytes: &[u8]) -> (String, Encoding) {
    let (body, had_bom) = match bytes.strip_prefix(&UTF8_BOM) {
        Some(rest) => (rest, true),
        None => (bytes, false),
    };
    match std::str::from_utf8(body) {
        Ok(text) if had_bom => (text.to_owned(), Encoding::Utf8WithBom),
        Ok(text) => (text.to_owned(), Encoding::Utf8),
        // Not UTF-8 (BOM or not): fall back to Latin-1 over the raw bytes.
        Err(_) => (bytes.iter().map(|&b| b as char).collect(), Encoding::Latin1),
    }
}

/// Encodes text for disk. Returns the bytes and the (possibly promoted)
/// encoding actually used.
fn encode(text: &str, encoding: Encoding) -> (Vec<u8>, Encoding) {
    match encoding {
        Encoding::Utf8 => (text.as_bytes().to_vec(), Encoding::Utf8),
        Encoding::Utf8WithBom => {
            let mut bytes = UTF8_BOM.to_vec();
            bytes.extend_from_slice(text.as_bytes());
            (bytes, Encoding::Utf8WithBom)
        }
        Encoding::Latin1 => {
            let mut bytes = Vec::with_capacity(text.len());
            for ch in text.chars() {
                match u32::from(ch) {
                    code @ 0..=0xFF => bytes.push(code as u8),
                    // An edit introduced a character Latin-1 cannot hold;
                    // promote the whole file to UTF-8 rather than lose it.
                    _ => return (text.as_bytes().to_vec(), Encoding::Utf8),
                }
            }
            (bytes, Encoding::Latin1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("textchum-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_line_and_column_resolve_to_an_offset_and_clamp_to_what_is_there() {
        let mut doc = Document::new();
        doc.replace_utf16(0, 0, "one\ntwo\n🙂 three\nfour").unwrap();
        assert_eq!(doc.len_lines(), 4);

        assert_eq!(doc.offset_for_line(1, 1), 0);
        assert_eq!(doc.offset_for_line(2, 1), 4);
        assert_eq!(doc.offset_for_line(2, 3), 6);
        // A column past the end of its line stops at the line's end,
        // never on the newline or the line after.
        assert_eq!(doc.offset_for_line(2, 99), 7);
        // The emoji is two UTF-16 units, so column 2 is one character
        // in and two units along.
        assert_eq!(doc.offset_for_line(3, 2), 10);
        // A line past the end is the last line.
        assert_eq!(doc.offset_for_line(9999, 1), doc.offset_for_line(4, 1));
        // Zero is the first line, whatever printed it.
        assert_eq!(doc.offset_for_line(0, 0), 0);
    }

    #[test]
    fn a_trailing_newline_does_not_add_a_line() {
        let mut doc = Document::new();
        doc.replace_utf16(0, 0, "one\ntwo\n").unwrap();
        // Two lines, which is what anyone reading the file would say.
        assert_eq!(doc.len_lines(), 2);
        // Past the end is the end of the text, not somewhere out of
        // range.
        assert_eq!(doc.offset_for_line(9999, 1), doc.len_utf16());
        assert_eq!(doc.offset_for_line(2, 1), 4);
        // An empty document is one empty line.
        assert_eq!(Document::new().len_lines(), 1);
    }

    #[test]
    fn hugo_layouts_are_go_templates_by_directory() {
        use crate::syntax::languages;
        let layout = std::path::Path::new("/site/layouts/partials/head.html");
        assert_eq!(languages::by_path(layout).map(|l| l.spec.name), Some("gotmpl"));
        // An ordinary page stays HTML.
        let page = std::path::Path::new("/site/static/index.html");
        assert_eq!(languages::by_path(page).map(|l| l.spec.name), Some("html"));

        // Template actions paint over the markup.
        let mut doc = Document::new();
        doc.replace_utf16(0, 0, "<h1>{{ .Title }}</h1>\n").unwrap();
        doc.set_language(Some("gotmpl"));
        let spans = doc.highlights(0, 21).unwrap();
        let function = crate::syntax::theme::resolve("function").unwrap();
        assert!(
            spans.iter().any(|span| span.style == function
                && span.start_utf16 == 4
                && span.end_utf16 == 16),
            "no action span: {spans:?}"
        );
    }

    #[test]
    fn save_as_detects_the_new_extension_language() {
        let mut doc = Document::new();
        doc.replace_utf16(0, 0, "fn main() {}\n").unwrap();
        assert_eq!(doc.language_name(), None);
        let path = temp_dir().join("gains-language.rs");
        doc.save_as(&path).unwrap();
        assert_eq!(doc.language_name(), Some("rust"));
        assert!(!doc.highlights(0, 12).unwrap().is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn edit_undo_redo_round_trip() {
        let mut doc = Document::new();
        doc.replace_utf16(0, 0, "hello").unwrap();
        doc.break_undo_group();
        doc.replace_utf16(5, 5, " world").unwrap();
        assert_eq!(doc.text(), "hello world");

        let edits = doc.undo();
        assert_eq!(doc.text(), "hello");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].start_utf16, 5);
        assert_eq!(edits[0].text, "");

        let edits = doc.redo();
        assert_eq!(doc.text(), "hello world");
        assert_eq!(edits[0].text, " world");

        assert!(!doc.undo().is_empty());
        assert!(!doc.undo().is_empty());
        assert_eq!(doc.text(), "");
        assert!(doc.undo().is_empty());
    }

    #[test]
    fn typing_undoes_as_one_step() {
        let mut doc = Document::new();
        for (i, ch) in ["a", "b", "c"].iter().enumerate() {
            doc.replace_utf16(i, i, ch).unwrap();
        }
        doc.undo();
        assert_eq!(doc.text(), "");
    }

    #[test]
    fn undo_over_surrogate_pairs_reports_utf16_units() {
        let mut doc = Document::new();
        doc.replace_utf16(0, 0, "a🎉").unwrap(); // 3 UTF-16 units total
        doc.break_undo_group();
        doc.replace_utf16(3, 3, "b").unwrap();
        let edits = doc.undo();
        assert_eq!((edits[0].start_utf16, edits[0].end_utf16), (3, 4));
        assert_eq!(doc.text(), "a🎉");
    }

    #[test]
    fn grouped_edits_undo_and_redo_as_one_step() {
        // A replace-all shaped operation: two disjoint replacements applied
        // back-to-front, exactly as the shell does it.
        let mut doc = Document::new();
        doc.replace_utf16(0, 0, "one two one").unwrap();
        doc.break_undo_group();

        doc.begin_edit_group();
        doc.replace_utf16(8, 11, "1").unwrap();
        doc.replace_utf16(0, 3, "1").unwrap();
        doc.end_edit_group();
        assert_eq!(doc.text(), "1 two 1");

        let edits = doc.undo();
        assert_eq!(doc.text(), "one two one");
        assert_eq!(edits.len(), 2, "one step, both records replayed");

        let edits = doc.redo();
        assert_eq!(doc.text(), "1 two 1");
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn reload_picks_up_disk_changes_and_is_one_undo_step() {
        let path = temp_dir().join("reload.txt");
        std::fs::write(&path, "from disk v1").unwrap();
        let mut doc = Document::open(&path).unwrap();

        std::fs::write(&path, "from disk v2").unwrap();
        let edit = doc.reload().unwrap();
        assert_eq!(doc.text(), "from disk v2");
        assert_eq!(edit.text, "from disk v2");
        assert!(!doc.is_dirty(), "a reloaded document matches the disk");

        // The reload is one ⌘Z away; undoing it diverges from disk again.
        let edits = doc.undo();
        assert_eq!(edits.len(), 1);
        assert_eq!(doc.text(), "from disk v1");
        assert!(doc.is_dirty());
    }

    #[test]
    fn reload_with_unchanged_file_is_a_noop() {
        let path = temp_dir().join("reload-same.txt");
        std::fs::write(&path, "same").unwrap();
        let mut doc = Document::open(&path).unwrap();
        let edit = doc.reload().unwrap();
        assert_eq!((edit.start_utf16, edit.end_utf16), (0, 0));
        assert!(edit.text.is_empty());
        assert!(doc.undo().is_empty(), "no-op reload records nothing");
    }

    #[test]
    fn open_edit_save_round_trip() {
        let path = temp_dir().join("roundtrip.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();

        let mut doc = Document::open(&path).unwrap();
        assert!(!doc.is_dirty());
        assert_eq!(doc.encoding(), Encoding::Utf8);

        doc.replace_utf16(0, 3, "uno").unwrap();
        assert!(doc.is_dirty());
        doc.save().unwrap();
        assert!(!doc.is_dirty());

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "uno\ntwo\n");
    }

    #[test]
    fn undo_to_saved_state_is_clean() {
        let path = temp_dir().join("clean.txt");
        std::fs::write(&path, "base").unwrap();
        let mut doc = Document::open(&path).unwrap();
        doc.replace_utf16(4, 4, "!").unwrap();
        doc.undo();
        assert!(!doc.is_dirty());
    }

    #[test]
    fn bom_is_stripped_and_restored() {
        let path = temp_dir().join("bom.txt");
        let mut on_disk = UTF8_BOM.to_vec();
        on_disk.extend_from_slice("hi".as_bytes());
        std::fs::write(&path, &on_disk).unwrap();

        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.encoding(), Encoding::Utf8WithBom);
        assert_eq!(doc.text(), "hi");

        doc.replace_utf16(2, 2, "!").unwrap();
        doc.save().unwrap();
        let mut expected = UTF8_BOM.to_vec();
        expected.extend_from_slice("hi!".as_bytes());
        assert_eq!(std::fs::read(&path).unwrap(), expected);
    }

    #[test]
    fn latin1_round_trips_and_promotes_when_needed() {
        let path = temp_dir().join("latin1.txt");
        std::fs::write(&path, [b'c', 0xE9]).unwrap(); // "cé" in Latin-1

        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.encoding(), Encoding::Latin1);
        assert_eq!(doc.text(), "cé");

        // Still representable: stays Latin-1 on disk.
        doc.replace_utf16(2, 2, "ü").unwrap();
        doc.save().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), vec![b'c', 0xE9, 0xFC]);
        assert_eq!(doc.encoding(), Encoding::Latin1);

        // Outside Latin-1: the save promotes to UTF-8.
        doc.replace_utf16(3, 3, "→").unwrap();
        doc.save().unwrap();
        assert_eq!(doc.encoding(), Encoding::Utf8);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "céü→");
    }

    #[test]
    fn untitled_save_requires_a_path() {
        let mut doc = Document::new();
        assert!(matches!(doc.save(), Err(DocumentError::NoPath)));
        let path = temp_dir().join("untitled.txt");
        doc.replace_utf16(0, 0, "x").unwrap();
        doc.save_as(&path).unwrap();
        assert_eq!(doc.path(), Some(path.as_path()));
        assert!(!doc.is_dirty());
    }

    #[test]
    fn line_endings_are_preserved() {
        let path = temp_dir().join("crlf.txt");
        std::fs::write(&path, "a\r\nb\r\n").unwrap();
        let mut doc = Document::open(&path).unwrap();
        doc.replace_utf16(0, 1, "A").unwrap();
        doc.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "A\r\nb\r\n");
    }

    #[test]
    fn a_snippet_selects_its_first_placeholder_and_tab_walks_the_rest() {
        let mut doc = Document::new();
        doc.replace_utf16(0, 0, "call ").unwrap();
        let first = doc.insert_snippet(5, 5, "frob(${1:x}, ${2:y})$0").unwrap();
        assert_eq!(doc.text(), "call frob(x, y)");
        assert_eq!((first.start, first.end), (10, 11));
        assert!(doc.snippet_active());

        // Typing over the first placeholder drags the second along.
        doc.replace_utf16(10, 11, "count").unwrap();
        let second = doc.snippet_advance(true).unwrap();
        assert_eq!((second.start, second.end), (17, 18));

        // The last Tab lands on $0 and hands the keys back.
        let exit = doc.snippet_advance(true).unwrap();
        assert_eq!((exit.start, exit.end), (19, 19));
        assert!(!doc.snippet_active());
    }

    #[test]
    fn a_snippet_with_nothing_to_walk_starts_no_session() {
        let mut doc = Document::new();
        let caret = doc.insert_snippet(0, 0, "done()$0").unwrap();
        assert_eq!(doc.text(), "done()");
        assert_eq!((caret.start, caret.end), (6, 6));
        assert!(!doc.snippet_active());
    }

    #[test]
    fn a_linked_stop_mirrors_as_it_is_typed() {
        let mut doc = Document::new();
        doc.insert_snippet(0, 0, "let ${1:name} = ${1:name}.into();").unwrap();
        assert_eq!(doc.text(), "let name = name.into();");

        doc.replace_utf16(4, 8, "value").unwrap();
        let edits = doc.snippet_sync();
        assert_eq!(doc.text(), "let value = value.into();");
        assert_eq!(
            edits,
            vec![AppliedEdit {
                start_utf16: 12,
                end_utf16: 16,
                text: "value".to_owned(),
            }]
        );
        // The mirroring is not itself an edit worth mirroring.
        assert!(doc.snippet_sync().is_empty());
    }

    #[test]
    fn a_caret_that_leaves_the_snippet_ends_the_session() {
        let mut doc = Document::new();
        doc.replace_utf16(0, 0, "before after").unwrap();
        doc.insert_snippet(7, 7, "${1:a}${2:b}").unwrap();
        assert!(doc.snippet_active());
        doc.snippet_caret_moved(8);
        assert!(doc.snippet_active());
        doc.snippet_caret_moved(0);
        assert!(!doc.snippet_active());
    }

    #[test]
    fn an_edit_across_a_stop_boundary_ends_the_session() {
        let mut doc = Document::new();
        doc.insert_snippet(0, 0, "frob(${1:x}, ${2:y})$0").unwrap();
        // Selecting "x, y" and typing over it leaves the stops with
        // nothing to point at.
        doc.replace_utf16(5, 9, "z").unwrap();
        assert!(!doc.snippet_active());
    }

    #[test]
    fn undoing_a_snippet_ends_the_session_with_it() {
        let mut doc = Document::new();
        doc.insert_snippet(0, 0, "${1:a} ${2:b}").unwrap();
        doc.undo();
        assert_eq!(doc.text(), "");
        assert!(!doc.snippet_active());
    }

    #[test]
    fn snippet_variables_come_from_the_file_the_document_is() {
        let dir = temp_dir();
        let path = dir.join("greeting.txt");
        std::fs::write(&path, "").unwrap();
        let mut doc = Document::open(&path).unwrap();
        doc.insert_snippet(0, 0, "// $TM_FILENAME_BASE line ${TM_LINE_NUMBER} ${NOPE:x}")
            .unwrap();
        assert_eq!(doc.text(), "// greeting line 1 x");
    }
}
