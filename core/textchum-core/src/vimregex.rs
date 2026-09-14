//! Find and replace in Vim's regular-expression dialect.
//!
//! Vim's patterns differ from the usual engines in what a backslash
//! means: in the default ("magic") mode, `\(` `\)` group, `\|` alternates,
//! `\+` `\?` `\=` `\{n,m}` repeat, `\<` `\>` bound words, while a bare
//! `(` `)` `|` `+` `?` `{` is itself. `\v` at the start turns everything
//! on (very magic), `\V` turns everything off but the backslash. `\c`
//! anywhere ignores case, `\C` minds it. The pattern is translated to
//! the syntax of the `regex` crate, which does the matching.
//!
//! Replacements take Vim's specials too: `&` and `\0` for the match,
//! `\1`…`\9` for groups, `\n` `\t` `\\` `\&`, and `\u` `\l` `\U` `\L` `\E`
//! for case.

use regex::{Captures, Regex};

/// How a pattern is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The pattern is a regular expression; otherwise it is literal text.
    pub regex: bool,
    /// Letters match their own case only. `\c` and `\C` in a regular
    /// expression override this.
    pub case_sensitive: bool,
    /// The match must sit on word boundaries on both sides.
    pub whole_word: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Magic {
    VeryNo,
    No,
    Yes,
    Very,
}

