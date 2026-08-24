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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Buffer(e) => write!(f, "{e}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::NoPath => write!(f, "document has no file path"),
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
        };
        doc.history.mark_saved();
        Ok(doc)
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
        self.buffer.replace_utf16(start, end, text)?;
        self.history.record(EditRecord {
            start_byte,
            old,
            new: text.to_owned(),
        });
        Ok(())
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
        self.buffer
            .replace_utf16(start_utf16, end_utf16, &insert)
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

    /// Saves to `path` and adopts it as the document's path.
    pub fn save_as(&mut self, path: &Path) -> Result<(), DocumentError> {
        self.save_to(path)?;
        self.path = Some(path.to_owned());
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
}
