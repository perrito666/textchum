//! Syntax highlighting: incremental tree-sitter parsing and styled spans.
//!
//! Each document with a recognized language keeps a [`SyntaxState`]: a
//! parser and the current tree. Edits feed the tree an `InputEdit` and
//! re-parse incrementally (millisecond-scale for keystroke edits), so a
//! highlight query is always answered from an up-to-date tree.
//!
//! Highlighting is pull-based: the shell asks for the styled spans of a
//! range (usually what is on screen), and the core runs the language's
//! highlight query restricted to that range. Spans are returned in
//! application order — later spans win where they overlap, which encodes
//! tree-sitter's "later patterns override" convention without the shell
//! knowing anything about captures.
//!
//! **Injections** are resolved at query time, one level deep: the host
//! language's injection query yields (language, content ranges) pairs —
//! from `#set!` properties or `@injection.language` captures — and each
//! recognized injected language is parsed over just those ranges and
//! queried for the requested span window. This is what makes Markdown
//! work: the block grammar injects the inline grammar for emphasis and
//! links, and fenced code blocks inject their fence's language.

pub mod languages;
pub mod theme;

use std::ops::Range;

use ropey::Rope;
use tree_sitter::{InputEdit, Node, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

use languages::{CompiledLanguage, RegisteredLanguage};

/// Documents larger than this get no syntax tracking: parsing stays fast
/// far beyond it, but there is no point burning memory on generated or
/// binary-ish giants.
pub const SYNTAX_MAX_BYTES: usize = 4 * 1024 * 1024;

/// One styled span, in UTF-16 code units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start_utf16: usize,
    pub end_utf16: usize,
    /// Index into the theme's style table.
    pub style: u32,
}

/// Parser + current tree for one document.
pub struct SyntaxState {
    language: &'static RegisteredLanguage,
    parser: Parser,
    tree: Tree,
}

impl SyntaxState {
    /// Parses `rope` as `language`. Returns None if the parser rejects the
    /// grammar (which would be a build problem, not a user error).
    pub fn new(language: &'static RegisteredLanguage, rope: &Rope) -> Option<Self> {
        let compiled = language.compiled();
        let mut parser = Parser::new();
        parser.set_language(&compiled.language).ok()?;
        let tree = parse_rope(&mut parser, rope, None)?;
        Some(Self {
            language,
            parser,
            tree,
        })
    }

    pub fn language(&self) -> &'static RegisteredLanguage {
        self.language
    }

    /// Applies an edit and re-parses incrementally. Positions describe the
    /// edit the same way tree-sitter wants them; the caller computes them
    /// around the buffer mutation.
    pub fn apply_edit(&mut self, rope: &Rope, edit: InputEdit) {
        self.tree.edit(&edit);
        if let Some(tree) = parse_rope(&mut self.parser, rope, Some(&self.tree)) {
            self.tree = tree;
        }
    }

    /// Styled spans of `byte_range`, host language plus one level of
    /// injections, in application order (later wins).
    pub fn highlights(&self, rope: &Rope, byte_range: Range<usize>) -> Vec<HighlightSpan> {
        let compiled = self.language.compiled();
        let mut spans = Vec::new();
        collect_spans(
            compiled,
            self.tree.root_node(),
            rope,
            byte_range.clone(),
            &mut spans,
        );

        for injection in find_injections(compiled, self.tree.root_node(), rope, &byte_range) {
            let Some(language) = languages::by_name(&injection.language) else {
                continue;
            };
            let injected = language.compiled();
            let mut parser = Parser::new();
            if parser.set_language(&injected.language).is_err() {
                continue;
            }
            let ranges: Vec<tree_sitter::Range> = injection
                .content
                .iter()
                .map(|range| tree_sitter::Range {
                    start_byte: range.start,
                    end_byte: range.end,
                    start_point: point_at(rope, range.start),
                    end_point: point_at(rope, range.end),
                })
                .collect();
            if parser.set_included_ranges(&ranges).is_err() {
                continue;
            }
            let Some(tree) = parse_rope(&mut parser, rope, None) else {
                continue;
            };
            // Injected spans come after the host's, overriding e.g. the
            // host's generic "this is a code fence" styling.
            collect_spans(injected, tree.root_node(), rope, byte_range.clone(), &mut spans);
        }
        spans
    }
}