/// Translates a Vim pattern into the `regex` crate's syntax. `Err` names
/// what could not be read.
pub fn translate(pattern: &str) -> Result<(String, Option<bool>), String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut mode = Magic::Yes;
    let mut ignore_case: Option<bool> = None;
    let mut index = 0;
    // `^` is an anchor only where Vim lets it be: at the start, after a
    // group opens, or after an alternation.
    let mut at_branch_start = true;
    while index < chars.len() {
        let character = chars[index];
        index += 1;
        let escaped = character == '\\';
        let next = if escaped {
            match chars.get(index) {
                Some(&n) => {
                    index += 1;
                    n
                }
                None => return Err("a backslash with nothing after it".into()),
            }
        } else {
            character
        };
        // Whether this char, with or without its backslash, acts as the
        // special meaning Vim gives it in the current mode.
        let special = |c: char, escaped: bool| -> bool {
            match mode {
                Magic::Very => !escaped && !c.is_ascii_alphanumeric() && c != '_',
                Magic::Yes => match c {
                    '^' | '$' | '.' | '*' | '[' | '~' => !escaped,
                    _ => escaped,
                },
                Magic::No => match c {
                    '^' | '$' => !escaped,
                    _ => escaped,
                },
                Magic::VeryNo => escaped,
            }
        };
        let was_branch_start = at_branch_start;
        at_branch_start = false;
        if escaped {
            match next {
                'v' => {
                    mode = Magic::Very;
                    at_branch_start = was_branch_start;
                    continue;
                }
                'm' => {
                    mode = Magic::Yes;
                    at_branch_start = was_branch_start;
                    continue;
                }
                'M' => {
                    mode = Magic::No;
                    at_branch_start = was_branch_start;
                    continue;
                }
                'V' => {
                    mode = Magic::VeryNo;
                    at_branch_start = was_branch_start;
                    continue;
                }
                'c' => {
                    ignore_case = Some(true);
                    at_branch_start = was_branch_start;
                    continue;
                }
                'C' => {
                    ignore_case = Some(false);
                    at_branch_start = was_branch_start;
                    continue;
                }
                _ => {}
            }
        }
        let is_special = special(next, escaped);
        match next {
            '(' if is_special => {
                out.push('(');
                at_branch_start = true;
            }
            ')' if is_special => out.push(')'),
            '|' if is_special => {
                out.push('|');
                at_branch_start = true;
            }
            '+' if is_special => out.push('+'),
            '?' | '=' if is_special => out.push('?'),
            '{' if is_special => {
                // `\{n,m}`, `\{-n,m}` (lazy), closed by `}` or `\}`.
                let mut body = String::new();
                while index < chars.len() && chars[index] != '}' {
                    if chars[index] == '\\' && chars.get(index + 1) == Some(&'}') {
                        index += 1;
                        break;
                    }
                    body.push(chars[index]);
                    index += 1;
                }
                if index < chars.len() && chars[index] == '}' {
                    index += 1;
                }
                let lazy = body.starts_with('-');
                let body = body.trim_start_matches('-');
                let body = if body.is_empty() { "0," } else { body };
                out.push('{');
                out.push_str(body);
                out.push('}');
                if lazy {
                    out.push('?');
                }
            }
            '<' | '>' if is_special => out.push_str("\\b"),
            '^' if is_special => {
                if was_branch_start {
                    out.push('^');
                } else {
                    out.push_str("\\^");
                }
            }
            '$' if is_special => {
                let at_end = index >= chars.len()
                    || (chars[index] == '\\' && matches!(chars.get(index + 1), Some('|') | Some(')')))
                    || (mode == Magic::Very && matches!(chars[index], '|' | ')'));
                if at_end {
                    out.push('$');
                } else {
                    out.push_str("\\$");
                }
            }
            '.' if is_special => out.push('.'),
            '*' if is_special => out.push('*'),
            '~' if is_special => out.push('~'),
            '[' if is_special => {
                // A class runs to its `]`; without one, the bracket is
                // itself, as in Vim.
                if let Some(close) = find_class_end(&chars, index) {
                    out.push('[');
                    let inner: String = chars[index..close].iter().collect();
                    out.push_str(&inner);
                    out.push(']');
                    index = close + 1;
                } else {
                    out.push_str("\\[");
                }
            }
            '%' if escaped => {
                // `\%(` opens a group that does not capture.
                if chars.get(index) == Some(&'(') {
                    index += 1;
                    out.push_str("(?:");
                    at_branch_start = true;
                } else {
                    return Err(format!("\\%{} is not supported", chars.get(index).map(|c| c.to_string()).unwrap_or_default()));
                }
            }
            's' if escaped => out.push_str("\\s"),
            'S' if escaped => out.push_str("\\S"),
            'd' if escaped => out.push_str("\\d"),
            'D' if escaped => out.push_str("\\D"),
            'w' if escaped => out.push_str("\\w"),
            'W' if escaped => out.push_str("\\W"),
            'a' if escaped => out.push_str("[A-Za-z]"),
            'A' if escaped => out.push_str("[^A-Za-z]"),
            'l' if escaped => out.push_str("[a-z]"),
            'L' if escaped => out.push_str("[^a-z]"),
            'u' if escaped => out.push_str("[A-Z]"),
            'U' if escaped => out.push_str("[^A-Z]"),
            'x' if escaped => out.push_str("[0-9A-Fa-f]"),
            'X' if escaped => out.push_str("[^0-9A-Fa-f]"),
            'o' if escaped => out.push_str("[0-7]"),
            'O' if escaped => out.push_str("[^0-7]"),
            'h' if escaped => out.push_str("[A-Za-z_]"),
            'H' if escaped => out.push_str("[^A-Za-z_]"),
            'n' if escaped => out.push('\n'),
            't' if escaped => out.push('\t'),
            'r' if escaped => out.push('\r'),
            'e' if escaped => out.push('\u{1b}'),
            _ => {
                // Itself — a very-magic char with no Vim meaning of its
                // own included — escaped where the engine would read it.
                out.push_str(&regex::escape(&next.to_string()));
            }
        }
    }
    Ok((out, ignore_case))
}

fn find_class_end(chars: &[char], from: usize) -> Option<usize> {
    let mut index = from;
    // A `]` right after `[` or `[^` is a member, not the end.
    if chars.get(index) == Some(&'^') {
        index += 1;
    }
    if chars.get(index) == Some(&']') {
        index += 1;
    }
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 2,
            '[' if chars.get(index + 1) == Some(&':') => {
                // `[:alpha:]` and friends run to `:]`.
                let mut inner = index + 2;
                while inner + 1 < chars.len() && !(chars[inner] == ':' && chars[inner + 1] == ']') {
                    inner += 1;
                }
                index = inner + 2;
            }
            ']' => return Some(index),
            _ => index += 1,
        }
    }
    None
}

