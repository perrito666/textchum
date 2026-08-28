//! Prose spell check, scoped the way the macOS shell scopes it: only
//! comments in code, and whole documents for Markdown, git commit
//! messages, and plain text — identifiers are never flagged.
//!
//! The checking itself rides `hunspell` (the dictionaries most desktops
//! already have): each pass feeds the prose to `hunspell -l`, gets the
//! misspelled words back, and tags every occurrence. `editor.spell`
//! picks the dictionaries — `"auto"` follows `$LANG`, a code like
//! `"es_ES"` names one, `"en_US, es_ES"` names several (hunspell reads
//! a comma-separated list and accepts a word any of them knows), and
//! absent means off.
//!
//! `editor.spell_words` is the personal list: project names, acronyms,
//! and everything no dictionary ships with. It is applied here rather
//! than handed to hunspell as a personal-dictionary file, because the
//! configuration is the one store and a second file on disk would be a
//! second place for the same setting to live.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io::Write;
use std::process::{Command, Stdio};
use std::rc::Rc;

use sourceview5::prelude::*;

use crate::page::Page;
use crate::shell::Shell;

pub const TAG: &str = "misspell";

thread_local! {
    static WARNED_MISSING: Cell<bool> = const { Cell::new(false) };
    /// Words the user waved through for this session only — the middle
    /// ground between "this is a word" (the personal list, which is
    /// saved) and "fix it" (a replacement).
    static IGNORED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// The word the open spelling menu is about: its text and the
    /// character offsets it occupied when the menu was built.
    static MENU_TARGET: RefCell<Option<(String, i32, i32)>> = const { RefCell::new(None) };
}

/// Records which word the spelling menu being built refers to.
pub fn note_menu_target(word: &str, start: i32, end: i32) {
    MENU_TARGET.with(|slot| *slot.borrow_mut() = Some((word.to_owned(), start, end)));
}

/// What the open spelling menu refers to, if anything.
pub fn menu_target() -> Option<(String, i32, i32)> {
    MENU_TARGET.with(|slot| slot.borrow().clone())
}

/// Installs the misspelling tag on a fresh buffer: a purple tint, so
/// spelling never reads as a diagnostic.
pub fn install_tag(buffer: &sourceview5::Buffer) {
    let tag = gtk::TextTag::new(Some(TAG));
    tag.set_background_rgba(Some(&gtk::gdk::RGBA::new(0.55, 0.36, 0.76, 0.25)));
    buffer.tag_table().add(&tag);
}

/// Accepts `word` for the rest of the session, without saving it.
pub fn ignore(word: &str) {
    IGNORED.with(|set| set.borrow_mut().insert(word.to_owned()));
}

/// Whether the word is one the user has already accepted, by the
/// personal list or by ignoring it. Matching ignores case: a personal
/// list is an allowlist someone typed, not a dictionary with rules
/// about capitalization.
fn accepted(word: &str, personal: &HashSet<String>) -> bool {
    let folded = word.to_lowercase();
    personal.contains(&folded) || IGNORED.with(|set| set.borrow().contains(word))
}

/// The personal list, folded for comparison.
fn personal_words() -> HashSet<String> {
    Shell::instance()
        .config
        .borrow()
        .spell_words()
        .iter()
        .map(|word| word.to_lowercase())
        .collect()
}

