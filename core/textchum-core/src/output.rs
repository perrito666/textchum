//! A command's output as the run panel shows it: lines of text with
//! their colours, and the places in them that open with a click.
//!
//! Output arrives through a pipe in chunks cut anywhere — inside a
//! character, inside an escape sequence. It is read the way a terminal
//! would read it, as far as a panel of lines needs: a carriage return
//! starts the line again, so a progress bar redrawn in place stays one
//! line; colour codes become colour; every other escape sequence is
//! read and dropped rather than shown.
//!
//! A place is a file and a line that a compiler, a test runner or a
//! stack trace named, found by the shapes those tools write, and kept
//! only when the file is there: `1.2.3:4` in a version string is not a
//! place. Relative paths are read against the directory the command
//! ran in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Value};

/// How many lines a run keeps; past it the oldest go, half at a time.
const MAX_LINES: usize = 50_000;

/// A stretch of a line in a colour, in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    /// 0xRRGGBB, or None for the text's own colour.
    pub colour: Option<u32>,
    pub bold: bool,
}

/// A place a line names, in characters within the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub start: usize,
    pub end: usize,
    pub path: PathBuf,
    /// Zero-based, as the editor counts.
    pub line: usize,
    /// Zero-based characters; 0 when the tool gave none.
    pub column: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub spans: Vec<Span>,
    pub places: Vec<Place>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Pen {
    colour: Option<u32>,
    bold: bool,
}

/// Where the reader is inside an escape sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Escape {
    None,
    /// Just read ESC.
    Start,
    /// Inside `ESC [`, collecting parameters.
    Csi(String),
    /// Inside `ESC ]`, until BEL or `ESC \`.
    Osc { saw_escape: bool },
}

/// A run's output so far.
pub struct Transcript {
    cwd: PathBuf,
    lines: Vec<Line>,
    /// Bytes of a character cut by the chunk boundary.
    partial: Vec<u8>,
    escape: Escape,
    pen: Pen,
    /// A carriage return was read: the next character starts the line
    /// again, unless it is the line feed of a CRLF.
    returned: bool,
    /// The first line changed since [`Transcript::take`] last answered.
    dirty_from: Option<usize>,
    /// Lines were dropped from the front since the last answer, so a
    /// shell must draw everything again.
    reset: bool,
    /// Whether a path is a file, asked once per path.
    known: HashMap<PathBuf, bool>,
    /// How many lines were dropped from the front, ever.
    dropped: usize,
}

/// What changed since the last time it was asked.
pub struct Changes<'a> {
    /// The index of the first line in `lines`; lines from it on replace
    /// what the shell has from it on.
    pub first: usize,
    pub lines: &'a [Line],
    /// Everything was drawn again from line 0.
    pub reset: bool,
}

impl Transcript {
    pub fn new(cwd: &Path) -> Transcript {
        Transcript {
            cwd: cwd.to_path_buf(),
            lines: vec![Line::default()],
            partial: Vec::new(),
            escape: Escape::None,
            pen: Pen::default(),
            returned: false,
            dirty_from: None,
            reset: false,
            known: HashMap::new(),
            dropped: 0,
        }
    }

