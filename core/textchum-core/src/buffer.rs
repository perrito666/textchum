//! Rope-backed text buffers.
//!
//! A [`Buffer`] stores text in a rope, which keeps edits cheap (`O(log n)`)
//! anywhere in the document regardless of size. Internally all positions are
//! byte offsets into UTF-8 text. Because Apple frameworks (and the Language
//! Server Protocol, by default) address text in UTF-16 code units, the buffer
//! also exposes UTF-16 based editing so shells can pass platform-native
//! ranges across the boundary without doing their own conversions.

use ropey::Rope;
use std::fmt;

/// Errors produced by buffer operations.
///
/// Every fallible operation validates its inputs up front and leaves the
/// buffer untouched on failure, so callers can treat any error as "nothing
/// happened".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferError {
    /// A byte offset was past the end of the buffer.
    OffsetOutOfBounds { offset: usize, len: usize },
    /// A byte offset did not fall on a UTF-8 character boundary.
    NotACharBoundary { offset: usize },
    /// A UTF-16 code unit offset was past the end of the buffer.
    Utf16OutOfBounds { offset: usize, len: usize },
    /// A range's start was greater than its end.
    InvertedRange { start: usize, end: usize },
}

impl fmt::Display for BufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OffsetOutOfBounds { offset, len } => {
                write!(f, "byte offset {offset} out of bounds (len {len})")
            }
            Self::NotACharBoundary { offset } => {
                write!(f, "byte offset {offset} is not a character boundary")
            }
            Self::Utf16OutOfBounds { offset, len } => {
                write!(f, "utf-16 offset {offset} out of bounds (len {len})")
            }
            Self::InvertedRange { start, end } => {
                write!(f, "range start {start} greater than end {end}")
            }
        }
    }
}

impl std::error::Error for BufferError {}

/// An in-memory text document.
///
/// The buffer is not thread-safe by itself; shells are expected to access it
/// from a single thread (the FFI layer enforces the calling convention).
#[derive(Debug, Default, Clone)]
pub struct Buffer {
    rope: Rope,
}

