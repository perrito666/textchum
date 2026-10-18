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
use crate::diagnostics::{self, Finding};
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
    /// The brackets of the current text, worked out once and kept
    /// until an edit; see [`Document::brackets`].
    brackets: std::cell::RefCell<Option<std::rc::Rc<Vec<crate::brackets::Bracket>>>>,
    /// What a server last said about the text, as ranges of it; see
    /// [`crate::diagnostics`]. [`Document::mutate_buffer`] moves them
    /// with every edit, so they mean the same code after an edit as
    /// before it.
    diagnostics: Vec<Finding>,
    /// The server's inlay hints, as positions in the text; see
    /// [`crate::inlay`]. Moved by [`Document::mutate_buffer`] like the
    /// findings, so they keep to their lines between one answer from
    /// the server and the next.
    inlay_hints: Vec<crate::inlay::Hint>,
    /// The server's colouring of names, in text order; see
    /// [`crate::semantic`]. Moved by every edit, dropped where one
    /// touched it.
    semantic_tokens: Vec<crate::semantic::Token>,
    /// The selections Expand Selection grew from, latest last, and what
    /// it grew to: Shrink Selection walks back down them while the
    /// selection is still the one it left. Any edit forgets them.
    selection_steps: Vec<(usize, usize)>,
    selection_grown: Option<(usize, usize)>,
    /// Where the text was changed, for Go to Last Edit; see
    /// [`crate::trail`].
    edit_trail: crate::trail::Trail,
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
            brackets: std::cell::RefCell::new(None),
            diagnostics: Vec::new(),
            inlay_hints: Vec::new(),
            semantic_tokens: Vec::new(),
            selection_steps: Vec::new(),
            selection_grown: None,
            edit_trail: crate::trail::Trail::default(),
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
            brackets: std::cell::RefCell::new(None),
            diagnostics: Vec::new(),
            inlay_hints: Vec::new(),
            semantic_tokens: Vec::new(),
            selection_steps: Vec::new(),
            selection_grown: None,
            edit_trail: crate::trail::Trail::default(),
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
        // A language decides where the strings and comments are, which
        // decides which brackets count.
        self.brackets.borrow_mut().take();
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

    /// The pinned context for a view whose first visible line is
    /// `top_line`: the first line of each construct enclosing what the
    /// view shows, outermost first, at most `max_rows` of them
    /// (innermost kept when there are more).
    ///
    /// Each pinned row covers one more line of the view, so the answer
    /// is computed against the first line still visible below the pins
    /// — the fixed point of that adjustment.
    ///
    /// Empty for plain text, which has no structure to pin.
    pub fn context_lines(&self, top_line: usize, max_rows: usize) -> Vec<usize> {
        let Some(syntax) = self.syntax.as_ref() else {
            return Vec::new();
        };
        let last_line = self.buffer.rope().len_lines().saturating_sub(1);
        let mut count = 0;
        let mut pinned = Vec::new();
        for _ in 0..=max_rows {
            let line = (top_line + count).min(last_line);
            let mut lines = syntax.context_lines(self.buffer.rope(), line);
            if lines.len() > max_rows {
                lines.drain(..lines.len() - max_rows);
            }
            if lines.len() == count {
                return lines;
            }
            if lines.len() < count {
                // The pins reached past the construct they pin; the
                // larger set stands — a pin held one line long beats a
                // breadcrumb with a step missing.
                return pinned;
            }
            count = lines.len();
            pinned = lines;
        }
        pinned
    }

    /// The context as JSON — `[3, 7]` — for shells that reach the core
    /// through the C ABI.
    pub fn context_lines_json(&self, top_line: usize, max_rows: usize) -> String {
        serde_json::Value::from(self.context_lines(top_line, max_rows)).to_string()
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

    /// Documents past this size keep no bracket structure: the scan is
    /// the whole text and its strings, again after every edit.
    pub const BRACKETS_CEILING_UTF16: usize = 1_000_000;

    /// The brackets of the text, as [`crate::brackets::scan`] answers
    /// them with the strings, characters and comments the syntax knows
    /// of left out. Worked out on the first ask after an edit and kept
    /// until the next; empty past [`Self::BRACKETS_CEILING_UTF16`].
    pub fn brackets(&self) -> std::rc::Rc<Vec<crate::brackets::Bracket>> {
        if let Some(cached) = self.brackets.borrow().as_ref() {
            return std::rc::Rc::clone(cached);
        }
        let length = self.len_utf16();
        let scanned = if length > Self::BRACKETS_CEILING_UTF16 {
            Vec::new()
        } else {
            let quiet: Vec<u32> = ["string", "string.special", "character", "comment"]
                .iter()
                .filter_map(|name| crate::theme::resolve(name))
                .collect();
            let mut excluded: Vec<(usize, usize)> = self
                .highlights(0, length)
                .unwrap_or_default()
                .into_iter()
                .filter(|span| quiet.contains(&span.style))
                .map(|span| (span.start_utf16, span.end_utf16))
                .collect();
            excluded.sort_unstable();
            crate::brackets::scan(&self.text(), &excluded)
        };
        let scanned = std::rc::Rc::new(scanned);
        *self.brackets.borrow_mut() = Some(std::rc::Rc::clone(&scanned));
        scanned
    }

    /// The bracket the caret is on and its partner; see
    /// [`crate::brackets::matching`].
    pub fn matching_bracket(&self, caret: usize) -> Option<(usize, usize)> {
        crate::brackets::matching(&self.brackets(), caret)
    }

    /// The paired brackets within `start..end` with their depth, for
    /// colouring by depth. A bracket without a partner is left out: it
    /// keeps the colour its syntax gives it.
    pub fn bracket_depths(&self, start: usize, end: usize) -> Vec<(usize, u32)> {
        crate::brackets::within(&self.brackets(), start, end)
            .iter()
            .filter(|bracket| bracket.partner.is_some())
            .map(|bracket| (bracket.offset, bracket.depth))
            .collect()
    }

    /// Styled spans over the UTF-16 code unit range `start..end`, in
    /// application order (later spans win where they overlap). Only the
    /// server's tokens, if any, for a document no grammar reads.
    pub fn highlights(
        &self,
        start: usize,
        end: usize,
    ) -> Result<Vec<HighlightSpan>, BufferError> {
        let Some(syntax) = &self.syntax else {
            return Ok(self.semantic_spans(start, end));
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
        // The server's word on what each name is comes last, so it wins
        // over the grammar's guess.
        spans.extend(self.semantic_spans(start, end));
        Ok(spans)
    }

    /// The server's tokens over `start..end`. They never overlap one
    /// another, so the first that ends past `start` begins the stretch.
    fn semantic_spans(&self, start: usize, end: usize) -> Vec<HighlightSpan> {
        let first = self.semantic_tokens.partition_point(|token| token.end <= start);
        self.semantic_tokens[first..]
            .iter()
            .take_while(|token| token.start < end)
            .map(|token| HighlightSpan {
                start_utf16: token.start,
                end_utf16: token.end,
                style: token.style,
            })
            .collect()
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
        // The brackets are the text's; the text is about to change.
        self.brackets.borrow_mut().take();
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

        // Loading a file, reloading it or replacing all of it is not a
        // place anything was edited.
        let whole = start == 0 && end == self.buffer.len_utf16();
        self.buffer.replace_utf16(start, end, text)?;

        // Fold the edit into the snippet's live regions. One that the
        // edit cut across cannot be shifted into meaning anything, so
        // the session ends here rather than pointing at the wrong text.
        let new_len = text.encode_utf16().count();
        self.edit_trail.shift(start, end, new_len);
        if !whole {
            let buffer = &self.buffer;
            self.edit_trail.note(start + new_len, |offset| buffer.line_at_utf16(offset));
        }
        if let Some(session) = self.snippet.as_mut() {
            if !session.adjust(start, end, new_len) {
                self.snippet = None;
            }
        }
        self.selection_steps.clear();
        self.selection_grown = None;
        for finding in &mut self.diagnostics {
            (finding.start, finding.end) =
                diagnostics::shifted((finding.start, finding.end), start, end, new_len);
        }
        self.semantic_tokens.retain_mut(|token| {
            match crate::semantic::shifted((token.start, token.end), start, end, new_len) {
                Some((from, to)) => {
                    (token.start, token.end) = (from, to);
                    true
                }
                None => false,
            }
        });
        self.inlay_hints.retain_mut(|hint| {
            // Every hint but a parameter's name is about the text before
            // it: an inferred type, a chained call's type.
            let about_text_before = hint.kind != 2;
            match crate::inlay::shifted(hint.offset, start, end, new_len, about_text_before) {
                Some(offset) => {
                    hint.offset = offset;
                    true
                }
                None => false,
            }
        });

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

    /// Takes a server's findings for this text: a JSON array of
    /// `{line, character, endLine, endCharacter, severity, message}`
    /// objects, positioned the LSP way against the text as it is now.
    /// Replaces whatever was held; an empty array clears it.
    pub fn set_diagnostics(&mut self, json: &str) {
        let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json)
        else {
            self.diagnostics.clear();
            return;
        };
        let position = |item: &serde_json::Value, line: &str, character: &str| {
            let line = item[line].as_u64().unwrap_or(0) as usize;
            let character = item[character].as_u64().unwrap_or(0) as usize;
            self.buffer.utf16_offset_at(line, character)
        };
        self.diagnostics = items
            .iter()
            .map(|item| {
                let start = position(item, "line", "character");
                let end = position(item, "endLine", "endCharacter").max(start);
                Finding {
                    start,
                    end,
                    severity: item["severity"].as_u64().unwrap_or(1) as u8,
                    message: item["message"].as_str().unwrap_or_default().to_owned(),
                }
            })
            .collect();
    }

    /// The findings as they stand, on the text as it is now.
    pub fn diagnostics(&self) -> &[Finding] {
        &self.diagnostics
    }

    /// Expand Selection: the UTF-16 selection `start..end` grown by one
    /// step of the code's structure (10.3) — see
    /// [`crate::syntax::SyntaxState::grow`]. Text no grammar reads grows
    /// by word, then by line, then to the whole. None when there is
    /// nothing larger.
    pub fn grow_selection(&mut self, start: usize, end: usize) -> Option<(usize, usize)> {
        if self.selection_grown != Some((start, end)) {
            self.selection_steps.clear();
        }
        let (from, to) = self.buffer.utf16_range_to_bytes(start, end).ok()?;
        let grown = self
            .syntax
            .as_ref()
            .and_then(|syntax| syntax.grow(self.buffer.rope(), from, to))
            .or_else(|| plain_growth(&self.buffer.text(), from, to))?;
        let grown = (
            self.buffer.byte_to_utf16(grown.start).ok()?,
            self.buffer.byte_to_utf16(grown.end).ok()?,
        );
        self.selection_steps.push((start, end));
        self.selection_grown = Some(grown);
        Some(grown)
    }

    /// Shrink Selection: the selection Expand Selection grew `start..end`
    /// from. None unless `start..end` is what it last grew to: a
    /// selection made any other way has no structure to give back.
    pub fn shrink_selection(&mut self, start: usize, end: usize) -> Option<(usize, usize)> {
        if self.selection_grown != Some((start, end)) {
            self.selection_steps.clear();
            self.selection_grown = None;
            return None;
        }
        let previous = self.selection_steps.pop()?;
        self.selection_grown = Some(previous);
        Some(previous)
    }

    /// Go to Last Edit: the newest place this document was changed
    /// that is older than `before`, a stamp on the clock every document
    /// shares — with `before` None, the newest of all. The shell asks
    /// each document it holds and goes to the newest answer.
    pub fn last_edit_before(&self, before: Option<u64>) -> Option<crate::trail::Place> {
        self.edit_trail.latest_before(before)
    }

    /// Every place this document was changed, as (line, stamp), for
    /// Recent Places.
    pub fn edit_places(&self) -> Vec<(usize, u64)> {
        self.edit_trail
            .places()
            .iter()
            .map(|place| (self.buffer.line_at_utf16(place.offset), place.stamp))
            .collect()
    }

    /// The breadcrumbs at the UTF-16 `offset`: each named construct
    /// around it, outermost first, as (label, where it starts, and the
    /// constructs beside it as (label, where each starts)). Offsets in
    /// UTF-16. Empty for text no grammar reads.
    pub fn breadcrumbs(&self, offset: usize) -> Vec<(String, usize, Vec<(String, usize)>)> {
        let Some(syntax) = &self.syntax else { return Vec::new() };
        let Ok((byte, _)) = self.buffer.utf16_range_to_bytes(offset, offset) else { return Vec::new() };
        let to_utf16 = |byte: usize| self.buffer.byte_to_utf16(byte).unwrap_or(0);
        syntax
            .breadcrumbs(self.buffer.rope(), byte)
            .into_iter()
            .map(|crumb| {
                let siblings = crumb.siblings.into_iter().map(|(label, at)| (label, to_utf16(at))).collect();
                (crumb.label, to_utf16(crumb.start), siblings)
            })
            .collect()
    }

    /// [`Self::breadcrumbs`] as JSON: `[{label, offset, siblings:
    /// [{label, offset}]}]`.
    pub fn breadcrumbs_json(&self, offset: usize) -> String {
        let crumbs: Vec<serde_json::Value> = self
            .breadcrumbs(offset)
            .into_iter()
            .map(|(label, at, siblings)| {
                serde_json::json!({
                    "label": label,
                    "offset": at,
                    "siblings": siblings
                        .into_iter()
                        .map(|(label, at)| serde_json::json!({"label": label, "offset": at}))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        serde_json::Value::Array(crumbs).to_string()
    }

    /// What in this file can be run — its `main`, each test, each
    /// benchmark — with the command for each; see [`crate::runnables`].
    /// Empty for a file with no path or no grammar.
    pub fn runnables(&self) -> Vec<crate::runnables::Runnable> {
        match (&self.syntax, &self.path) {
            (Some(syntax), Some(path)) => syntax.runnables(path, self.buffer.rope()),
            _ => Vec::new(),
        }
    }

    /// [`Self::runnables`] as JSON: `[{line, endLine, kind, name,
    /// command, cwd}]`, lines zero-based.
    pub fn runnables_json(&self) -> String {
        let rows: Vec<serde_json::Value> = self
            .runnables()
            .into_iter()
            .map(|runnable| {
                serde_json::json!({
                    "line": runnable.line, "endLine": runnable.end_line, "kind": runnable.kind,
                    "name": runnable.name, "command": runnable.command,
                    "cwd": runnable.cwd.to_string_lossy(),
                })
            })
            .collect();
        serde_json::Value::Array(rows).to_string()
    }

    /// [`Self::runnable_at`] as one row of [`Self::runnables_json`], or
    /// `null`.
    pub fn runnable_at_json(&self, line: usize) -> String {
        match self.runnable_at(line) {
            Some(runnable) => serde_json::json!({
                "line": runnable.line, "endLine": runnable.end_line, "kind": runnable.kind,
                "name": runnable.name, "command": runnable.command,
                "cwd": runnable.cwd.to_string_lossy(),
            })
            .to_string(),
            None => "null".to_owned(),
        }
    }

    /// Run at Caret: the innermost runnable around zero-based `line` —
    /// the test the caret is in, else the file's `main` when the caret is
    /// in it.
    pub fn runnable_at(&self, line: usize) -> Option<crate::runnables::Runnable> {
        self.runnables()
            .into_iter()
            .filter(|runnable| runnable.line <= line && line <= runnable.end_line)
            .min_by_key(|runnable| runnable.end_line - runnable.line)
    }

    /// Where Next Finding (or, with `forward` false, Previous Finding)
    /// goes from the UTF-16 `offset`: a finding's start, wrapping round
    /// the document. None with nothing reported.
    pub fn neighbouring_finding(&self, offset: usize, forward: bool) -> Option<usize> {
        crate::diagnostics::neighbour(&self.diagnostics, offset, forward)
    }

    /// The findings as JSON, in the shape [`Document::set_diagnostics`]
    /// takes plus each one's `start` and `end` in UTF-16 units, so a
    /// shell can paint by offset and list by line from one answer.
    pub fn diagnostics_json(&self) -> String {
        let items: Vec<serde_json::Value> = self
            .diagnostics
            .iter()
            .map(|finding| {
                let line = self.buffer.line_at_utf16(finding.start);
                let end_line = self.buffer.line_at_utf16(finding.end);
                serde_json::json!({
                    "line": line,
                    "character": finding.start - self.buffer.utf16_line_start(line),
                    "endLine": end_line,
                    "endCharacter": finding.end - self.buffer.utf16_line_start(end_line),
                    "severity": finding.severity,
                    "message": finding.message,
                    "start": finding.start,
                    "end": finding.end,
                })
            })
            .collect();
        serde_json::Value::Array(items).to_string()
    }

    /// Takes a server's semantic tokens for this text: the `result` of
    /// `textDocument/semanticTokens/full` and the legend the server gave
    /// at initialize, positioned against the text as it is now.
    /// Replaces whatever was held; anything unreadable clears it.
    pub fn set_semantic_tokens(&mut self, result: &str, legend: &str) {
        let parse = |json: &str| serde_json::from_str::<serde_json::Value>(json).unwrap_or_default();
        self.semantic_tokens = crate::semantic::decode(&parse(result), &parse(legend), |line, character| {
            self.buffer.utf16_offset_at(line, character)
        });
    }

    /// The server's tokens as they stand on the current text.
    pub fn semantic_tokens(&self) -> &[crate::semantic::Token] {
        &self.semantic_tokens
    }

    /// Takes a server's inlay hints for this text: the `result` of a
    /// `textDocument/inlayHint` request, positioned against the text
    /// as it is now. Replaces whatever was held; anything that is not
    /// a list clears it.
    pub fn set_inlay_hints(&mut self, json: &str) {
        let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json)
        else {
            self.inlay_hints.clear();
            return;
        };
        let mut hints: Vec<crate::inlay::Hint> = items
            .iter()
            .map(|item| {
                let position = &item["position"];
                // A label is a string, or parts each with a value.
                let label = match &item["label"] {
                    serde_json::Value::String(text) => text.clone(),
                    serde_json::Value::Array(parts) => parts
                        .iter()
                        .filter_map(|part| part["value"].as_str())
                        .collect::<String>(),
                    _ => String::new(),
                };
                crate::inlay::Hint {
                    offset: self.buffer.utf16_offset_at(
                        position["line"].as_u64().unwrap_or(0) as usize,
                        position["character"].as_u64().unwrap_or(0) as usize,
                    ),
                    label,
                    kind: item["kind"].as_u64().unwrap_or(0) as u8,
                    pad_left: item["paddingLeft"].as_bool().unwrap_or(false),
                    pad_right: item["paddingRight"].as_bool().unwrap_or(false),
                }
            })
            .collect();
        hints.sort_by_key(|hint| hint.offset);
        self.inlay_hints = hints;
    }

    /// The text to show after each line that has hints: (zero-based
    /// line, text), in line order. A type hint is written with the name
    /// that stands before it in the text as it is now.
    pub fn inlay_annotations(&self) -> Vec<(usize, String)> {
        let rope = self.buffer.rope();
        let mut lines: Vec<(usize, Vec<String>)> = Vec::new();
        for hint in &self.inlay_hints {
            let at = rope.utf16_cu_to_char(hint.offset.min(rope.len_utf16_cu()));
            let mut from = at;
            while from > 0 {
                let character = rope.char(from - 1);
                if !(character.is_alphanumeric() || character == '_') {
                    break;
                }
                from -= 1;
            }
            let name: String = rope.slice(from..at).chars().collect();
            let Some(piece) = crate::inlay::piece(hint, &name) else {
                continue;
            };
            let line = rope.char_to_line(at);
            match lines.last_mut() {
                Some((last, pieces)) if *last == line => pieces.push(piece),
                _ => lines.push((line, vec![piece])),
            }
        }
        lines
            .into_iter()
            .map(|(line, pieces)| (line, crate::inlay::joined(&pieces)))
            .collect()
    }

    /// The hints as they go inside the lines: (UTF-16 offset, text),
    /// in offset order, two hints at one offset joined into one.
    pub fn inlay_inline(&self) -> Vec<(usize, String)> {
        let mut notes: Vec<(usize, String)> = Vec::new();
        let length = self.buffer.len_utf16();
        for hint in &self.inlay_hints {
            let Some(text) = crate::inlay::inline_text(hint) else { continue };
            let offset = hint.offset.min(length);
            match notes.last_mut() {
                Some((at, joined)) if *at == offset => joined.push_str(&text),
                _ => notes.push((offset, text)),
            }
        }
        notes
    }

    /// [`Self::inlay_inline`] as a JSON array of `{offset, text}`.
    pub fn inlay_inline_json(&self) -> String {
        let notes: Vec<serde_json::Value> = self
            .inlay_inline()
            .into_iter()
            .map(|(offset, text)| serde_json::json!({"offset": offset, "text": text}))
            .collect();
        serde_json::Value::Array(notes).to_string()
    }

    /// [`Self::inlay_annotations`] as a JSON array of `{line, text}`.
    pub fn inlay_annotations_json(&self) -> String {
        let annotations: Vec<serde_json::Value> = self
            .inlay_annotations()
            .into_iter()
            .map(|(line, text)| serde_json::json!({"line": line, "text": text}))
            .collect();
        serde_json::Value::Array(annotations).to_string()
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

/// How text no grammar reads grows, in bytes: the word around the
/// selection, then the whole lines it is on, then everything.
fn plain_growth(text: &str, from: usize, to: usize) -> Option<std::ops::Range<usize>> {
    let larger = |a: usize, b: usize| a <= from && to <= b && b - a > to - from;
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut a = from;
    while let Some(c) = text[..a].chars().next_back().filter(|c| is_word(*c)) {
        a -= c.len_utf8();
    }
    let mut b = to;
    while let Some(c) = text[b..].chars().next().filter(|c| is_word(*c)) {
        b += c.len_utf8();
    }
    if larger(a, b) && text[from..to].chars().all(is_word) {
        return Some(a..b);
    }
    let a = text[..from].rfind('\n').map_or(0, |at| at + 1);
    let b = text[to..].find('\n').map_or(text.len(), |at| to + at);
    if larger(a, b) {
        return Some(a..b);
    }
    larger(0, text.len()).then_some(0..text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(document: &Document) -> Vec<(u64, u64, u64, u64)> {
        let json: serde_json::Value = serde_json::from_str(&document.diagnostics_json()).unwrap();
        json.as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["line"].as_u64().unwrap(),
                    item["character"].as_u64().unwrap(),
                    item["start"].as_u64().unwrap(),
                    item["end"].as_u64().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn findings_ride_with_the_text_above_them() {
        let mut document = Document::new();
        document.replace_utf16(0, 0, "use a;

struct Drinker;
").unwrap();
        document.set_diagnostics(
            r#"[{"line":2,"character":7,"endLine":2,"endCharacter":14,"severity":2,"message":"never constructed"}]"#,
        );
        assert_eq!(lines_of(&document), vec![(2, 7, 15, 22)]);
        // Three lines typed at the top move the finding three lines down.
        document.replace_utf16(0, 0, "use b;
use c;
use d;
").unwrap();
        assert_eq!(lines_of(&document), vec![(5, 7, 36, 43)]);
        assert_eq!(document.diagnostics()[0].message, "never constructed");
        // Typing on the finding's own line, before it, moves it along that line.
        let line_start = document.buffer.utf16_line_start(5);
        document.replace_utf16(line_start, line_start, "pub ").unwrap();
        assert_eq!(lines_of(&document), vec![(5, 11, 40, 47)]);
        // Undo takes it back the same way.
        document.undo();
        assert_eq!(lines_of(&document), vec![(5, 7, 36, 43)]);
    }

    #[test]
    fn the_servers_tokens_paint_over_the_grammar_and_ride_the_text() {
        let mut document = Document::new();
        document.set_language(Some("rust"));
        document.replace_utf16(0, 0, "fn f(n: u8) {\n    let total = n;\n}\n").unwrap();
        document.set_semantic_tokens(
            r#"{"data": [0, 5, 1, 0, 0, 1, 8, 5, 1, 0, 0, 8, 1, 0, 0]}"#,
            r#"{"tokenTypes": ["parameter", "variable"], "tokenModifiers": []}"#,
        );
        let parameter = crate::syntax::theme::resolve("variable.parameter").unwrap();
        let variable = crate::syntax::theme::resolve("variable").unwrap();
        let painted = |document: &Document, from: usize, to: usize| {
            document.highlights(0, document.buffer.len_utf16()).unwrap().into_iter().rev().find(|span| {
                span.start_utf16 == from && span.end_utf16 == to
            }).map(|span| span.style)
        };
        // Later spans win, and the server's come last.
        let last = document.highlights(0, 35).unwrap().last().cloned().unwrap();
        assert_eq!((last.start_utf16, last.end_utf16, last.style), (30, 31, parameter));
        assert_eq!(painted(&document, 5, 6), Some(parameter));
        assert_eq!(painted(&document, 22, 27), Some(variable));
        // Asked about a stretch, only its tokens come.
        assert!(document.highlights(0, 10).unwrap().iter().all(|span| span.start_utf16 < 10));
        // A line typed above moves them with their code; renaming one
        // leaves it to the grammar until the server reads it again.
        document.replace_utf16(0, 0, "// x\n").unwrap();
        assert_eq!(painted(&document, 10, 11), Some(parameter));
        document.replace_utf16(27, 32, "sum").unwrap();
        let tokens: Vec<(usize, usize)> =
            document.semantic_tokens().iter().map(|token| (token.start, token.end)).collect();
        assert_eq!(tokens, vec![(10, 11), (33, 34)]);
        document.set_semantic_tokens("null", "null");
        assert!(document.semantic_tokens().is_empty());
    }

    #[test]
    fn selection_grows_by_structure_and_shrinks_back_the_same_way() {
        let mut document = Document::new();
        document.set_language(Some("rust"));
        let text = "fn main() {\n    let total = add(first, \"two\");\n}\n";
        document.replace_utf16(0, 0, text).unwrap();
        let caret = text.find("first").unwrap() + 2;
        let mut steps = Vec::new();
        let (mut start, mut end) = (caret, caret);
        while let Some(grown) = document.grow_selection(start, end) {
            (start, end) = grown;
            steps.push(text[start..end].to_owned());
        }
        assert_eq!(
            steps[..4],
            ["first", "first, \"two\"", "(first, \"two\")", "add(first, \"two\")"],
            "the name, the inside of the brackets, the brackets, the call"
        );
        assert!(steps.iter().any(|step| step == "let total = add(first, \"two\");"), "the statement");
        assert_eq!(steps.last().map(String::as_str), Some(text), "the whole file last");
        // Back down, one step at a time, to where it started.
        let mut back = Vec::new();
        while let Some(shrunk) = document.shrink_selection(start, end) {
            (start, end) = shrunk;
            back.push(text[start..end].to_owned());
        }
        assert_eq!((start, end), (caret, caret));
        assert_eq!(back[back.len() - 2], "first");
        // A string's text is inside its quotes before the quotes.
        let inside = text.find("two").unwrap() + 1;
        let grown = document.grow_selection(inside, inside).unwrap();
        assert_eq!(&text[grown.0..grown.1], "two");
        let grown = document.grow_selection(grown.0, grown.1).unwrap();
        assert_eq!(&text[grown.0..grown.1], "\"two\"");
        // A selection made some other way has nothing to shrink back to,
        // and an edit forgets the steps.
        assert_eq!(document.shrink_selection(0, 2), None);
        document.replace_utf16(0, 0, " ").unwrap();
        assert_eq!(document.shrink_selection(grown.0 + 1, grown.1 + 1), None);
    }

    #[test]
    fn plain_text_grows_by_word_then_line_then_everything() {
        let mut document = Document::new();
        let text = "one two\nthree four\n";
        document.replace_utf16(0, 0, text).unwrap();
        let at = text.find("four").unwrap() + 1;
        let word = document.grow_selection(at, at).unwrap();
        let line = document.grow_selection(word.0, word.1).unwrap();
        let all = document.grow_selection(line.0, line.1).unwrap();
        assert_eq!(
            [&text[word.0..word.1], &text[line.0..line.1], &text[all.0..all.1]],
            ["four", "three four", text]
        );
        assert_eq!(document.grow_selection(all.0, all.1), None);
    }

    #[test]
    fn edits_leave_a_trail_and_loading_does_not() {
        let mut document = Document::new();
        let text = (0..40).map(|n| format!("line {n}\n")).collect::<String>();
        document.replace_utf16(0, 0, &text).unwrap();
        assert_eq!(document.last_edit_before(None), None, "loading the text is no edit place");
        let at = document.buffer.utf16_line_start(5);
        document.replace_utf16(at, at, "x").unwrap();
        let far = document.buffer.utf16_line_start(30);
        document.replace_utf16(far, far, "y").unwrap();
        let newest = document.last_edit_before(None).unwrap();
        assert_eq!(document.buffer.line_at_utf16(newest.offset), 30);
        let older = document.last_edit_before(Some(newest.stamp)).unwrap();
        assert_eq!(document.buffer.line_at_utf16(older.offset), 5);
        // A line typed above moves both with their code.
        document.replace_utf16(0, 0, "new\n").unwrap();
        let newest = document.last_edit_before(None).unwrap();
        assert_eq!(
            document.buffer.line_at_utf16(newest.offset),
            1,
            "the newest place is where the typing left the caret, after the line"
        );
        let then = document.last_edit_before(Some(newest.stamp)).unwrap();
        assert_eq!(document.buffer.line_at_utf16(then.offset), 31);
    }

    #[test]
    fn breadcrumbs_are_in_utf16_offsets() {
        let mut document = Document::new();
        document.set_language(Some("rust"));
        let text = "// é\nfn main() {\n    let x = 1;\n}\n";
        document.replace_utf16(0, 0, text).unwrap();
        let caret = text.encode_utf16().count() - 4;
        let crumbs = document.breadcrumbs(caret);
        assert_eq!(crumbs.len(), 1);
        assert_eq!(crumbs[0].0, "main");
        assert_eq!(crumbs[0].1, 5, "after a two-byte character that is one UTF-16 unit");
        assert!(document.breadcrumbs_json(caret).contains("\"label\":\"main\""));
    }

    #[test]
    fn the_runnable_at_a_line_is_the_innermost_around_it() {
        let dir = std::env::temp_dir().join(format!("textchum-runnable-at-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        let path = dir.join("src/main.rs");
        std::fs::write(&path, "").unwrap();
        let mut document = Document::open(&path).unwrap();
        let text = "fn main() {\n    run();\n}\n\n#[test]\nfn adds() {\n    check();\n}\n";
        document.replace_utf16(0, 0, text).unwrap();
        assert_eq!(document.runnable_at(1).map(|runnable| runnable.kind), Some("main"));
        assert_eq!(document.runnable_at(6).map(|runnable| runnable.name), Some("adds".to_owned()));
        assert_eq!(document.runnable_at(3), None, "between the two, nothing");
        assert!(document.runnables_json().contains("cargo test -- adds --exact"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inferred_types_are_listed_after_their_line_and_stay_on_it() {
        let mut document = Document::new();
        document
            .replace_utf16(0, 0, "fn main() {\n    let d = make();\n    let (a, b) = pair(d, 2);\n}\n")
            .unwrap();
        document.set_inlay_hints(
            r#"[
              {"position": {"line": 1, "character": 9}, "label": ": Drinker", "kind": 1},
              {"position": {"line": 2, "character": 13}, "label": [{"value": ": "}, {"value": "u8"}], "kind": 1},
              {"position": {"line": 2, "character": 10}, "label": ": i32", "kind": 1},
              {"position": {"line": 2, "character": 21}, "label": "who:", "kind": 2}
            ]"#,
        );
        assert_eq!(
            document.inlay_annotations(),
            vec![(1, "d: Drinker".to_owned()), (2, "a: i32, b: u8".to_owned())],
            "in the order they are written, each with its name, parameters left out"
        );
        // A line typed above moves them down with their code.
        document.replace_utf16(0, 0, "// note\n").unwrap();
        let lines: Vec<usize> = document.inlay_annotations().iter().map(|(line, _)| *line).collect();
        assert_eq!(lines, vec![2, 3]);
        // Renaming on the line is reflected at once; the type is the
        // server's last word until it is asked again.
        let name = document.buffer.utf16_offset_at(2, 8);
        document.replace_utf16(name, name + 1, "drinker").unwrap();
        assert_eq!(document.inlay_annotations()[0], (2, "drinker: Drinker".to_owned()));
        // Deleting the code a hint was about takes the hint with it.
        let line = document.buffer.utf16_line_start(3);
        let next = document.buffer.utf16_line_start(4);
        document.replace_utf16(line, next, "").unwrap();
        assert_eq!(document.inlay_annotations().len(), 1);
        assert!(document.inlay_annotations_json().contains("\"text\":\"drinker: Drinker\""));
        document.set_inlay_hints("null");
        assert!(document.inlay_annotations().is_empty());
    }

    #[test]
    fn inside_the_line_every_hint_sits_at_its_own_place_and_rides_the_text() {
        let mut document = Document::new();
        document
            .replace_utf16(0, 0, "fn main() {\n    let d = make();\n    pair(d, 2);\n}\n")
            .unwrap();
        document.set_inlay_hints(
            r#"[
              {"position": {"line": 1, "character": 9}, "label": ": Drinker", "kind": 1},
              {"position": {"line": 2, "character": 9}, "label": "who", "kind": 2, "paddingRight": true},
              {"position": {"line": 2, "character": 12}, "label": "n", "kind": 2, "paddingRight": true},
              {"position": {"line": 2, "character": 12}, "label": ": u8", "kind": 1}
            ]"#,
        );
        assert_eq!(
            document.inlay_inline(),
            vec![(21, ": Drinker".to_owned()), (41, "who ".to_owned()), (44, "n : u8".to_owned())],
            "parameters are in, with their space; two hints at one place are one note"
        );
        document.replace_utf16(0, 0, "// note\n").unwrap();
        let offsets: Vec<usize> = document.inlay_inline().iter().map(|(at, _)| *at).collect();
        assert_eq!(offsets, vec![29, 49, 52]);
        assert!(document.inlay_inline_json().contains("\"offset\":29"));
    }

    #[test]
    fn a_stale_column_lands_on_the_line_and_an_empty_list_clears() {
        let mut document = Document::new();
        document.replace_utf16(0, 0, "ab
cd
").unwrap();
        document.set_diagnostics(
            r#"[{"line":1,"character":40,"endLine":9,"endCharacter":0,"severity":1,"message":"x"}]"#,
        );
        assert_eq!(lines_of(&document), vec![(1, 2, 5, 6)]);
        document.set_diagnostics("[]");
        assert!(document.diagnostics().is_empty());
        document.set_diagnostics("not json");
        assert!(document.diagnostics().is_empty());
    }

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