    /// Reads a chunk of output, which may be cut anywhere.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut data = std::mem::take(&mut self.partial);
        data.extend_from_slice(bytes);
        let (text, rest) = match std::str::from_utf8(&data) {
            Ok(text) => (text.to_owned(), Vec::new()),
            Err(error) if error.error_len().is_none() => {
                // A character cut by the chunk: the rest comes next time.
                let valid = error.valid_up_to();
                (String::from_utf8_lossy(&data[..valid]).into_owned(), data[valid..].to_vec())
            }
            Err(_) => (String::from_utf8_lossy(&data).into_owned(), Vec::new()),
        };
        self.partial = rest;
        let first_touched = self.lines.len() - 1;
        for character in text.chars() {
            self.read(character);
        }
        self.mark_dirty(first_touched);
        if self.lines.len() > MAX_LINES {
            let drop = self.lines.len() / 2;
            self.lines.drain(..drop);
            self.dropped += drop;
            self.reset = true;
        }
    }

    /// Notes something written by the editor rather than the command —
    /// what was sent to its input, how it ended — as a line of its own.
    pub fn note(&mut self, text: &str) {
        if !self.lines.last().is_some_and(|line| line.text.is_empty()) {
            self.newline();
        }
        let at = self.lines.len() - 1;
        let line = &mut self.lines[at];
        line.text = text.to_owned();
        let length = text.chars().count();
        line.spans = vec![Span { start: 0, end: length, colour: Some(0x8a8a8a), bold: false }];
        self.newline();
        self.mark_dirty(at);
    }

    fn mark_dirty(&mut self, from: usize) {
        self.dirty_from = Some(self.dirty_from.map_or(from, |known| known.min(from)));
    }

    fn read(&mut self, character: char) {
        match std::mem::replace(&mut self.escape, Escape::None) {
            Escape::None => {}
            Escape::Start => {
                self.escape = match character {
                    '[' => Escape::Csi(String::new()),
                    ']' => Escape::Osc { saw_escape: false },
                    // Any other escape is two characters long.
                    _ => Escape::None,
                };
                return;
            }
            Escape::Csi(mut parameters) => {
                if ('@'..='~').contains(&character) {
                    self.csi(&parameters, character);
                } else {
                    parameters.push(character);
                    self.escape = Escape::Csi(parameters);
                }
                return;
            }
            Escape::Osc { saw_escape } => {
                let done = character == '\u{7}' || (saw_escape && character == '\\');
                if !done {
                    self.escape = Escape::Osc { saw_escape: character == '\u{1b}' };
                }
                return;
            }
        }
        match character {
            '\u{1b}' => self.escape = Escape::Start,
            '\r' => self.returned = true,
            '\n' => {
                self.returned = false;
                self.newline();
            }
            '\t' => self.put('\t'),
            // Other control characters — the bell, a backspace — are
            // not text.
            c if c.is_control() => {}
            c => {
                if std::mem::take(&mut self.returned) {
                    self.clear_line();
                }
                self.put(c);
            }
        }
    }

    fn csi(&mut self, parameters: &str, command: char) {
        match command {
            'm' => self.sgr(parameters),
            // Erase in line. Writing always happens at the end of the
            // line here, so erasing to its end is nothing; erasing the
            // whole line, or erasing after a carriage return, is the line.
            'K' if self.returned || parameters == "2" => {
                self.returned = false;
                self.clear_line();
            }
            _ => {}
        }
    }

    /// Select Graphic Rendition: the colours and the weight.
    fn sgr(&mut self, parameters: &str) {
        let codes: Vec<u32> = if parameters.is_empty() {
            vec![0]
        } else {
            parameters.split(';').map(|code| code.parse().unwrap_or(0)).collect()
        };
        let mut at = 0;
        while at < codes.len() {
            match codes[at] {
                0 => self.pen = Pen::default(),
                1 => self.pen.bold = true,
                22 => self.pen.bold = false,
                39 => self.pen.colour = None,
                code @ 30..=37 => self.pen.colour = Some(ANSI[(code - 30) as usize]),
                code @ 90..=97 => self.pen.colour = Some(ANSI[(code - 90 + 8) as usize]),
                38 => match codes.get(at + 1) {
                    Some(5) => {
                        self.pen.colour = codes.get(at + 2).map(|index| colour_256(*index));
                        at += 2;
                    }
                    Some(2) => {
                        if let (Some(r), Some(g), Some(b)) =
                            (codes.get(at + 2), codes.get(at + 3), codes.get(at + 4))
                        {
                            self.pen.colour = Some((r & 0xff) << 16 | (g & 0xff) << 8 | (b & 0xff));
                        }
                        at += 4;
                    }
                    _ => {}
                },
                // Backgrounds and the rest are not drawn.
                48 => at += if codes.get(at + 1) == Some(&5) { 2 } else { 4 },
                _ => {}
            }
            at += 1;
        }
    }

    fn put(&mut self, character: char) {
        let pen = self.pen;
        let line = self.lines.last_mut().expect("there is always a line");
        let at = line.text.chars().count();
        line.text.push(character);
        if pen == Pen::default() {
            return;
        }
        match line.spans.last_mut() {
            Some(span) if span.end == at && span.colour == pen.colour && span.bold == pen.bold => span.end += 1,
            _ => line.spans.push(Span { start: at, end: at + 1, colour: pen.colour, bold: pen.bold }),
        }
    }

    fn clear_line(&mut self) {
        let line = self.lines.last_mut().expect("there is always a line");
        line.text.clear();
        line.spans.clear();
    }

    fn newline(&mut self) {
        let finished = self.lines.len() - 1;
        let places = self.places_in(&self.lines[finished].text.clone());
        self.lines[finished].places = places;
        self.lines.push(Line::default());
    }

    /// Finds the places a line names whose files are there.
    fn places_in(&mut self, text: &str) -> Vec<Place> {
        let mut found: Vec<Place> = Vec::new();
        for (start, end, path, line, column) in candidates(text) {
            if found.iter().any(|place| start < place.end && place.start < end) {
                continue;
            }
            let full = if Path::new(&path).is_absolute() { PathBuf::from(&path) } else { self.cwd.join(&path) };
            let exists = *self.known.entry(full.clone()).or_insert_with(|| full.is_file());
            if exists && line > 0 {
                found.push(Place { start, end, path: full, line: line - 1, column: column.saturating_sub(1) });
            }
        }
        found.sort_by_key(|place| place.start);
        found
    }

    /// What changed since the last time this was asked, and forgets it.
    pub fn take(&mut self) -> Option<Changes<'_>> {
        let first = self.dirty_from.take()?;
        let reset = std::mem::take(&mut self.reset);
        let first = if reset { 0 } else { first.min(self.lines.len()) };
        Some(Changes { first, lines: &self.lines[first..], reset })
    }

    /// How many lines were dropped from the front to keep the transcript
    /// in bounds: line `i` of [`Self::lines`] is line `dropped + i` of
    /// everything read.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// The place after (`forward`) or before the line and character
    /// given, round the end; with nothing given, the first or the last.
    pub fn neighbouring_place(&self, from: Option<(usize, usize)>, forward: bool) -> Option<(usize, &Place)> {
        let all: Vec<(usize, &Place)> = self
            .lines
            .iter()
            .enumerate()
            .flat_map(|(index, line)| line.places.iter().map(move |place| (index, place)))
            .collect();
        let key = |(index, place): &(usize, &Place)| (*index, place.start);
        let found = match (from, forward) {
            (None, true) => all.first(),
            (None, false) => all.last(),
            (Some(at), true) => all.iter().find(|entry| key(entry) > at).or(all.first()),
            (Some(at), false) => all.iter().rev().find(|entry| key(entry) < at).or(all.last()),
        };
        found.copied()
    }
}

