//! Snippet expansion and tabstops.
//!
//! A language server answers a completion with a body like
//! `frob(${1:x}, ${2:y})$0`. Expanding it means two things: turning that
//! into the text `frob(x, y)`, and remembering where the placeholders
//! landed so Tab can walk them.
//!
//! The remembering is the hard half. The moment the first placeholder is
//! typed over, every offset after it moves, so the stops cannot be plain
//! numbers handed to the shell once. They are live regions, kept by
//! [`Session`] and shifted by [`Session::adjust`] from inside the
//! document's edit choke point, which every edit — typed, pasted, undone
//! — already passes through. A session that cannot express what an edit
//! did says so, and the document ends it.
//!
//! ## Syntax
//!
//! What LSP calls a snippet, in the subset servers actually emit:
//!
//! * `$1`, `${1}` — a tabstop with nothing in it.
//! * `${1:text}` — a tabstop with a placeholder, nestable:
//!   `${1:frob(${2:x})}` is a stop containing a stop.
//! * `${1|a,b,c|}` — a choice; the first is inserted, since there is no
//!   picker to offer the rest.
//! * `$0`, `${0}` — where the caret goes when the walk is over. Always
//!   last however early it is written.
//! * The same number twice — `${1:name} = ${1:name}` — is one stop with
//!   two regions, mirrored as it is typed.
//! * `$NAME`, `${NAME}`, `${NAME:default}` — a variable, resolved by the
//!   caller; unresolved, it leaves its default, or nothing.
//! * `\$`, `\}`, `\\` — the character itself.
//!
//! Offsets throughout are UTF-16 code units, the unit both shells count
//! text in.

/// A live span of the document, in UTF-16 code units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub start: usize,
    pub end: usize,
}

impl Region {
    fn shifted(self, delta: isize) -> Self {
        Self {
            start: (self.start as isize + delta).max(0) as usize,
            end: (self.end as isize + delta).max(0) as usize,
        }
    }
}

/// One tabstop: its number, and every place in the text that carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
    pub number: u32,
    pub regions: Vec<Region>,
}

/// The result of expanding a snippet body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    /// The text to insert, with every construct resolved.
    pub text: String,
    /// The stops, in the order Tab should visit them: ascending by
    /// number, with `$0` last. Regions are relative to `text`.
    pub stops: Vec<Stop>,
}

/// Expands a snippet body. `variables` is asked for each `$NAME`; it
/// returns `None` for names it does not know, and the name's default (or
/// nothing) is used instead.
pub fn expand(body: &str, variables: &dyn Fn(&str) -> Option<String>) -> Expansion {
    let mut parser = Parser {
        chars: body.chars().collect(),
        at: 0,
        out: String::with_capacity(body.len()),
        out_len: 0,
        found: Vec::new(),
        variables,
    };
    parser.parse_text(None);
    let Parser {
        out, mut found, ..
    } = parser;

    // Ascending by number, `$0` after every other, and each stop's own
    // regions in the order they were written so the first is the one the
    // others mirror.
    found.sort_by_key(|(number, region)| {
        (
            if *number == 0 { u32::MAX } else { *number },
            region.start,
        )
    });
    let mut stops: Vec<Stop> = Vec::new();
    for (number, region) in found {
        match stops.last_mut() {
            Some(stop) if stop.number == number => stop.regions.push(region),
            _ => stops.push(Stop {
                number,
                regions: vec![region],
            }),
        }
    }
    Expansion { text: out, stops }
}