/// The engine's pattern for `pattern` under `options`.
pub fn compile(pattern: &str, options: Options) -> Result<Regex, String> {
    let (body, case_override) = if options.regex {
        translate(pattern)?
    } else {
        (regex::escape(pattern), None)
    };
    let mut source = String::new();
    let ignore_case = case_override.unwrap_or(!options.case_sensitive);
    if ignore_case {
        source.push_str("(?i)");
    }
    if options.whole_word {
        source.push_str("\\b(?:");
        source.push_str(&body);
        source.push_str(")\\b");
    } else {
        source.push_str(&body);
    }
    Regex::new(&source).map_err(|error| error.to_string())
}

/// Every match in `text`, as UTF-16 unit ranges in order.
pub fn find_all(text: &str, pattern: &str, options: Options) -> Result<Vec<(usize, usize)>, String> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let re = compile(pattern, options)?;
    let mut ranges = Vec::new();
    let mut byte = 0usize;
    let mut unit = 0usize;
    for found in re.find_iter(text) {
        // Walk forward from the last match: byte offset to UTF-16 units.
        unit += text[byte..found.start()].encode_utf16().count();
        byte = found.start();
        let start = unit;
        unit += text[byte..found.end()].encode_utf16().count();
        byte = found.end();
        ranges.push((start, unit));
    }
    Ok(ranges)
}

/// `replacement` with Vim's specials filled in from `captures`.
pub fn expand(replacement: &str, captures: &Captures<'_>) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum Case {
        None,
        Upper,
        Lower,
    }
    let mut out = String::new();
    let mut running = Case::None;
    let mut next_one: Option<Case> = None;
    let push = |out: &mut String, piece: &str, running: Case, next_one: &mut Option<Case>| {
        for character in piece.chars() {
            let cased: String = match next_one.take().unwrap_or(running) {
                Case::Upper => character.to_uppercase().collect(),
                Case::Lower => character.to_lowercase().collect(),
                Case::None => character.to_string(),
            };
            out.push_str(&cased);
        }
    };
    let chars: Vec<char> = replacement.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        index += 1;
        match character {
            '&' => {
                let whole = captures.get(0).map(|m| m.as_str()).unwrap_or("");
                push(&mut out, whole, running, &mut next_one);
            }
            '\\' => {
                let Some(&next) = chars.get(index) else {
                    out.push('\\');
                    break;
                };
                index += 1;
                match next {
                    '0'..='9' => {
                        let group = captures
                            .get(next as usize - '0' as usize)
                            .map(|m| m.as_str())
                            .unwrap_or("");
                        push(&mut out, group, running, &mut next_one);
                    }
                    '&' => push(&mut out, "&", running, &mut next_one),
                    '\\' => push(&mut out, "\\", running, &mut next_one),
                    'n' | 'r' => out.push('\n'),
                    't' => out.push('\t'),
                    'u' => next_one = Some(Case::Upper),
                    'l' => next_one = Some(Case::Lower),
                    'U' => running = Case::Upper,
                    'L' => running = Case::Lower,
                    'E' | 'e' => running = Case::None,
                    other => push(&mut out, &other.to_string(), running, &mut next_one),
                }
            }
            other => push(&mut out, &other.to_string(), running, &mut next_one),
        }
    }
    out
}

/// What `replacement` says for the `index`-th match of `pattern` in
/// `text`, or `None` when there is no such match.
pub fn expansion_for_match(
    text: &str,
    pattern: &str,
    replacement: &str,
    options: Options,
    index: usize,
) -> Result<Option<String>, String> {
    if pattern.is_empty() {
        return Ok(None);
    }
    let re = compile(pattern, options)?;
    // Bound before the tail: a tail expression's temporaries outlive
    // the locals, and the iterator borrows the engine.
    let expanded: Option<String> = match re.captures_iter(text).nth(index) {
        // Literal text: the specials are letters like any other.
        Some(_) if !options.regex => Some(replacement.to_string()),
        Some(captures) => Some(expand(replacement, &captures)),
        None => None,
    };
    Ok(expanded)
}