/// [`Changes`] as JSON for a shell: `{first, reset, lines: [{text,
/// spans: [[start, end, colour or null, bold]], places: [[start, end,
/// path, line, column]]}]}`.
pub fn changes_json(changes: &Changes) -> Value {
    let lines: Vec<Value> = changes
        .lines
        .iter()
        .map(|line| {
            json!({
                "text": line.text,
                "spans": line.spans.iter().map(|span| json!([
                    span.start, span.end,
                    span.colour.map(|colour| format!("#{colour:06X}")),
                    span.bold,
                ])).collect::<Vec<_>>(),
                "places": line.places.iter().map(|place| json!([
                    place.start, place.end, place.path.to_string_lossy(), place.line, place.column,
                ])).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({"first": changes.first, "reset": changes.reset, "lines": lines})
}

/// The places a line seems to name, in characters, before checking the
/// files exist: (start, end, path, one-based line, one-based column or
/// 0).
fn candidates(text: &str) -> Vec<(usize, usize, String, usize, usize)> {
    static SHAPES: OnceLock<Vec<Regex>> = OnceLock::new();
    let shapes = SHAPES.get_or_init(|| {
        [
            // Python's traceback: File "x.py", line 12
            r#"File "(?P<path>[^"]+)", line (?P<line>\d+)"#,
            // TypeScript and C#: x.ts(12,5)
            r"(?P<path>[^\s:()\[\]'`]+\.\w+)\((?P<line>\d+),(?P<col>\d+)\)",
            // Everyone else: x.rs:12:5, ./x.go:12, x.py:12: — and the same
            // inside a stack trace's parentheses.
            r"(?P<path>(?:[A-Za-z]:)?[^\s:()\[\]'`,]*[\w)]\.[A-Za-z][\w]*):(?P<line>\d+)(?::(?P<col>\d+))?",
        ]
        .iter()
        .map(|shape| Regex::new(shape).expect("the shapes are valid"))
        .collect()
    });
    let to_chars = |byte: usize| text[..byte].chars().count();
    let mut found = Vec::new();
    for shape in shapes {
        for captures in shape.captures_iter(text) {
            let whole = captures.get(0).expect("a match has a whole");
            let path = captures["path"].trim_start_matches("--> ").to_owned();
            let line = captures["line"].parse().unwrap_or(0);
            let column = captures.name("col").and_then(|col| col.as_str().parse().ok()).unwrap_or(0);
            let path_start = captures.name("path").map_or(whole.start(), |path| path.start());
            found.push((to_chars(path_start), to_chars(whole.end()), path, line, column));
        }
    }
    found
}

/// The sixteen colours, chosen to read on a light background and a
/// dark one alike.
const ANSI: [u32; 16] = [
    0x6b6b6b, 0xc9373c, 0x2e9c47, 0xb5860b, 0x3a6fd8, 0xa543b5, 0x1a9aa6, 0x9a9a9a,
    0x8a8a8a, 0xe2575b, 0x3fbf5f, 0xd4a017, 0x5b8cf0, 0xc16bd1, 0x2fbccb, 0xbdbdbd,
];

fn colour_256(index: u32) -> u32 {
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let index = index - 16;
            let level = |value: u32| if value == 0 { 0 } else { 55 + value * 40 };
            level(index / 36) << 16 | level(index / 6 % 6) << 8 | level(index % 6)
        }
        _ => {
            let grey = 8 + (index.min(255) - 232) * 10;
            grey << 16 | grey << 8 | grey
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(transcript: &Transcript) -> Vec<String> {
        transcript.lines().iter().map(|line| line.text.clone()).collect()
    }

    #[test]
    fn a_progress_bar_redrawn_in_place_stays_one_line() {
        let mut transcript = Transcript::new(Path::new("/"));
        transcript.feed(b"Compiling\n 10%\r 50%\r100%\r\ndone\n");
        assert_eq!(texts(&transcript), ["Compiling", "100%", "done", ""]);
    }

    #[test]
    fn colours_become_spans_and_other_escapes_go() {
        let mut transcript = Transcript::new(Path::new("/"));
        transcript.feed(b"\x1b[1m\x1b[31merror\x1b[0m: bad \x1b]8;;http://x\x07link\x1b]8;;\x07 \x1b[Kok\n");
        let line = &transcript.lines()[0];
        assert_eq!(line.text, "error: bad link ok");
        assert_eq!(line.spans, [Span { start: 0, end: 5, colour: Some(ANSI[1]), bold: true }]);
    }

    #[test]
    fn a_character_or_an_escape_cut_by_the_chunk_is_read_whole() {
        let mut transcript = Transcript::new(Path::new("/"));
        let bytes = "é\x1b[32mgo\n".as_bytes();
        for byte in bytes {
            transcript.feed(&[*byte]);
        }
        let line = &transcript.lines()[0];
        assert_eq!(line.text, "égo");
        assert_eq!(line.spans[0].colour, Some(ANSI[2]));
    }

    #[test]
    fn the_places_tools_name_are_found_when_the_files_are_there() {
        let dir = std::env::temp_dir().join(format!("textchum-output-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("app.py"), "x\n").unwrap();
        std::fs::write(dir.join("ui.ts"), "x\n").unwrap();
        let mut transcript = Transcript::new(&dir);
        transcript.feed(
            b"  --> src/main.rs:3:5\n  File \"app.py\", line 12, in <module>\nui.ts(4,7): error\nversion 1.2.3:4 and gone.rs:9\n",
        );
        let place = |line: usize| transcript.lines()[line].places.clone();
        assert_eq!(place(0).len(), 1);
        assert_eq!((place(0)[0].line, place(0)[0].column), (2, 4));
        assert_eq!(place(0)[0].path, dir.join("src/main.rs"));
        assert_eq!(&transcript.lines()[0].text[place(0)[0].start..place(0)[0].end], "src/main.rs:3:5");
        assert_eq!((place(1)[0].path.clone(), place(1)[0].line), (dir.join("app.py"), 11));
        assert_eq!((place(2)[0].line, place(2)[0].column), (3, 6));
        assert!(place(3).is_empty(), "a version and a file that is not there are not places");
        // Walked round the end, like findings.
        let first = transcript.neighbouring_place(None, true).unwrap().0;
        assert_eq!(first, 0);
        assert_eq!(transcript.neighbouring_place(Some((0, 6)), true).unwrap().0, 1, "from the first place, the next");
        assert_eq!(transcript.neighbouring_place(Some((2, 0)), true).unwrap().0, 0, "round the end");
        assert_eq!(transcript.neighbouring_place(Some((0, 6)), false).unwrap().0, 2, "back from the first, round to the last");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changes_are_given_once_from_the_first_line_touched() {
        let mut transcript = Transcript::new(Path::new("/"));
        transcript.feed(b"one\ntwo");
        let changes = transcript.take().unwrap();
        assert_eq!((changes.first, changes.lines.len()), (0, 2));
        assert!(transcript.take().is_none());
        transcript.feed(b" more\nthree\n");
        let changes = transcript.take().unwrap();
        assert_eq!(changes.first, 1, "the line that was still being written");
        let json = changes_json(&changes);
        assert_eq!(json["lines"][0]["text"], "two more");
        transcript.note("exited with 0");
        assert_eq!(transcript.lines().iter().rev().nth(1).unwrap().text, "exited with 0");
    }
}