struct Parser<'a> {
    chars: Vec<char>,
    at: usize,
    out: String,
    out_len: usize,
    found: Vec<(u32, Region)>,
    variables: &'a dyn Fn(&str) -> Option<String>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn push(&mut self, c: char) {
        self.out.push(c);
        self.out_len += c.len_utf16();
    }

    fn push_str(&mut self, text: &str) {
        for c in text.chars() {
            self.push(c);
        }
    }

    /// Consumes text until `until` (unconsumed) or the end.
    fn parse_text(&mut self, until: Option<char>) {
        while let Some(c) = self.peek() {
            if Some(c) == until {
                return;
            }
            match c {
                '\\' => {
                    self.at += 1;
                    match self.peek() {
                        // Only these three are escapes; anything else
                        // keeps its backslash, which is what a Windows
                        // path in a snippet needs.
                        Some(escaped @ ('$' | '}' | '\\')) => {
                            self.push(escaped);
                            self.at += 1;
                        }
                        _ => self.push('\\'),
                    }
                }
                '$' => {
                    self.at += 1;
                    if !self.parse_dollar() {
                        self.push('$');
                    }
                }
                _ => {
                    self.push(c);
                    self.at += 1;
                }
            }
        }
    }

    /// Parses what follows a `$`. Returns false when it is not a
    /// construct at all, so the caller can emit a literal dollar.
    fn parse_dollar(&mut self) -> bool {
        match self.peek() {
            Some('{') => {
                let brace = self.at;
                self.at += 1;
                if self.parse_braced() {
                    true
                } else {
                    // Not a construct after all: rewind so the brace is
                    // read as the ordinary character it is.
                    self.at = brace;
                    false
                }
            }
            Some(c) if c.is_ascii_digit() => {
                let number = self.take_number();
                let at = self.out_len;
                self.found.push((
                    number,
                    Region {
                        start: at,
                        end: at,
                    },
                ));
                true
            }
            Some(c) if is_name_start(c) => {
                let name = self.take_name();
                if let Some(value) = (self.variables)(&name) {
                    self.push_str(&value);
                }
                true
            }
            _ => false,
        }
    }

    /// Parses the inside of `${...}`, the opening brace consumed.
    fn parse_braced(&mut self) -> bool {
        let opened_at = self.at;
        if matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            let number = self.take_number();
            match self.peek() {
                Some('}') => {
                    self.at += 1;
                    let at = self.out_len;
                    self.found.push((
                        number,
                        Region {
                            start: at,
                            end: at,
                        },
                    ));
                    true
                }
                Some(':') => {
                    self.at += 1;
                    let start = self.out_len;
                    // Recursive: a placeholder may hold further stops,
                    // and they belong to the document just as much.
                    self.parse_text(Some('}'));
                    self.at += 1; // the closing brace, or the end
                    self.found.push((
                        number,
                        Region {
                            start,
                            end: self.out_len,
                        },
                    ));
                    true
                }
                Some('|') => {
                    self.at += 1;
                    let start = self.out_len;
                    let first = self.take_first_choice();
                    self.push_str(&first);
                    self.found.push((
                        number,
                        Region {
                            start,
                            end: self.out_len,
                        },
                    ));
                    true
                }
                // `${1/regex/format/}`: a transform of another stop's
                // text. Nothing here evaluates regexes, and printing the
                // source would be worse than printing nothing, so the
                // stop is kept and left empty.
                Some('/') => {
                    self.skip_to_close();
                    let at = self.out_len;
                    self.found.push((
                        number,
                        Region {
                            start: at,
                            end: at,
                        },
                    ));
                    true
                }
                _ => {
                    self.at = opened_at;
                    false
                }
            }
        } else if matches!(self.peek(), Some(c) if is_name_start(c)) {
            let name = self.take_name();
            match self.peek() {
                Some('}') => {
                    self.at += 1;
                    if let Some(value) = (self.variables)(&name) {
                        self.push_str(&value);
                    }
                    true
                }
                Some(':') => {
                    self.at += 1;
                    match (self.variables)(&name) {
                        Some(value) => {
                            self.push_str(&value);
                            self.skip_to_close();
                        }
                        // The default is itself a snippet body, so a
                        // stop written inside one counts.
                        None => {
                            self.parse_text(Some('}'));
                            self.at += 1;
                        }
                    }
                    true
                }
                Some('/') => {
                    self.skip_to_close();
                    if let Some(value) = (self.variables)(&name) {
                        self.push_str(&value);
                    }
                    true
                }
                _ => {
                    self.at = opened_at;
                    false
                }
            }
        } else {
            self.at = opened_at;
            false
        }
    }

    fn take_number(&mut self) -> u32 {
        let mut digits = String::new();
        while let Some(c) = self.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            digits.push(c);
            self.at += 1;
        }
        // Saturating: a number nobody can type is still a stop, and
        // clamping keeps it one instead of dropping it.
        digits.parse().unwrap_or(u32::MAX - 1)
    }

    fn take_name(&mut self) -> String {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if !(c.is_ascii_alphanumeric() || c == '_') {
                break;
            }
            name.push(c);
            self.at += 1;
        }
        name
    }

    /// Reads up to the first `,` or the closing `|`, then skips the rest
    /// of the choices and the brace.
    fn take_first_choice(&mut self) -> String {
        let mut choice = String::new();
        while let Some(c) = self.peek() {
            match c {
                ',' | '|' => break,
                '\\' => {
                    self.at += 1;
                    if let Some(escaped) = self.peek() {
                        choice.push(escaped);
                        self.at += 1;
                    }
                }
                _ => {
                    choice.push(c);
                    self.at += 1;
                }
            }
        }
        self.skip_to_close();
        choice
    }

    /// Skips to just past the `}` that closes the construct being
    /// parsed, honouring nesting and escapes.
    fn skip_to_close(&mut self) {
        let mut depth = 1usize;
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '\\' => self.at += 1,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// A snippet being filled in: the stops that are left, where the walk
/// has got to, and how much of the document the snippet covers.
#[derive(Debug, Clone)]
pub struct Session {
    stops: Vec<Stop>,
    current: usize,
    extent: Region,
    /// Which region of the current stop an edit just landed in, waiting
    /// for the document to copy it to that stop's other regions.
    pending_mirror: Option<usize>,
    /// Set while those copies are being applied, so they do not queue
    /// mirrors of their own.
    mirroring: bool,
}

impl Session {
    /// Starts a session for an expansion inserted at `origin`. Returns
    /// `None` when there is nothing to walk — no stops, or only `$0`,
    /// which is a caret position and not a mode.
    pub fn begin(expansion: &Expansion, origin: usize, inserted_len: usize) -> Option<Self> {
        if expansion.stops.iter().all(|stop| stop.number == 0) {
            return None;
        }
        let stops = expansion
            .stops
            .iter()
            .map(|stop| Stop {
                number: stop.number,
                regions: stop
                    .regions
                    .iter()
                    .map(|region| Region {
                        start: region.start + origin,
                        end: region.end + origin,
                    })
                    .collect(),
            })
            .collect();
        Some(Self {
            stops,
            current: 0,
            extent: Region {
                start: origin,
                end: origin + inserted_len,
            },
            pending_mirror: None,
            mirroring: false,
        })
    }

    /// The region the caret should occupy right now: the current stop's
    /// first region, selected so typing replaces it.
    pub fn current_region(&self) -> Region {
        self.stops
            .get(self.current)
            .and_then(|stop| stop.regions.first().copied())
            .unwrap_or(Region {
                start: self.extent.end,
                end: self.extent.end,
            })
    }

    /// Whether the walk has reached the last stop, so Tab has nothing
    /// left to move to.
    pub fn is_last(&self) -> bool {
        self.current + 1 >= self.stops.len()
    }

    /// Whether the current stop is the exit point.
    pub fn at_exit(&self) -> bool {
        self.stops
            .get(self.current)
            .is_some_and(|stop| stop.number == 0)
    }

    /// Moves to the next stop (or the previous one). Returns the region
    /// to select, and whether the session is over — landing on `$0`, or
    /// running off the end, both end it.
    pub fn advance(&mut self, forward: bool) -> (Region, bool) {
        if forward {
            if self.current + 1 < self.stops.len() {
                self.current += 1;
                self.pending_mirror = None;
                return (self.current_region(), self.at_exit());
            }
            // Nothing left: the caret goes to the end of what was
            // inserted, and the keys go back to the text view.
            (
                Region {
                    start: self.extent.end,
                    end: self.extent.end,
                },
                true,
            )
        } else {
            self.current = self.current.saturating_sub(1);
            self.pending_mirror = None;
            (self.current_region(), false)
        }
    }

    /// The extent of the snippet in the document.
    pub fn extent(&self) -> Region {
        self.extent
    }

    /// Whether a caret at `position` is still inside the snippet. A
    /// caret that has left it has moved on, and so should the session.
    pub fn contains_caret(&self, position: usize) -> bool {
        position >= self.extent.start && position <= self.extent.end
    }

    /// Folds an edit into every live region. Returns false when the
    /// edit straddles a region's boundary, which no shifting can
    /// describe — the caller ends the session rather than track regions
    /// that have stopped meaning anything.
    pub fn adjust(&mut self, start: usize, end: usize, new_len: usize) -> bool {
        let delta = new_len as isize - (end - start) as isize;
        let current = self.current;
        let mut landed_in = None;

        for (index, stop) in self.stops.iter_mut().enumerate() {
            for (region_index, region) in stop.regions.iter_mut().enumerate() {
                // A stop's own regions claim edits on their boundaries:
                // typing at the end of the placeholder you are filling
                // extends it, rather than falling outside it.
                let claims_boundary = index == current;
                match adjusted(*region, start, end, delta, claims_boundary) {
                    Fall::Before(next) | Fall::After(next) => *region = next,
                    Fall::Inside(next) => {
                        *region = next;
                        if index == current {
                            landed_in = Some(region_index);
                        }
                    }
                    Fall::Across => return false,
                }
            }
        }

        self.extent = match adjusted(self.extent, start, end, delta, true) {
            Fall::Before(next) | Fall::After(next) | Fall::Inside(next) => next,
            Fall::Across => return false,
        };
        if !self.mirroring {
            if let Some(region_index) = landed_in {
                self.pending_mirror = Some(region_index);
            }
        }
        true
    }

    /// The mirroring an edit left to do: the region that changed, and
    /// the sibling regions that should be made to match it. Clears the
    /// pending state, so asking twice yields nothing the second time.
    pub fn take_mirror(&mut self) -> Option<(Region, Vec<Region>)> {
        let region_index = self.pending_mirror.take()?;
        let stop = self.stops.get(self.current)?;
        if stop.regions.len() < 2 || stop.number == 0 {
            return None;
        }
        let source = *stop.regions.get(region_index)?;
        let targets = stop
            .regions
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != region_index)
            .map(|(_, region)| *region)
            .collect();
        Some((source, targets))
    }

    /// Marks the start and end of applying mirror copies, so they do not
    /// queue mirrors of their own.
    pub fn set_mirroring(&mut self, mirroring: bool) {
        self.mirroring = mirroring;
        if !mirroring {
            self.pending_mirror = None;
        }
    }
}

