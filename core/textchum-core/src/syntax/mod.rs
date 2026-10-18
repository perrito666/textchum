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
    ///
    /// A position in an alphabetical list, so it moves whenever a
    /// capture is added. Never compare it to a literal: ask
    /// [`crate::theme::resolve`] for the name you mean. Comparing two
    /// spans' ids to each other is fine.
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

    /// The byte range of the innermost multi-line named node containing
    /// `byte` — the caret's enclosing *block*, language-agnostically:
    /// a function body, a brace pair, a list, a markdown section…
    /// `None` when nothing multi-line encloses the position.
    pub fn block_at(&self, rope: &Rope, byte: usize) -> Option<Range<usize>> {
        let root = self.tree.root_node();
        let mut node = root.named_descendant_for_byte_range(byte, byte)?;
        loop {
            let multi_line = node.start_position().row < node.end_position().row;
            if multi_line && node != root {
                let _ = rope; // ranges are byte offsets; no conversion here
                return Some(node.start_byte()..node.end_byte());
            }
            node = node.parent()?;
        }
    }

    /// The next larger piece of the code's structure around the bytes
    /// `start..end`: a name, then what holds it — the inside of a pair
    /// of brackets or quotes before the pair itself, the expression, the
    /// statement, the block — one step at a time. None past the whole
    /// file.
    pub fn grow(&self, rope: &Rope, start: usize, end: usize) -> Option<Range<usize>> {
        let mut node = self.tree.root_node().named_descendant_for_byte_range(start, end)?;
        let larger = |range: &Range<usize>| {
            range.start <= start && end <= range.end && range.end - range.start > end - start
        };
        loop {
            if let Some(inside) = inside_of_pair(node, rope).filter(|inside| larger(inside)) {
                return Some(inside);
            }
            let whole = node.start_byte()..node.end_byte();
            if larger(&whole) {
                return Some(whole);
            }
            node = node.parent()?;
        }
    }

    /// The named constructs around byte `byte`, outermost first — the
    /// impl and the function, the class and the method — each with the
    /// named constructs beside it: what the breadcrumbs show, and what
    /// each of them offers to go to.
    pub fn breadcrumbs(&self, rope: &Rope, byte: usize) -> Vec<Crumb> {
        let root = self.tree.root_node();
        let Some(mut node) = root.named_descendant_for_byte_range(byte, byte) else {
            return Vec::new();
        };
        let mut crumbs = Vec::new();
        loop {
            if let Some(label) = crumb_label(node, rope) {
                let parent = node.parent().unwrap_or(root);
                let mut cursor = parent.walk();
                let siblings = parent
                    .named_children(&mut cursor)
                    .flat_map(|sibling| labelled_within(sibling, rope))
                    .collect();
                crumbs.push(Crumb { label, start: node.start_byte(), siblings });
            }
            let Some(parent) = node.parent() else { break };
            node = parent;
        }
        crumbs.reverse();
        crumbs
    }

    /// The lines that answer "where am I?" for `line`: the first line
    /// of each enclosing multi-line construct, outermost first. The
    /// `class` line and the `def` line for a statement inside a Python
    /// method, the `impl` and the `fn` for a Rust one.
    ///
    /// A construct that starts on `line` itself is not context — it is
    /// already on screen — and several constructs starting on one line
    /// count once, the way [`Self::fold_ranges`] folds them once.
    pub fn context_lines(&self, rope: &Rope, line: usize) -> Vec<usize> {
        let root = self.tree.root_node();
        let Some(byte) = line_start_byte(rope, line) else {
            return Vec::new();
        };
        let Some(mut node) = root.named_descendant_for_byte_range(byte, byte) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        loop {
            let start = node.start_position().row;
            let mut end = node.end_position().row;
            // Python's blocks end where the dedent is, which is the
            // start of the line after them; see fold_ranges.
            if node.end_position().column == 0 {
                end = end.saturating_sub(1);
            }
            // A node whose first named child starts at its own first
            // byte is a body, not a header: Python's class body starts
            // at the first statement, and pinning that line would show
            // the statement, which says nothing about where you are.
            let is_body = node
                .named_child(0)
                .is_some_and(|child| child.start_byte() == node.start_byte());
            if node != root
                && !is_body
                && start < line
                && end >= line
                && lines.last() != Some(&start)
            {
                lines.push(start);
            }
            let Some(parent) = node.parent() else { break };
            node = parent;
        }
        lines.reverse();
        lines
    }

    /// Every stretch that can be folded: a line that opens a block,
    /// and the last line of it.
    ///
    /// One fold per opening line, and the widest one when several
    /// nodes start there — `impl Item {` opens both the impl and its
    /// body, and folding the impl is what was meant.
    ///
    /// A block that would hide a single line is not offered: in a brace
    /// language that line is the closing brace, and everywhere else it
    /// saves one line of screen in exchange for an arrow in the gutter
    /// on every other line.
    pub fn fold_ranges(&self, rope: &Rope) -> Vec<(usize, usize)> {
        let mut widest: std::collections::BTreeMap<usize, usize> = Default::default();
        let mut cursor = self.tree.walk();
        let mut stack = vec![self.tree.root_node()];
        while let Some(node) = stack.pop() {
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
            let start = node.start_position().row;
            let mut end = node.end_position().row;
            // A node that ends at the first column of a line stops
            // before that line — Python's blocks end where the dedent
            // is, which is the start of the line after them.
            if node.end_position().column == 0 {
                end = end.saturating_sub(1);
            }
            if end <= start + 1 || node == self.tree.root_node() {
                continue;
            }
            let entry = widest.entry(start).or_insert(end);
            *entry = (*entry).max(end);
        }
        let _ = rope;
        widest.into_iter().collect()
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

/// What a node holds between its opening and closing token — the
/// arguments inside parentheses, the statements inside braces, a
/// string's text inside its quotes — without the space next to them.
/// None for a node that does not open and close with such a pair.
fn inside_of_pair(node: Node<'_>, rope: &Rope) -> Option<Range<usize>> {
    const PAIRS: [(&str, &str); 6] =
        [("(", ")"), ("[", "]"), ("{", "}"), ("<", ">"), ("\"", "\""), ("'", "'")];
    if node.child_count() < 2 {
        return None;
    }
    let first = node.child(0)?;
    let last = node.child(node.child_count() as u32 - 1)?;
    if first.is_named() || last.is_named() || !PAIRS.contains(&(first.kind(), last.kind())) {
        return None;
    }
    let (mut from, mut to) = (first.end_byte(), last.start_byte());
    while from < to && rope.byte(from).is_ascii_whitespace() {
        from += 1;
    }
    while to > from && rope.byte(to - 1).is_ascii_whitespace() {
        to -= 1;
    }
    (from < to).then_some(from..to)
}

/// One breadcrumb: a named construct around the caret, where it starts
/// in bytes, and the named constructs beside it, itself included, with
/// where each starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub start: usize,
    pub siblings: Vec<(String, usize)>,
}