/// `text` with every match replaced, and how many there were.
pub fn replace_all(
    text: &str,
    pattern: &str,
    replacement: &str,
    options: Options,
) -> Result<(String, usize), String> {
    if pattern.is_empty() {
        return Ok((text.to_string(), 0));
    }
    let re = compile(pattern, options)?;
    let mut count = 0;
    let replaced = re.replace_all(text, |captures: &Captures<'_>| {
        count += 1;
        if options.regex {
            expand(replacement, captures)
        } else {
            replacement.to_string()
        }
    });
    Ok((replaced.into_owned(), count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn magic(pattern: &str) -> String {
        translate(pattern).expect("translates").0
    }

    #[test]
    fn magic_mode_reads_backslashes_the_vim_way() {
        assert_eq!(magic(r"\(ab\)\+"), "(ab)+");
        assert_eq!(magic(r"a\|b"), "a|b");
        assert_eq!(magic(r"\<word\>"), r"\bword\b");
        assert_eq!(magic(r"x\{2,3}"), "x{2,3}");
        assert_eq!(magic(r"x\{-1,}"), "x{1,}?");
        assert_eq!(magic(r"f(x)"), r"f\(x\)");
        assert_eq!(magic(r"a+b"), r"a\+b");
        assert_eq!(magic(r"\d\+\s\w"), r"\d+\s\w");
        assert_eq!(magic(r"^a.*b$"), "^a.*b$");
        assert_eq!(magic(r"a^b"), r"a\^b");
        assert_eq!(magic(r"[a-z]\+"), "[a-z]+");
        assert_eq!(magic(r"\%(a\|b\)c"), "(?:a|b)c");
    }

    #[test]
    fn very_magic_and_very_nomagic() {
        assert_eq!(magic(r"\v(a|b)+"), "(a|b)+");
        assert_eq!(magic(r"\v<word>"), r"\bword\b");
        assert_eq!(magic(r"\Va.b"), r"a\.b");
        assert_eq!(magic(r"\Va\.b"), "a.b");
    }

    #[test]
    fn case_flags_and_options() {
        let (_, flag) = translate(r"\cfoo").unwrap();
        assert_eq!(flag, Some(true));
        let options = Options { regex: true, case_sensitive: true, whole_word: false };
        assert_eq!(find_all("Foo foo", r"\cfoo", options).unwrap(), vec![(0, 3), (4, 7)]);
        assert_eq!(find_all("Foo foo", "foo", options).unwrap(), vec![(4, 7)]);
        let loose = Options { regex: false, case_sensitive: false, whole_word: true };
        assert_eq!(find_all("food foo Foo", "foo", loose).unwrap(), vec![(5, 8), (9, 12)]);
    }

    #[test]
    fn offsets_are_utf16_units() {
        let options = Options { regex: true, case_sensitive: true, whole_word: false };
        // 😀 is two UTF-16 units; the match after it starts at 3.
        assert_eq!(find_all("😀 ab", "ab", options).unwrap(), vec![(3, 5)]);
    }

    #[test]
    fn replacements_take_groups_and_case() {
        let options = Options { regex: true, case_sensitive: true, whole_word: false };
        let (text, count) =
            replace_all("john smith", r"\(\w\+\) \(\w\+\)", r"\u\2, \U\1\E!", options).unwrap();
        assert_eq!(text, "Smith, JOHN!");
        assert_eq!(count, 1);
        let (text, _) = replace_all("a-b", "-", r"[&]", options).unwrap();
        assert_eq!(text, "a[-]b");
        let (text, _) = replace_all("a-b", "-", r"\&\\\n", options).unwrap();
        assert_eq!(text, "a&\\\nb");
        let one = expansion_for_match("x1 x2", r"x\(\d\)", r"y\1", options, 1).unwrap();
        assert_eq!(one.as_deref(), Some("y2"));
        let literal = Options { regex: false, case_sensitive: true, whole_word: false };
        let (text, _) = replace_all("a.b", ".", r"\1&", literal).unwrap();
        assert_eq!(text, r"a\1&b");
    }

    #[test]
    fn bad_patterns_say_so() {
        let options = Options { regex: true, case_sensitive: true, whole_word: false };
        assert!(find_all("x", r"\(", options).is_err());
        assert!(find_all("x", r"a\", options).is_err());
    }
}