/// Runs one spell pass over the page (or clears the marks when the
/// configuration says off). Blocking on a hunspell run — call it from
/// the settle timer, not per keystroke.
pub fn run(page: &Rc<Page>) {
    let buffer = &page.buffer;
    let clear = |buffer: &sourceview5::Buffer| {
        buffer.remove_tag_by_name(TAG, &buffer.start_iter(), &buffer.end_iter());
    };
    let languages = Shell::instance().config.borrow().spell_languages();
    if languages.is_empty() {
        clear(buffer);
        return;
    }

    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true).to_string();
    let ranges = prose_char_ranges(page, &text);
    clear(buffer);
    if ranges.is_empty() {
        return;
    }
    let prose: String = {
        let characters: Vec<char> = text.chars().collect();
        ranges
            .iter()
            .map(|(start, end)| characters[*start..*end].iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let Some(misspelled) = misspelled_words(&prose, &languages) else {
        return;
    };
    if misspelled.is_empty() {
        return;
    }
    let personal = personal_words();

    // Tag every occurrence of a misspelled word inside the prose
    // ranges. Word scanning mirrors hunspell's: letter runs, with
    // apostrophes allowed inside.
    let characters: Vec<char> = text.chars().collect();
    for (start, end) in ranges {
        let mut index = start;
        while index < end {
            if !characters[index].is_alphabetic() {
                index += 1;
                continue;
            }
            let word_start = index;
            while index < end
                && (characters[index].is_alphabetic() || characters[index] == '\'')
            {
                index += 1;
            }
            let word: String = characters[word_start..index].iter().collect();
            let word = word.trim_matches('\'');
            if word.chars().count() > 1
                && misspelled.contains(word)
                && !accepted(word, &personal)
            {
                let from = buffer.iter_at_offset(word_start as i32);
                let to = buffer.iter_at_offset(index as i32);
                buffer.apply_tag_by_name(TAG, &from, &to);
            }
        }
    }
}

/// The misspelled word the character offset sits inside, with its
/// bounds — what a context menu needs to know to offer replacements.
pub fn word_at(buffer: &sourceview5::Buffer, offset: i32) -> Option<(String, i32, i32)> {
    let tag = buffer.tag_table().lookup(TAG)?;
    let mut start = buffer.iter_at_offset(offset);
    // `has_tag` is false at the very end of a tagged run, so a click
    // just past the last letter has to look one character back before
    // giving up.
    if !start.has_tag(&tag) {
        if offset == 0 {
            return None;
        }
        start = buffer.iter_at_offset(offset - 1);
        if !start.has_tag(&tag) {
            return None;
        }
    }
    let mut end = start.clone();
    if !start.starts_tag(Some(&tag)) {
        start.backward_to_tag_toggle(Some(&tag));
    }
    end.forward_to_tag_toggle(Some(&tag));
    let word = buffer.text(&start, &end, false).to_string();
    if word.is_empty() {
        return None;
    }
    Some((word, start.offset(), end.offset()))
}

/// Replacements hunspell offers for one word, best first. Empty when it
/// has none, or when hunspell is unavailable.
pub fn suggestions(word: &str) -> Vec<String> {
    let languages = Shell::instance().config.borrow().spell_languages();
    if languages.is_empty() {
        return Vec::new();
    }
    // `-a` is hunspell's pipe mode: a miss with suggestions comes back
    // as "& word count offset: first, second, …", a miss without them
    // as "# word offset", and a hit as "*" or "+ stem".
    let Some(output) = hunspell(&["-a"], word, &languages) else {
        return Vec::new();
    };
    for line in output.lines() {
        let Some(rest) = line.strip_prefix("& ") else { continue };
        let Some((_, list)) = rest.split_once(": ") else { continue };
        return list
            .split(',')
            .map(str::trim)
            .filter(|suggestion| !suggestion.is_empty())
            .map(str::to_owned)
            .collect();
    }
    Vec::new()
}

/// Where prose lives, as character ranges: everywhere for languages
/// that are prose, only inside comments for code.
fn prose_char_ranges(page: &Rc<Page>, text: &str) -> Vec<(usize, usize)> {
    let state = page.state.borrow();
    let language = state.document.language_name();
    let total_chars = text.chars().count();
    if language.is_none() || language == Some("markdown") || language == Some("gitcommit") {
        if total_chars == 0 {
            return Vec::new();
        }
        if language != Some("markdown") {
            return vec![(0, total_chars)];
        }
        // Hugo posts carry structured data and template calls among
        // the prose; a slug is not a misspelling.
        let skip = textchum_core::hugo::non_prose_ranges(text);
        if skip.is_empty() {
            return vec![(0, total_chars)];
        }
        let mut kept = Vec::new();
        let mut cursor = 0usize;
        for range in skip {
            let start = text[..range.start].chars().count();
            let end = start + text[range.clone()].chars().count();
            if start > cursor {
                kept.push((cursor, start));
            }
            cursor = cursor.max(end);
        }
        if cursor < total_chars {
            kept.push((cursor, total_chars));
        }
        return kept;
    }
    let comment_style = textchum_core::theme::resolve("comment");
    let total_utf16 = text.encode_utf16().count();
    let Ok(spans) = state.document.highlights(0, total_utf16) else {
        return Vec::new();
    };
    spans
        .iter()
        // Asked by name: style ids are positions in an alphabetical
        // table and move whenever a capture is added.
        .filter(|span| Some(span.style) == comment_style)
        .map(|span| {
            (
                crate::page::char_offset(text, span.start_utf16) as usize,
                crate::page::char_offset(text, span.end_utf16) as usize,
            )
        })
        .filter(|(start, end)| end > start)
        .collect()
}

/// The words hunspell rejects, or `None` when hunspell is unavailable
/// (warned once).
fn misspelled_words(prose: &str, languages: &[String]) -> Option<HashSet<String>> {
    let output = hunspell(&["-l"], prose, languages)?;
    Some(
        output
            .lines()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect(),
    )
}

/// Runs hunspell over `input` and returns its standard output.
///
/// The dictionaries go in as one comma-separated `-d` argument, which
/// is how hunspell takes several at once: a word any of them knows is
/// spelled correctly, which is what a bilingual document needs.
/// `"auto"` means "say nothing and let hunspell follow the locale".
fn hunspell(arguments: &[&str], input: &str, languages: &[String]) -> Option<String> {
    let mut command = Command::new("hunspell");
    command.args(arguments);
    let named: Vec<&str> = languages
        .iter()
        .map(String::as_str)
        .filter(|language| *language != "auto")
        .collect();
    if !named.is_empty() {
        command.args(["-d", &named.join(",")]);
    }
    let mut child = match command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            WARNED_MISSING.with(|warned| {
                if !warned.replace(true) {
                    if let Some(workbench) = crate::workbench::Workbench::active() {
                        workbench.explain(
                            "Spell check needs hunspell — install it (and a dictionary) \
                             or clear editor.spell.",
                        );
                    }
                }
            });
            return None;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
        // Pipe mode reads until end of input; without the drop it never
        // sees one and the wait below never returns.
        drop(stdin);
    }
    let output = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The dictionaries hunspell can actually load, for a settings screen
/// that would rather offer a list than a text field. Empty when
/// hunspell is missing or lists none.
///
/// `hunspell -D` prints its search path and then the dictionaries it
/// found, one absolute path per line, on standard error.
pub fn available_dictionaries() -> Vec<String> {
    let Ok(output) = Command::new("hunspell")
        .arg("-D")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
    else {
        return Vec::new();
    };
    let listing = String::from_utf8_lossy(&output.stderr);
    let mut names: Vec<String> = listing
        .lines()
        // The section this needs is the one after "AVAILABLE
        // DICTIONARIES"; every line in it is a path whose file name is
        // the dictionary. The lines before it are search directories,
        // which end in no file name worth offering.
        .skip_while(|line| !line.contains("AVAILABLE DICTIONARIES"))
        .skip(1)
        .take_while(|line| !line.trim().is_empty() && !line.contains("LOADED DICTIONARY"))
        .filter_map(|line| {
            std::path::Path::new(line.trim())
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names
}