/// A sibling as a breadcrumb menu lists it: itself when it is named,
/// else the named constructs one level in — Go's `type (…)` holds its
/// type specs, a decorated Python function holds the function.
fn labelled_within(node: Node<'_>, rope: &Rope) -> Vec<(String, usize)> {
    if let Some(label) = crumb_label(node, rope) {
        return vec![(label, node.start_byte())];
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter_map(|child| Some((crumb_label(child, rope)?, child.start_byte())))
        .collect()
}

/// What a node is called, when it is a construct a breadcrumb names:
/// one with a name of its own that is a declaration — a function, a
/// type, a class, a module — or that spans lines, which a passing
/// expression with a named argument does not. An impl block is named
/// by what it is for. None for the rest.
fn crumb_label(node: Node<'_>, rope: &Rope) -> Option<String> {
    let text = |node: Node<'_>| -> String {
        let piece: String = rope.byte_slice(node.start_byte()..node.end_byte()).chars().take(80).collect();
        piece.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let kind = node.kind();
    if kind.contains("impl") {
        let target = node.child_by_field_name("type")?;
        return Some(match node.child_by_field_name("trait") {
            Some(implemented) => format!("impl {} for {}", text(implemented), text(target)),
            None => format!("impl {}", text(target)),
        });
    }
    let name = node.child_by_field_name("name")?;
    const DECLARATIONS: [&str; 12] = [
        "function", "method", "class", "struct", "enum", "trait", "interface", "module", "mod_",
        "namespace", "type", "macro",
    ];
    let multi_line = node.start_position().row < node.end_position().row;
    let declares = DECLARATIONS.iter().any(|word| kind.contains(word));
    (declares || multi_line).then(|| text(name)).filter(|label| !label.is_empty())
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
/// The byte offset where `line` (zero-based) starts, or `None` past
/// the end of the text.
fn line_start_byte(rope: &Rope, line: usize) -> Option<usize> {
    if line >= rope.len_lines() {
        return None;
    }
    Some(rope.line_to_byte(line))
}

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

/// Styled spans over a snippet of `language`, the way the editor would
/// paint it: for a line shown outside its file — a reference, a place in
/// the jump history. Empty for an unknown language or plain text.
pub fn snippet_highlights(language: &str, code: &str) -> Vec<HighlightSpan> {
    let Some(spec) = languages::by_name(language) else {
        return Vec::new();
    };
    let rope = ropey::Rope::from_str(code);
    let Some(syntax) = SyntaxState::new(spec, &rope) else {
        return Vec::new();
    };
    syntax.highlights(&rope, 0..code.len())
}

#[cfg(test)]
mod breadcrumb_tests {
    use super::*;

    fn state(language: &str, text: &str) -> (SyntaxState, Rope) {
        let rope = Rope::from_str(text);
        let state = SyntaxState::new(languages::by_name(language).unwrap(), &rope).unwrap();
        (state, rope)
    }

    #[test]
    fn a_method_in_an_impl_is_the_impl_then_the_method_with_its_siblings() {
        let text = "struct Server;\n\nimpl Server {\n    fn start(&self) {\n        let x = 1;\n    }\n    fn stop(&self) {}\n}\n";
        let (state, rope) = state("rust", text);
        let crumbs = state.breadcrumbs(&rope, text.find("let x").unwrap());
        let labels: Vec<&str> = crumbs.iter().map(|crumb| crumb.label.as_str()).collect();
        assert_eq!(labels, ["impl Server", "start"]);
        let beside: Vec<&str> = crumbs[1].siblings.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(beside, ["start", "stop"]);
        let top: Vec<&str> = crumbs[0].siblings.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(top, ["Server", "impl Server"]);
        assert_eq!(crumbs[1].start, text.find("fn start").unwrap());
    }

    #[test]
    fn a_python_method_and_a_call_with_a_named_argument() {
        let text = "class Shop:\n    def sell(self):\n        pay(amount=3)\n";
        let (state, rope) = state("python", text);
        let crumbs = state.breadcrumbs(&rope, text.find("amount").unwrap());
        let labels: Vec<&str> = crumbs.iter().map(|crumb| crumb.label.as_str()).collect();
        assert_eq!(labels, ["Shop", "sell"], "a named argument is not a place");
    }

    #[test]
    fn a_go_method_lists_the_types_beside_it() {
        let text = "package main\n\ntype Server struct{}\n\nfunc (s *Server) Start() {\n\tx := 1\n}\n";
        let (state, rope) = state("go", text);
        let crumbs = state.breadcrumbs(&rope, text.find("x :=").unwrap());
        let labels: Vec<&str> = crumbs.iter().map(|crumb| crumb.label.as_str()).collect();
        assert_eq!(labels, ["Start"]);
        let beside: Vec<&str> = crumbs[0].siblings.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(beside, ["Server", "Start"]);
    }

    #[test]
    fn top_level_code_has_no_crumbs() {
        let (state, rope) = state("rust", "use std::io;\n");
        assert!(state.breadcrumbs(&rope, 4).is_empty());
    }
}

/// The same colouring as `spans` painted in order, as stretches that do
/// not overlap: where two spans cover the same text, the later one's
/// style is the one left. A shell whose styles rank by a fixed priority
/// rather than by when they were applied paints these instead, so a
/// server's token over the grammar's span, or an escape inside a
/// string, shows the colour the other shell shows.
pub fn flatten(spans: &[HighlightSpan]) -> Vec<HighlightSpan> {
    let mut boundaries: Vec<usize> =
        spans.iter().flat_map(|span| [span.start_utf16, span.end_utf16]).collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    // The style of each stretch between two boundaries, by the last
    // span that covers it.
    let mut winners: Vec<Option<u32>> = vec![None; boundaries.len().saturating_sub(1)];
    for span in spans {
        let first = boundaries.partition_point(|at| *at < span.start_utf16);
        for (index, slot) in winners.iter_mut().enumerate().skip(first) {
            if boundaries[index] >= span.end_utf16 {
                break;
            }
            *slot = Some(span.style);
        }
    }
    let mut flat: Vec<HighlightSpan> = Vec::new();
    for (index, winner) in winners.into_iter().enumerate() {
        let Some(style) = winner else { continue };
        let (start, end) = (boundaries[index], boundaries[index + 1]);
        match flat.last_mut() {
            Some(last) if last.end_utf16 == start && last.style == style => last.end_utf16 = end,
            _ => flat.push(HighlightSpan { start_utf16: start, end_utf16: end, style }),
        }
    }
    flat
}

#[cfg(test)]
mod flatten_tests {
    use super::*;

    fn span(start_utf16: usize, end_utf16: usize, style: u32) -> HighlightSpan {
        HighlightSpan { start_utf16, end_utf16, style }
    }

    #[test]
    fn the_later_span_wins_where_two_overlap() {
        // A string with an escape inside, then a token over a name.
        let spans = [span(0, 10, 7), span(3, 5, 2), span(12, 15, 1), span(12, 15, 9)];
        assert_eq!(
            flatten(&spans),
            vec![span(0, 3, 7), span(3, 5, 2), span(5, 10, 7), span(12, 15, 9)]
        );
    }

    #[test]
    fn stretches_of_one_style_are_joined_and_gaps_kept() {
        let spans = [span(0, 4, 1), span(4, 8, 1), span(10, 12, 1)];
        assert_eq!(flatten(&spans), vec![span(0, 8, 1), span(10, 12, 1)]);
        assert!(flatten(&[]).is_empty());
    }
}