/// One resolved injection site: a language name and the byte ranges of its
/// content within the host document.
struct Injection {
    language: String,
    content: Vec<Range<usize>>,
}

/// Runs the injection query over `byte_range` of the host tree.
fn find_injections(
    compiled: &CompiledLanguage,
    root: Node<'_>,
    rope: &Rope,
    byte_range: &Range<usize>,
) -> Vec<Injection> {
    let Some(query) = &compiled.injections else {
        return Vec::new();
    };
    let language_capture = capture_index(query, "injection.language");
    let content_capture = capture_index(query, "injection.content");
    let Some(content_capture) = content_capture else {
        return Vec::new();
    };

    let mut injections = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(byte_range.clone());
    let mut matches = cursor.matches(query, root, RopeProvider(rope));
    while let Some(matched) = matches.next() {
        // Language: a `#set! injection.language "x"` property on the
        // pattern, or the text of the `@injection.language` capture.
        let mut language = query
            .property_settings(matched.pattern_index)
            .iter()
            .find(|p| p.key.as_ref() == "injection.language")
            .and_then(|p| p.value.as_ref())
            .map(|v| v.to_string());
        let mut content = Vec::new();
        for capture in matched.captures {
            if Some(capture.index) == language_capture && language.is_none() {
                language = Some(
                    rope.byte_slice(capture.node.start_byte()..capture.node.end_byte())
                        .to_string(),
                );
            } else if capture.index == content_capture {
                content.push(capture.node.start_byte()..capture.node.end_byte());
            }
        }
        if let (Some(language), false) = (language, content.is_empty()) {
            injections.push(Injection { language, content });
        }
    }
    injections
}

/// Runs a language's highlight query over `byte_range`, appending styled
/// spans in capture order.
fn collect_spans(
    compiled: &CompiledLanguage,
    root: Node<'_>,
    rope: &Rope,
    byte_range: Range<usize>,
    out: &mut Vec<HighlightSpan>,
) {
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(byte_range.clone());
    let mut captures = cursor.captures(&compiled.highlights, root, RopeProvider(rope));
    while let Some((matched, capture_ix)) = captures.next() {
        let capture = matched.captures[*capture_ix];
        let Some(style) = compiled.capture_styles[capture.index as usize] else {
            continue;
        };
        let start = capture.node.start_byte().max(byte_range.start);
        let end = capture.node.end_byte().min(byte_range.end);
        if start >= end {
            continue;
        }
        out.push(HighlightSpan {
            start_utf16: byte_to_utf16(rope, start),
            end_utf16: byte_to_utf16(rope, end),
            style,
        });
    }
}

fn capture_index(query: &Query, name: &str) -> Option<u32> {
    query
        .capture_names()
        .iter()
        .position(|n| *n == name)
        .map(|i| i as u32)
}

/// Parses a rope without copying it, chunk by chunk.
fn parse_rope(parser: &mut Parser, rope: &Rope, old_tree: Option<&Tree>) -> Option<Tree> {
    parser.parse_with_options(
        &mut |byte, _point| {
            if byte >= rope.len_bytes() {
                return &[] as &[u8];
            }
            let (chunk, chunk_start, _, _) = rope.chunk_at_byte(byte);
            &chunk.as_bytes()[byte - chunk_start..]
        },
        old_tree,
        None,
    )
}

/// tree-sitter text provider over a rope: yields the chunks of a node's
/// byte range without materializing the text.
struct RopeProvider<'a>(&'a Rope);

impl<'a> tree_sitter::TextProvider<&'a [u8]> for RopeProvider<'a> {
    type I = ChunkBytes<'a>;

    fn text(&mut self, node: Node<'_>) -> Self::I {
        ChunkBytes(self.0.byte_slice(node.start_byte()..node.end_byte()).chunks())
    }
}

struct ChunkBytes<'a>(ropey::iter::Chunks<'a>);

impl<'a> Iterator for ChunkBytes<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        self.0.next().map(str::as_bytes)
    }
}

/// The (row, column-in-bytes) position of a byte offset, as tree-sitter
/// counts them.
pub fn point_at(rope: &Rope, byte: usize) -> Point {
    let row = rope.byte_to_line(byte);
    let column = byte - rope.line_to_byte(row);
    Point { row, column }
}

fn byte_to_utf16(rope: &Rope, byte: usize) -> usize {
    rope.char_to_utf16_cu(rope.byte_to_char(byte))
}