/// Where an edit fell relative to one region.
enum Fall {
    /// Entirely before it: the region moves by the edit's delta.
    Before(Region),
    /// Entirely after it: the region is untouched.
    After(Region),
    /// Within it: the region grew or shrank around the new text.
    Inside(Region),
    /// Across one of its boundaries: no region describes what is left.
    Across,
}

/// One region under one edit. `claims_boundary` decides who an
/// insertion sitting exactly on an edge belongs to: the stop being
/// typed in takes it, everything else lets it pass by.
fn adjusted(region: Region, start: usize, end: usize, delta: isize, claims_boundary: bool) -> Fall {
    let claims = claims_boundary && start == end;
    if end < region.start || (end == region.start && !claims) {
        Fall::Before(region.shifted(delta))
    } else if start > region.end || (start == region.end && !claims) {
        Fall::After(region)
    } else if start >= region.start && end <= region.end {
        Fall::Inside(Region {
            start: region.start,
            end: (region.end as isize + delta).max(region.start as isize) as usize,
        })
    } else {
        Fall::Across
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_variables(_: &str) -> Option<String> {
        None
    }

    fn expand_plain(body: &str) -> Expansion {
        expand(body, &no_variables)
    }

    fn region(start: usize, end: usize) -> Region {
        Region { start, end }
    }

    #[test]
    fn placeholders_expand_and_are_numbered() {
        let expansion = expand_plain("frob(${1:x}, ${2:y})$0");
        assert_eq!(expansion.text, "frob(x, y)");
        assert_eq!(
            expansion.stops,
            vec![
                Stop {
                    number: 1,
                    regions: vec![region(5, 6)]
                },
                Stop {
                    number: 2,
                    regions: vec![region(8, 9)]
                },
                Stop {
                    number: 0,
                    regions: vec![region(10, 10)]
                },
            ]
        );
    }

    #[test]
    fn the_exit_stop_sorts_last_however_early_it_is_written() {
        let expansion = expand_plain("$0 then ${1:a}");
        assert_eq!(expansion.text, " then a");
        assert_eq!(expansion.stops[0].number, 1);
        assert_eq!(expansion.stops[1].number, 0);
    }

    #[test]
    fn the_same_number_twice_is_one_stop_with_two_regions() {
        let expansion = expand_plain("${1:name} = ${1:name};");
        assert_eq!(expansion.text, "name = name;");
        assert_eq!(expansion.stops.len(), 1);
        assert_eq!(expansion.stops[0].regions, vec![region(0, 4), region(7, 11)]);
    }

    #[test]
    fn placeholders_nest() {
        let expansion = expand_plain("if (${1:${2:cond}}) {}");
        assert_eq!(expansion.text, "if (cond) {}");
        assert_eq!(expansion.stops[0].regions, vec![region(4, 8)]);
        assert_eq!(expansion.stops[1].regions, vec![region(4, 8)]);
    }

    #[test]
    fn a_choice_inserts_its_first_option() {
        let expansion = expand_plain("let ${1|mut ,|}x");
        assert_eq!(expansion.text, "let mut x");
        assert_eq!(expansion.stops[0].regions, vec![region(4, 8)]);
    }

    #[test]
    fn escapes_survive_and_lone_dollars_stay() {
        assert_eq!(expand_plain("cost \\$5").text, "cost $5");
        assert_eq!(expand_plain("a \\} b").text, "a } b");
        assert_eq!(expand_plain("C:\\\\path").text, "C:\\path");
        assert_eq!(expand_plain("100% $ down").text, "100% $ down");
        assert_eq!(expand_plain("${}").text, "${}");
    }

    #[test]
    fn variables_resolve_or_leave_their_default() {
        let expansion = expand("// ${TM_FILENAME} ${NOPE:fallback}", &|name| {
            (name == "TM_FILENAME").then(|| "main.rs".to_owned())
        });
        assert_eq!(expansion.text, "// main.rs fallback");
    }

    #[test]
    fn a_stop_inside_an_unresolved_variable_default_still_counts() {
        let expansion = expand_plain("${NOPE:${1:here}}");
        assert_eq!(expansion.text, "here");
        assert_eq!(expansion.stops[0].regions, vec![region(0, 4)]);
    }

    #[test]
    fn utf16_offsets_count_code_units_not_characters() {
        let expansion = expand_plain("🙂${1:x}");
        assert_eq!(expansion.stops[0].regions, vec![region(2, 3)]);
    }

    fn session_for(body: &str, origin: usize) -> (String, Session) {
        let expansion = expand_plain(body);
        let len = expansion.text.encode_utf16().count();
        let session = Session::begin(&expansion, origin, len).expect("a session");
        (expansion.text, session)
    }

    #[test]
    fn a_session_starts_on_the_first_stop_and_walks_to_the_exit() {
        let (_, mut session) = session_for("frob(${1:x}, ${2:y})$0", 10);
        assert_eq!(session.current_region(), region(15, 16));
        assert_eq!(session.advance(true), (region(18, 19), false));
        assert_eq!(session.advance(true), (region(20, 20), true));
    }

    #[test]
    fn shift_tab_walks_back_and_stops_at_the_first() {
        let (_, mut session) = session_for("${1:a} ${2:b}", 0);
        session.advance(true);
        assert_eq!(session.advance(false), (region(0, 1), false));
        assert_eq!(session.advance(false), (region(0, 1), false));
    }

    #[test]
    fn a_snippet_of_only_an_exit_point_is_not_a_session() {
        let expansion = expand_plain("done()$0");
        assert!(Session::begin(&expansion, 0, 6).is_none());
    }

    #[test]
    fn typing_in_a_stop_moves_the_ones_after_it() {
        // "frob(x, y)" with x at 5..6 and y at 8..9.
        let (_, mut session) = session_for("frob(${1:x}, ${2:y})$0", 0);
        // Replace "x" with "count".
        assert!(session.adjust(5, 6, 5));
        assert_eq!(session.current_region(), region(5, 10));
        assert_eq!(session.advance(true), (region(12, 13), false));
        assert_eq!(session.extent(), region(0, 14));
    }

    #[test]
    fn typing_at_the_end_of_the_current_placeholder_extends_it() {
        let (_, mut session) = session_for("${1:ab} ${2:c}", 0);
        assert!(session.adjust(2, 2, 1));
        assert_eq!(session.current_region(), region(0, 3));
    }

    #[test]
    fn typing_at_the_start_of_a_later_stop_does_not_join_it() {
        // "ab c", with stop 2 the single character at 3..4.
        let (_, mut session) = session_for("${1:ab} ${2:c}", 0);
        // An insertion where stop 2 begins belongs to whatever is being
        // typed outside it — stop 2 shifts along instead of swallowing
        // the character.
        assert!(session.adjust(3, 3, 1));
        session.advance(true);
        assert_eq!(session.current_region(), region(4, 5));
    }

    #[test]
    fn an_edit_across_a_boundary_ends_the_session() {
        let (_, mut session) = session_for("frob(${1:x}, ${2:y})$0", 0);
        // Selecting "x, y" and typing over it: no arrangement of the
        // stops describes what is left.
        assert!(!session.adjust(5, 9, 1));
    }

    #[test]
    fn a_caret_outside_the_snippet_has_left_it() {
        // "abc" inserted at 4, so the snippet occupies 4..7.
        let (_, session) = session_for("${1:a}bc", 4);
        assert!(session.contains_caret(4));
        assert!(session.contains_caret(7));
        assert!(!session.contains_caret(3));
        assert!(!session.contains_caret(8));
    }

    #[test]
    fn editing_a_linked_stop_asks_for_its_twin_to_be_updated() {
        let (_, mut session) = session_for("${1:name} = ${1:name};", 0);
        assert!(session.adjust(0, 4, 3));
        let (source, targets) = session.take_mirror().expect("a mirror");
        assert_eq!(source, region(0, 3));
        assert_eq!(targets, vec![region(6, 10)]);
        assert!(session.take_mirror().is_none());
    }

    #[test]
    fn a_stop_with_one_region_asks_for_no_mirroring() {
        let (_, mut session) = session_for("${1:a} ${2:b}", 0);
        assert!(session.adjust(0, 1, 3));
        assert!(session.take_mirror().is_none());
    }

    #[test]
    fn undoing_the_whole_snippet_leaves_nothing_to_track() {
        let (text, mut session) = session_for("${1:a} ${2:b}", 0);
        let len = text.encode_utf16().count();
        assert!(!session.adjust(0, len, 0));
    }
}