impl Buffer {
    /// Creates an empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a buffer holding `text`.
    pub fn from_str(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
        }
    }

    /// Length of the text in bytes.
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Length of the text in UTF-16 code units.
    pub fn len_utf16(&self) -> usize {
        self.rope.len_utf16_cu()
    }

    /// Number of lines. An empty buffer has one (empty) line.
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// The entire buffer contents as an owned string.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Inserts `text` at the given byte offset.
    pub fn insert_bytes(&mut self, offset: usize, text: &str) -> Result<(), BufferError> {
        let char_idx = self.byte_to_char(offset)?;
        self.rope.insert(char_idx, text);
        Ok(())
    }

    /// Deletes the byte range `start..end`.
    pub fn delete_bytes(&mut self, start: usize, end: usize) -> Result<(), BufferError> {
        if start > end {
            return Err(BufferError::InvertedRange { start, end });
        }
        let start_char = self.byte_to_char(start)?;
        let end_char = self.byte_to_char(end)?;
        self.rope.remove(start_char..end_char);
        Ok(())
    }

    /// Replaces the UTF-16 code unit range `start..end` with `text`.
    ///
    /// This is the primitive the macOS shell uses: AppKit reports edits as
    /// `NSRange`s of UTF-16 code units, which map directly onto this call.
    pub fn replace_utf16(
        &mut self,
        start: usize,
        end: usize,
        text: &str,
    ) -> Result<(), BufferError> {
        if start > end {
            return Err(BufferError::InvertedRange { start, end });
        }
        let start_char = self.utf16_to_char(start)?;
        let end_char = self.utf16_to_char(end)?;
        self.rope.remove(start_char..end_char);
        self.rope.insert(start_char, text);
        Ok(())
    }

    /// Converts a UTF-16 code unit range to a byte range.
    pub fn utf16_range_to_bytes(
        &self,
        start: usize,
        end: usize,
    ) -> Result<(usize, usize), BufferError> {
        if start > end {
            return Err(BufferError::InvertedRange { start, end });
        }
        let start_char = self.utf16_to_char(start)?;
        let end_char = self.utf16_to_char(end)?;
        Ok((
            self.rope.char_to_byte(start_char),
            self.rope.char_to_byte(end_char),
        ))
    }

    /// Converts a byte offset (which must be a character boundary) to a
    /// UTF-16 code unit offset.
    pub fn byte_to_utf16(&self, offset: usize) -> Result<usize, BufferError> {
        let char_idx = self.byte_to_char(offset)?;
        Ok(self.rope.char_to_utf16_cu(char_idx))
    }

    /// The text in the byte range `start..end` as an owned string.
    pub fn slice_bytes(&self, start: usize, end: usize) -> Result<String, BufferError> {
        if start > end {
            return Err(BufferError::InvertedRange { start, end });
        }
        let start_char = self.byte_to_char(start)?;
        let end_char = self.byte_to_char(end)?;
        Ok(self.rope.slice(start_char..end_char).to_string())
    }

    fn byte_to_char(&self, offset: usize) -> Result<usize, BufferError> {
        let len = self.rope.len_bytes();
        if offset > len {
            return Err(BufferError::OffsetOutOfBounds { offset, len });
        }
        let char_idx = self.rope.byte_to_char(offset);
        // `byte_to_char` floors mid-character offsets; round-trip to reject them.
        if self.rope.char_to_byte(char_idx) != offset {
            return Err(BufferError::NotACharBoundary { offset });
        }
        Ok(char_idx)
    }

    fn utf16_to_char(&self, offset: usize) -> Result<usize, BufferError> {
        let len = self.rope.len_utf16_cu();
        if offset > len {
            return Err(BufferError::Utf16OutOfBounds { offset, len });
        }
        Ok(self.rope.utf16_cu_to_char(offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_read_back() {
        let mut buf = Buffer::new();
        buf.insert_bytes(0, "hello world").unwrap();
        buf.insert_bytes(5, ",").unwrap();
        assert_eq!(buf.text(), "hello, world");
        assert_eq!(buf.len_bytes(), 12);
    }

    #[test]
    fn delete_range() {
        let mut buf = Buffer::from_str("hello, world");
        buf.delete_bytes(5, 7).unwrap();
        assert_eq!(buf.text(), "helloworld");
    }

    #[test]
    fn rejects_mid_character_offsets() {
        let mut buf = Buffer::from_str("héllo"); // 'é' spans bytes 1..3
        let err = buf.insert_bytes(2, "x").unwrap_err();
        assert_eq!(err, BufferError::NotACharBoundary { offset: 2 });
        assert_eq!(buf.text(), "héllo");
    }

    #[test]
    fn rejects_out_of_bounds() {
        let mut buf = Buffer::from_str("abc");
        assert!(matches!(
            buf.insert_bytes(4, "x"),
            Err(BufferError::OffsetOutOfBounds { .. })
        ));
        assert!(matches!(
            buf.delete_bytes(2, 1),
            Err(BufferError::InvertedRange { .. })
        ));
    }

    #[test]
    fn replace_utf16_handles_surrogate_pairs() {
        // '🎉' is one char, two UTF-16 code units, four UTF-8 bytes.
        let mut buf = Buffer::from_str("a🎉b");
        assert_eq!(buf.len_utf16(), 4);
        buf.replace_utf16(1, 3, "-").unwrap();
        assert_eq!(buf.text(), "a-b");
    }

    #[test]
    fn replace_utf16_insert_only() {
        let mut buf = Buffer::from_str("ab");
        buf.replace_utf16(1, 1, "XYZ").unwrap();
        assert_eq!(buf.text(), "aXYZb");
    }

    #[test]
    fn empty_buffer_has_one_line() {
        assert_eq!(Buffer::new().len_lines(), 1);
        assert_eq!(Buffer::from_str("a\nb").len_lines(), 2);
    }
}
