//! Prose spell check, scoped the way the macOS shell scopes it: only
//! comments in code, and whole documents for Markdown, git commit
//! messages, and plain text — identifiers are never flagged.
//!
//! The checking itself rides `hunspell -l` (the dictionaries most
//! desktops already have): each pass feeds the prose to hunspell, gets
//! the misspelled words back, and tags every occurrence. `editor.spell`
//! picks the dictionary — `"auto"` follows `$LANG`, a code like
//! `"es_ES"` names one, absent means off.

use std::cell::Cell;
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
}

/// Installs the misspelling tag on a fresh buffer: a purple tint, so
/// spelling never reads as a diagnostic.
pub fn install_tag(buffer: &sourceview5::Buffer) {
    let tag = gtk::TextTag::new(Some(TAG));
    tag.set_background_rgba(Some(&gtk::gdk::RGBA::new(0.55, 0.36, 0.76, 0.25)));
    buffer.tag_table().add(&tag);
}

/// Runs one spell pass over the page (or clears the marks when the
/// configuration says off). Blocking on a hunspell run — call it from
/// the settle timer, not per keystroke.
pub fn run(page: &Rc<Page>) {
    let buffer = &page.buffer;
    let clear = |buffer: &sourceview5::Buffer| {
        buffer.remove_tag_by_name(TAG, &buffer.start_iter(), &buffer.end_iter());
    };
    let Some(language) = Shell::instance().config.borrow().spell_language() else {
        clear(buffer);
        return;
    };

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
    let Some(misspelled) = misspelled_words(&prose, &language) else {
        return;
    };
    if misspelled.is_empty() {
        return;
    }

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
            if word.chars().count() > 1 && misspelled.contains(word) {
                let from = buffer.iter_at_offset(word_start as i32);
                let to = buffer.iter_at_offset(index as i32);
                buffer.apply_tag_by_name(TAG, &from, &to);
            }
        }
    }
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
    let total_utf16 = text.encode_utf16().count();
    let Ok(spans) = state.document.highlights(0, total_utf16) else {
        return Vec::new();
    };
    spans
        .iter()
        // Style index 1 is the canonical comment capture.
        .filter(|span| span.style == 1)
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
fn misspelled_words(prose: &str, language: &str) -> Option<HashSet<String>> {
    let mut command = Command::new("hunspell");
    command.arg("-l");
    if language != "auto" {
        command.args(["-d", language]);
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
                        workbench.toast(
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
        let _ = stdin.write_all(prose.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect(),
    )
}
