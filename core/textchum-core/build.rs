//! Compiles the translation catalogues.
//!
//! The source of truth is `i18n/<language>.po` — the format translators
//! and their tools speak. Each is compiled to a binary `.mo` catalogue,
//! which is what the editor reads at runtime, and which any gettext
//! tool can also read.
//!
//! `msgfmt` does the compiling when it is installed. It usually is
//! (GNU gettext ships with most distributions and comes with Homebrew),
//! and when it is not, a build should not fail over a translation, so
//! the fallback is a compiler written here against the documented `.mo`
//! format.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let catalogues = manifest.join("i18n");
    println!("cargo:rerun-if-changed=i18n");
    let Ok(entries) = std::fs::read_dir(&catalogues) else { return };
    for entry in entries.flatten() {
        let source = entry.path();
        if source.extension().and_then(|e| e.to_str()) != Some("po") {
            continue;
        }
        let Some(stem) = source.file_stem().and_then(|s| s.to_str()) else { continue };
        println!("cargo:rerun-if-changed=i18n/{stem}.po");
        let target = out.join(format!("{stem}.mo"));
        if compile_with_msgfmt(&source, &target) {
            continue;
        }
        let entries = parse_po(&std::fs::read_to_string(&source).unwrap_or_default());
        std::fs::write(&target, write_mo(&entries)).expect("write catalogue");
    }
}

fn compile_with_msgfmt(source: &Path, target: &Path) -> bool {
    Command::new("msgfmt")
        .arg("--output-file")
        .arg(target)
        .arg(source)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The entries of a `.po` file, as `.mo` wants them: the key is the
/// msgid, with `msgctxt` and plural forms joined by the separators the
/// format specifies (EOT for context, NUL between plural forms).
fn parse_po(text: &str) -> BTreeMap<String, String> {
    let mut entries: BTreeMap<String, String> = BTreeMap::new();
    let mut context: Option<String> = None;
    let mut id = String::new();
    let mut plural_id: Option<String> = None;
    let mut translations: Vec<String> = Vec::new();
    let mut current: Option<usize> = None;
    let mut in_id = false;
    let mut in_plural_id = false;

    let flush = |entries: &mut BTreeMap<String, String>,
                 context: &Option<String>,
                 id: &str,
                 plural_id: &Option<String>,
                 translations: &[String]| {
        if translations.iter().all(|text| text.is_empty()) && !id.is_empty() {
            return;
        }
        let mut key = String::new();
        if let Some(context) = context {
            key.push_str(context);
            key.push('\u{4}');
        }
        key.push_str(id);
        if let Some(plural) = plural_id {
            key.push('\0');
            key.push_str(plural);
        }
        entries.insert(key, translations.join("\0"));
    };

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if line.is_empty() {
            flush(&mut entries, &context, &id, &plural_id, &translations);
            context = None;
            id.clear();
            plural_id = None;
            translations.clear();
            current = None;
            in_id = false;
            in_plural_id = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("msgctxt ") {
            context = Some(unquote(rest));
            in_id = false;
            in_plural_id = false;
            current = None;
        } else if let Some(rest) = line.strip_prefix("msgid_plural ") {
            plural_id = Some(unquote(rest));
            in_id = false;
            in_plural_id = true;
            current = None;
        } else if let Some(rest) = line.strip_prefix("msgid ") {
            id = unquote(rest);
            in_id = true;
            in_plural_id = false;
            current = None;
        } else if let Some(rest) = line.strip_prefix("msgstr[") {
            let (index, value) = rest.split_once(']').unwrap_or(("0", ""));
            let at: usize = index.parse().unwrap_or(0);
            while translations.len() <= at {
                translations.push(String::new());
            }
            translations[at] = unquote(value.trim());
            current = Some(at);
            in_id = false;
            in_plural_id = false;
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            translations.clear();
            translations.push(unquote(rest));
            current = Some(0);
            in_id = false;
            in_plural_id = false;
        } else if line.starts_with('"') {
            // A continuation line belongs to whatever came before it.
            let text = unquote(line);
            if let Some(at) = current {
                translations[at].push_str(&text);
            } else if in_plural_id {
                if let Some(plural) = plural_id.as_mut() {
                    plural.push_str(&text);
                }
            } else if in_id {
                id.push_str(&text);
            } else if let Some(context) = context.as_mut() {
                context.push_str(&text);
            }
        }
    }
    flush(&mut entries, &context, &id, &plural_id, &translations);
    entries
}

/// Unquotes one `"…"` string, undoing the escapes the format allows.
fn unquote(text: &str) -> String {
    let trimmed = text.trim();
    let inner = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(trimmed);
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for c in inner.chars() {
        if escaped {
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other,
            });
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// The `.mo` format: a magic number, two string tables, and no hash
/// table (which the format allows and readers cope with).
fn write_mo(entries: &BTreeMap<String, String>) -> Vec<u8> {
    let count = entries.len() as u32;
    let ids: Vec<&[u8]> = entries.keys().map(|key| key.as_bytes()).collect();
    let texts: Vec<&[u8]> = entries.values().map(|value| value.as_bytes()).collect();
    let header = 28u32;
    let id_table = header;
    let text_table = id_table + count * 8;
    let mut strings_at = text_table + count * 8;
    let mut out: Vec<u8> = Vec::new();
    out.extend(0x950412deu32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(id_table.to_le_bytes());
    out.extend(text_table.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    let mut offsets = Vec::new();
    for id in &ids {
        offsets.push((id.len() as u32, strings_at));
        strings_at += id.len() as u32 + 1;
    }
    for text in &texts {
        offsets.push((text.len() as u32, strings_at));
        strings_at += text.len() as u32 + 1;
    }
    for (length, at) in &offsets {
        out.extend(length.to_le_bytes());
        out.extend(at.to_le_bytes());
    }
    for id in &ids {
        out.extend(*id);
        out.push(0);
    }
    for text in &texts {
        out.extend(*text);
        out.push(0);
    }
    out
}
