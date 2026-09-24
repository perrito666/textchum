//! Grammars loaded from a library at runtime.
//!
//! The built-in set is whatever the release was compiled with, so a
//! language missing from it had no way in short of a new build. A
//! grammar named in the configuration is opened here instead:
//!
//! ```json
//! {"languages": {"zig": {
//!   "grammar": "~/.local/share/textchum/grammars/libtree-sitter-zig.dylib",
//!   "highlights": "~/.local/share/textchum/grammars/zig/highlights.scm",
//!   "extensions": ["zig", "zon"],
//!   "aliases": ["ziglang"],
//!   "injections": "…/injections.scm",
//!   "symbol": "tree_sitter_zig"
//! }}}
//! ```
//!
//! `symbol` is optional: `tree_sitter_<name>` is the convention every
//! grammar follows, with dashes turned into underscores.
//!
//! The library is never unloaded. A syntax tree points into the
//! grammar's static tables, so closing the library under a live tree
//! ends the process — and a grammar is wanted for as long as a document
//! that uses it is open, which is until the editor quits.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tree_sitter::Language;

use crate::syntax::languages::{LanguageSource, LanguageSpec};

/// Loads every grammar named in the configuration's `languages`
/// section, and answers with what went wrong — one line per entry that
/// could not be loaded, for the shell to show.
///
/// An entry that fails is skipped rather than fatal: a stale path in a
/// configuration file should cost that one language, not the editor.
pub fn load_configured(config_json: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<Value>(config_json) else {
        return vec!["languages: the configuration is not JSON".to_string()];
    };
    let Some(entries) = root.get("languages").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut problems = Vec::new();
    for (name, entry) in entries {
        if let Err(problem) = load_entry(name, entry) {
            problems.push(format!("{name}: {problem}"));
        }
    }
    problems
}

fn load_entry(name: &str, entry: &Value) -> Result<(), String> {
    let library = entry
        .get("grammar")
        .and_then(Value::as_str)
        .ok_or("no grammar library named (\"grammar\")")?;
    let highlights_path = entry
        .get("highlights")
        .and_then(Value::as_str)
        .ok_or("no highlights query named (\"highlights\")")?;
    let symbol = entry
        .get("symbol")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("tree_sitter_{}", name.replace(['-', '.'], "_")));
    let extensions: Vec<String> = strings(entry.get("extensions"));
    let aliases: Vec<String> = strings(entry.get("aliases"));
    let filenames: Vec<String> = strings(entry.get("filenames"));

    let language = open(&expand(library), &symbol)?;
    let highlights = read(&expand(highlights_path))?;
    let injections = match entry.get("injections").and_then(Value::as_str) {
        Some(path) => Some(read(&expand(path))?),
        None => None,
    };
    // A query that does not compile against this grammar is the mistake
    // worth catching here: the alternative is a language that loads and
    // then paints nothing, with no way to tell why.
    tree_sitter::Query::new(&language, &highlights)
        .map_err(|error| format!("the highlights query does not compile: {error}"))?;
    if let Some(source) = &injections {
        tree_sitter::Query::new(&language, source)
            .map_err(|error| format!("the injections query does not compile: {error}"))?;
    }

    crate::syntax::languages::register_loaded(LanguageSpec {
        name: leak(name.to_string()),
        aliases: leak_all(aliases),
        extensions: leak_all(extensions),
        filenames: leak_all(filenames),
        language: LanguageSource::Loaded(language),
        highlights: leak(highlights),
        highlights_extra: None,
        injections: injections.map(leak).map(|source| source as &'static str),
    });
    Ok(())
}

/// Opens the library and asks it for its grammar.
fn open(path: &Path, symbol: &str) -> Result<Language, String> {
    if !path.exists() {
        return Err(format!("{} is not there", path.display()));
    }
    // Safety: loading a library runs its initializers, and the symbol is
    // called as a grammar constructor. Both are what the configuration
    // asked for; a wrong path is a wrong path either way.
    unsafe {
        let library = libloading::Library::new(path)
            .map_err(|error| format!("{} could not be opened: {error}", path.display()))?;
        let constructor: libloading::Symbol<unsafe extern "C" fn() -> Language> = library
            .get(symbol.as_bytes())
            .map_err(|error| format!("{symbol} is not in {}: {error}", path.display()))?;
        let language = constructor();
        // Deliberate: see the module note. The trees outlive any point
        // at which the library could be closed.
        std::mem::forget(library);
        let version = language.abi_version();
        if !(tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
            .contains(&version)
        {
            return Err(format!(
                "the grammar speaks ABI {version}; this build speaks {}–{}",
                tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
                tree_sitter::LANGUAGE_VERSION
            ));
        }
        Ok(language)
    }
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))
}

/// `~` is what a configuration file shared between machines can say.
fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// What building a grammar from its repository produced: the name it
/// goes by, the `languages` entry that names the library and queries
/// written under the grammars directory, and what was worth saying
/// about the queries on the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub name: String,
    pub entry: Value,
    pub warnings: Vec<String>,
}

impl Built {
    /// `{"name": …, "entry": {…}, "warnings": […]}`, for the shells.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "name": self.name,
            "entry": self.entry,
            "warnings": self.warnings,
        })
        .to_string()
    }
}

/// Builds the grammar checked out at `repository` into `grammars_dir`:
/// the parser compiled with the system C compiler into
/// `libtree-sitter-<name>.<dylib|so>`, the highlight and injection
/// queries copied beside it with their capture names mapped onto the
/// ones the editor's themes know, and the name and file types read
/// from the repository's `tree-sitter.json`. The library is opened and
/// the queries compiled against it before anything is answered, so
/// what comes back loads.
///
/// The `languages` entry is returned rather than written: the caller
/// owns the configuration and saves it, then loads the entry through
/// [`load_configured`]. Nothing is installed on an error, and the
/// error says what would make it work.
pub fn build(repository: &Path, grammars_dir: &Path) -> Result<Built, String> {
    let repository = repository
        .canonicalize()
        .map_err(|error| format!("{} could not be read: {error}", repository.display()))?;
    let (grammar, mut warnings) = describe(&repository)?;
    let sources = grammar.root.join("src");
    let parser = sources.join("parser.c");
    if !parser.exists() {
        return Err(format!(
            "{} has no src/parser.c: run `tree-sitter generate` in the repository first",
            grammar.root.display()
        ));
    }
    let queries_dir = [grammar.root.join("queries"), repository.join("queries")]
        .into_iter()
        .find(|dir| dir.join("highlights.scm").exists())
        .ok_or_else(|| {
            format!(
                "{} has no queries/highlights.scm, so nothing says what to colour; \
                 the editors that use this grammar keep their own query for it — \
                 copy one there and try again",
                repository.display()
            )
        })?;

    let out_dir = grammars_dir.join(&grammar.name);
    std::fs::create_dir_all(&out_dir)
        .map_err(|error| format!("{} could not be created: {error}", out_dir.display()))?;
    let library = grammars_dir.join(format!("libtree-sitter-{}.{}", grammar.name, LIBRARY_EXTENSION));
    compile(&sources, &library)?;

    let (highlights, mut noted) = translate_query(&read(&queries_dir.join("highlights.scm"))?);
    warnings.append(&mut noted);
    let highlights_path = out_dir.join("highlights.scm");
    write(&highlights_path, &highlights)?;
    let injections_source = queries_dir.join("injections.scm");
    let injections_path = if injections_source.exists() {
        let (injections, mut noted) = translate_query(&read(&injections_source)?);
        warnings.append(&mut noted);
        let path = out_dir.join("injections.scm");
        write(&path, &injections)?;
        Some(path)
    } else {
        None
    };

    let mut entry = serde_json::Map::new();
    entry.insert("grammar".into(), Value::String(portable(&library)));
    entry.insert("highlights".into(), Value::String(portable(&highlights_path)));
    if let Some(path) = &injections_path {
        entry.insert("injections".into(), Value::String(portable(path)));
    }
    entry.insert(
        "extensions".into(),
        Value::Array(grammar.extensions.iter().cloned().map(Value::String).collect()),
    );
    if grammar.symbol != format!("tree_sitter_{}", grammar.name.replace(['-', '.'], "_")) {
        entry.insert("symbol".into(), Value::String(grammar.symbol.clone()));
    }
    if grammar.extensions.is_empty() {
        warnings.push(format!(
            "the repository names no file types; add \"extensions\" to the {} entry \
             for files to open as it",
            grammar.name
        ));
    }
    let entry = Value::Object(entry);

    // What was written has to load, or it is not installed: the library
    // is opened and both queries compiled, exactly as at launch.
    let language = open(&library, &grammar.symbol)?;
    tree_sitter::Query::new(&language, &highlights)
        .map_err(|error| format!("the highlights query does not compile against the grammar: {error}"))?;
    if let Some(path) = &injections_path {
        tree_sitter::Query::new(&language, &read(path)?)
            .map_err(|error| format!("the injections query does not compile against the grammar: {error}"))?;
    }
    Ok(Built {
        name: grammar.name,
        entry,
        warnings,
    })
}

#[cfg(target_os = "macos")]
const LIBRARY_EXTENSION: &str = "dylib";
#[cfg(not(target_os = "macos"))]
const LIBRARY_EXTENSION: &str = "so";

/// A grammar as its repository describes it.
struct Described {
    name: String,
    /// The directory holding `src/` — the repository, or a grammar's
    /// own subdirectory in one that holds several.
    root: PathBuf,
    /// The constructor's name, from `src/grammar.json`.
    symbol: String,
    extensions: Vec<String>,
}

/// Reads `tree-sitter.json` for the name and file types, `src/grammar.json`
/// for the constructor's name, and falls back to the directory's name
/// with the `tree-sitter-` prefix dropped when neither says.
fn describe(repository: &Path) -> Result<(Described, Vec<String>), String> {
    let mut warnings = Vec::new();
    let manifest: Option<Value> = std::fs::read_to_string(repository.join("tree-sitter.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    let grammars: Vec<Value> = manifest
        .as_ref()
        .and_then(|manifest| manifest.get("grammars"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let first = grammars.first();
    let fallback_name = repository
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.strip_prefix("tree-sitter-").unwrap_or(name).to_string())
        .unwrap_or_else(|| "grammar".to_string());
    let name = first
        .and_then(|grammar| grammar.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or(fallback_name);
    let root = first
        .and_then(|grammar| grammar.get("path"))
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty() && *path != ".")
        .map(|path| repository.join(path))
        .unwrap_or_else(|| repository.to_path_buf());
    let extensions: Vec<String> = first
        .and_then(|grammar| grammar.get("file-types"))
        .map(|types| strings(Some(types)))
        .unwrap_or_default()
        .into_iter()
        .map(|extension| extension.trim_start_matches('.').to_string())
        .collect();
    if grammars.len() > 1 {
        let others: Vec<&str> = grammars[1..]
            .iter()
            .filter_map(|grammar| grammar.get("name").and_then(Value::as_str))
            .collect();
        warnings.push(format!(
            "the repository holds more than one grammar; {} was installed, and {} can be \
             installed from its own directory",
            name,
            others.join(", ")
        ));
    }
    let symbol = std::fs::read_to_string(root.join("src").join("grammar.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|grammar| grammar.get("name").and_then(Value::as_str).map(str::to_owned))
        .map(|grammar_name| format!("tree_sitter_{grammar_name}"))
        .unwrap_or_else(|| format!("tree_sitter_{}", name.replace(['-', '.'], "_")));
    Ok((
        Described {
            name,
            root,
            symbol,
            extensions,
        },
        warnings,
    ))
}

/// Compiles `parser.c` and the scanner, when there is one, into a
/// shared library at `library`, with the system's C (and C++, for a
/// C++ scanner) compiler. Nothing here imitates a tool: this is the
/// command the documentation asks a person to run.
fn compile(sources: &Path, library: &Path) -> Result<(), String> {
    let cc = compiler("CC", "cc")?;
    let mut objects = Vec::new();
    let mut linker = cc.clone();
    let out_dir = library.parent().unwrap_or(Path::new("."));
    let object_of = |name: &str| out_dir.join(format!(".{name}.o"));

    let parser_object = object_of("parser");
    run(
        std::process::Command::new(&cc)
            .args(["-O2", "-fPIC", "-std=c11", "-c"])
            .arg("-I")
            .arg(sources)
            .arg(sources.join("parser.c"))
            .arg("-o")
            .arg(&parser_object),
    )?;
    objects.push(parser_object);

    let scanner_c = sources.join("scanner.c");
    let scanner_cpp = ["scanner.cc", "scanner.cpp"]
        .iter()
        .map(|name| sources.join(name))
        .find(|path| path.exists());
    if scanner_c.exists() {
        let object = object_of("scanner");
        run(
            std::process::Command::new(&cc)
                .args(["-O2", "-fPIC", "-std=c11", "-c"])
                .arg("-I")
                .arg(sources)
                .arg(&scanner_c)
                .arg("-o")
                .arg(&object),
        )?;
        objects.push(object);
    } else if let Some(scanner) = scanner_cpp {
        let cxx = compiler("CXX", "c++")?;
        let object = object_of("scanner");
        run(
            std::process::Command::new(&cxx)
                .args(["-O2", "-fPIC", "-std=c++17", "-c"])
                .arg("-I")
                .arg(sources)
                .arg(&scanner)
                .arg("-o")
                .arg(&object),
        )?;
        objects.push(object);
        // A C++ scanner wants the C++ runtime linked in, which the C++
        // driver does and the C driver does not.
        linker = cxx;
    }

    run(
        std::process::Command::new(&linker)
            .arg("-shared")
            .args(&objects)
            .arg("-o")
            .arg(library),
    )?;
    for object in objects {
        let _ = std::fs::remove_file(object);
    }
    Ok(())
}

/// The compiler named by `variable`, or `default` on the path — and
/// when neither answers, what to install.
fn compiler(variable: &str, default: &str) -> Result<String, String> {
    let name = std::env::var(variable).ok().filter(|name| !name.is_empty()).unwrap_or_else(|| default.to_string());
    let answers = std::process::Command::new(&name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if answers {
        return Ok(name);
    }
    let fix = if cfg!(target_os = "macos") {
        "install the Command Line Tools with `xcode-select --install`"
    } else {
        "install a C compiler (gcc or clang) with your package manager"
    };
    Err(format!("no C compiler found as `{name}`: {fix}"))
}

fn run(command: &mut std::process::Command) -> Result<(), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .output()
        .map_err(|error| format!("{program} could not be run: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let said = said.lines().take(12).collect::<Vec<_>>().join("\n");
    Err(format!("{program} failed:\n{said}"))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text)
        .map_err(|error| format!("{} could not be written: {error}", path.display()))
}

/// The path as the configuration file should carry it: under `~` when
/// it is in the home directory, so the file reads the same on another
/// machine.
fn portable(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(rest) = path.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

/// A query as another editor wrote it, rewritten for this one: capture
/// names the themes here have no colour for are mapped onto the ones
/// they do, captures that mean nothing here are dropped, and a Lua
/// pattern predicate becomes the regular expression tree-sitter
/// evaluates. What could not be rewritten is named in the warnings,
/// since a predicate the runtime does not know is not an error to it —
/// the pattern simply matches without it, which is the silent failure
/// worth a word.
pub fn translate_query(source: &str) -> (String, Vec<String>) {
    let mut warnings = Vec::new();
    let mut out = String::with_capacity(source.len());
    let mut unknown_predicates: Vec<String> = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let number = index + 1;
        if let Some(rest) = line.trim().strip_prefix(";") {
            if let Some(inherited) = rest.trim().strip_prefix("inherits:") {
                warnings.push(format!(
                    "line {number}: the query inherits from {}, which is not available here; \
                     only its own patterns are used",
                    inherited.trim()
                ));
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let line = rewrite_captures(line);
        let line = match rewrite_lua_match(&line) {
            Ok(line) => line,
            Err(pattern) => {
                warnings.push(format!(
                    "line {number}: the Lua pattern {pattern:?} could not be turned into a \
                     regular expression; that pattern matches without it"
                ));
                line
            }
        };
        for predicate in predicates_in(&line) {
            if !KNOWN_PREDICATES.contains(&predicate.as_str())
                && !unknown_predicates.contains(&predicate)
            {
                warnings.push(format!(
                    "line {number}: `{predicate}` is not evaluated here; the patterns using it \
                     match without it"
                ));
                unknown_predicates.push(predicate);
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    (out, warnings)
}

/// The predicates tree-sitter's runtime evaluates, or accepts.
const KNOWN_PREDICATES: &[&str] = &[
    "#eq?", "#not-eq?", "#any-eq?", "#any-not-eq?", "#match?", "#not-match?", "#any-match?",
    "#any-not-match?", "#any-of?", "#not-any-of?", "#is?", "#is-not?", "#set!",
];

/// Capture names other editors' queries use, and the editor's own for
/// each. Longest name first, so `keyword.control.repeat` is matched
/// before `keyword.control`. A name not here is kept: the themes
/// resolve a dotted name by trimming it — `function.method` is a
/// `function` — so most names need no help.
const CAPTURE_RENAMES: &[(&str, &str)] = &[
    ("keyword.control.conditional", "conditional"),
    ("keyword.control.repeat", "repeat"),
    ("keyword.control.exception", "exception"),
    ("keyword.control.import", "include"),
    ("keyword.conditional", "conditional"),
    ("keyword.repeat", "repeat"),
    ("keyword.exception", "exception"),
    ("keyword.import", "include"),
    ("keyword.storage", "storageclass"),
    ("constant.numeric.float", "float"),
    ("constant.numeric", "number"),
    ("constant.character.escape", "escape"),
    ("constant.character", "character"),
    ("constant.builtin.boolean", "boolean"),
    ("string.escape", "escape"),
    ("string.regexp", "string.special"),
    ("string.special.url", "text.uri"),
    ("string.special.path", "text.uri"),
    ("markup.strong", "text.strong"),
    ("markup.bold", "text.strong"),
    ("markup.italic", "text.emphasis"),
    ("markup.raw", "text.literal"),
    ("markup.link.url", "text.uri"),
    ("markup.link.label", "markup.link"),
    ("markup.link.text", "markup.link"),
    ("markup.list", "punctuation.special"),
    ("markup.quote", "comment"),
    ("variable.other.member", "variable.member"),
    ("tag.attribute", "attribute"),
    ("punctuation.delimiter", "delimiter"),
    ("special", "punctuation.special"),
];

/// Captures that carry no colour anywhere: spell-checking hints and
/// the explicit "nothing" other editors use to undo a parent's colour.
const DROPPED_CAPTURES: &[&str] = &["spell", "nospell", "none", "conceal"];

fn rewrite_captures(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find('@') {
        let (before, from_at) = rest.split_at(at);
        out.push_str(before);
        let name_end = from_at[1..]
            .find(|c: char| !(c.is_alphanumeric() || c == '.' || c == '_' || c == '-'))
            .map_or(from_at.len(), |end| end + 1);
        let name = &from_at[1..name_end];
        rest = &from_at[name_end..];
        if name.is_empty() {
            out.push('@');
            continue;
        }
        if DROPPED_CAPTURES.contains(&name) {
            // The capture goes, and the blank before it with it.
            while out.ends_with(' ') {
                out.pop();
            }
            continue;
        }
        match CAPTURE_RENAMES.iter().find(|(theirs, _)| {
            name == *theirs || name.starts_with(&format!("{theirs}."))
        }) {
            Some((_, ours)) => {
                out.push('@');
                out.push_str(ours);
            }
            None => {
                out.push('@');
                out.push_str(name);
            }
        }
    }
    out.push_str(rest);
    out
}

/// `(#lua-match? @c "^%d+$")` as `(#match? @c "^[0-9]+$")`. Lua's
/// pattern language is small enough to carry over item by item; what
/// it cannot express in a regular expression — `%b`, `%f` — is
/// answered with the pattern as the error.
fn rewrite_lua_match(line: &str) -> Result<String, String> {
    let Some(at) = line.find("#lua-match?") else {
        return Ok(line.to_string());
    };
    let (before, from) = line.split_at(at);
    let after_name = &from["#lua-match?".len()..];
    let Some(open) = after_name.find('"') else {
        return Ok(line.to_string());
    };
    let Some(close) = after_name[open + 1..].find('"') else {
        return Ok(line.to_string());
    };
    let pattern = &after_name[open + 1..open + 1 + close];
    let regex = lua_pattern_to_regex(pattern).ok_or_else(|| pattern.to_string())?;
    Ok(format!(
        "{before}#match?{}\"{regex}\"{}",
        &after_name[..open],
        &after_name[open + 1 + close + 1..]
    ))
}

fn lua_pattern_to_regex(pattern: &str) -> Option<String> {
    let mut out = String::new();
    let mut characters = pattern.chars().peekable();
    let mut in_class = false;
    while let Some(c) = characters.next() {
        match c {
            '%' => {
                let next = characters.next()?;
                // A class stands alone as a bracket expression and
                // inside one as its contents.
                let set = |members: &str| -> String {
                    if in_class { members.to_string() } else { format!("[{members}]") }
                };
                let piece = match next {
                    'd' => set("0-9"),
                    'a' => set("A-Za-z"),
                    'w' => set("A-Za-z0-9"),
                    'l' => set("a-z"),
                    'u' => set("A-Z"),
                    'x' => set("0-9A-Fa-f"),
                    'p' => set("[:punct:]"),
                    's' => r"\s".to_string(),
                    // Balanced matches and frontiers have no regular
                    // expression.
                    'b' | 'f' => return None,
                    other if other.is_ascii_punctuation() => format!("\\{other}"),
                    other => other.to_string(),
                };
                out.push_str(&piece);
            }
            '[' if !in_class => {
                in_class = true;
                out.push('[');
            }
            ']' if in_class => {
                in_class = false;
                out.push(']');
            }
            '-' if !in_class => out.push_str("*?"),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    Some(out)
}

fn predicates_in(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("(#") {
        let from = &rest[at + 1..];
        let end = from
            .find(|c: char| c.is_whitespace() || c == ')')
            .unwrap_or(from.len());
        found.push(from[..end].to_string());
        rest = &from[end..];
    }
    found
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn leak_all(items: Vec<String>) -> &'static [&'static str] {
    let leaked: Vec<&'static str> = items.into_iter().map(leak).collect();
    Box::leak(leaked.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configuration_without_languages_asks_for_nothing() {
        assert!(load_configured(r#"{"editor":{}}"#).is_empty());
    }

    #[test]
    fn an_entry_says_what_it_is_missing() {
        let problems = load_configured(r#"{"languages":{"zig":{}}}"#);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("grammar"), "{problems:?}");
    }

    #[test]
    fn a_library_that_is_not_there_is_one_language_lost_and_not_a_crash() {
        let problems = load_configured(
            r#"{"languages":{"zig":{"grammar":"/nowhere/libzig.dylib",
                 "highlights":"/nowhere/highlights.scm"}}}"#,
        );
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("is not there"), "{problems:?}");
    }

    #[test]
    fn another_editors_query_is_rewritten_for_this_one() {
        let source = "; inherits: hcl\n\
                      (comment) @comment @spell\n\
                      \"for\" @keyword.repeat\n\
                      (a) @markup.bold\n\
                      (b) @function.method\n\
                      ((c) @constant (#lua-match? @constant \"^[A-Z][A-Z_%d]*$\"))\n\
                      ((d) @type (#has-ancestor? @type block))\n\
                      ((e) @_x (#eq? @_x \"y\")) @none\n";
        let (rewritten, warnings) = translate_query(source);
        assert!(rewritten.contains("(comment) @comment\n"), "{rewritten}");
        assert!(rewritten.contains("\"for\" @repeat\n"), "{rewritten}");
        assert!(rewritten.contains("(a) @text.strong\n"), "{rewritten}");
        // Left alone: the themes trim a dotted name to one they know.
        assert!(rewritten.contains("(b) @function.method\n"), "{rewritten}");
        assert!(
            rewritten.contains("(#match? @constant \"^[A-Z][A-Z_0-9]*$\")"),
            "{rewritten}"
        );
        assert!(rewritten.contains("((e) @_x (#eq? @_x \"y\"))\n"), "{rewritten}");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("inherits from hcl"));
        assert!(warnings[1].contains("#has-ancestor?"));
    }

    #[test]
    fn lua_patterns_become_regular_expressions() {
        assert_eq!(lua_pattern_to_regex("^%a[%w_]*$").as_deref(), Some("^[A-Za-z][A-Za-z0-9_]*$"));
        assert_eq!(lua_pattern_to_regex("%.%d-x").as_deref(), Some(r"\.[0-9]*?x"));
        assert_eq!(lua_pattern_to_regex("%bxy"), None);
    }

    #[test]
    fn a_grammar_is_built_from_its_repository_and_loads() {
        // A repository stood up from the Dockerfile grammar the build
        // carries as C: what a checkout of any grammar looks like, with
        // the queries as another editor would write them.
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let scratch = std::env::temp_dir().join(format!("textchum-grammar-install-{}", std::process::id()));
        let repository = scratch.join("tree-sitter-dockertest");
        let sources = repository.join("src");
        std::fs::create_dir_all(sources.join("tree_sitter")).unwrap();
        for name in ["parser.c", "scanner.c"] {
            std::fs::copy(
                manifest.join("grammars/dockerfile/src").join(name),
                sources.join(name),
            )
            .unwrap();
        }
        for name in ["parser.h", "alloc.h", "array.h"] {
            std::fs::copy(
                manifest.join("grammars/dockerfile/src/tree_sitter").join(name),
                sources.join("tree_sitter").join(name),
            )
            .unwrap();
        }
        std::fs::write(sources.join("grammar.json"), r#"{"name": "dockerfile"}"#).unwrap();
        std::fs::write(
            repository.join("tree-sitter.json"),
            r#"{"grammars": [{"name": "dockertest", "file-types": ["dtest"]}]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(repository.join("queries")).unwrap();
        std::fs::write(
            repository.join("queries/highlights.scm"),
            "(comment) @comment @spell\n\"FROM\" @keyword.import\n\
             ((variable) @constant (#lua-match? @constant \"^[A-Z][A-Z_%d]*$\"))\n",
        )
        .unwrap();
        let grammars = scratch.join("grammars");

        let built = build(&repository, &grammars).unwrap();
        assert_eq!(built.name, "dockertest");
        assert!(built.warnings.is_empty(), "{:?}", built.warnings);
        let entry = &built.entry;
        assert_eq!(entry["symbol"], "tree_sitter_dockerfile", "the constructor is the grammar's");
        assert_eq!(entry["extensions"], serde_json::json!(["dtest"]));
        assert!(grammars.join(format!("libtree-sitter-dockertest.{LIBRARY_EXTENSION}")).exists());
        let highlights = std::fs::read_to_string(grammars.join("dockertest/highlights.scm")).unwrap();
        assert!(highlights.contains("\"FROM\" @include"), "{highlights}");
        assert!(!highlights.contains("@spell"), "{highlights}");

        // The entry loads the way the configuration's would, and a file
        // of that type opens as it.
        let config = serde_json::json!({"languages": {built.name.clone(): entry}}).to_string();
        assert!(load_configured(&config).is_empty());
        assert_eq!(
            crate::syntax::languages::by_path(Path::new("x.dtest")).map(|l| l.spec.name),
            Some("dockertest")
        );
        let mut document = crate::Document::new();
        document.replace_utf16(0, 0, "FROM alpine\n").unwrap();
        assert!(document.set_language(Some("dockertest")));
        assert!(!document.highlights(0, document.len_utf16()).unwrap().is_empty());

        // Without a parser, the message says what to run.
        std::fs::remove_file(sources.join("parser.c")).unwrap();
        let error = build(&repository, &grammars).unwrap_err();
        assert!(error.contains("tree-sitter generate"), "{error}");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_symbol_follows_the_name_when_it_is_not_given() {
        // The convention every grammar follows, with the punctuation a
        // language name can carry turned into underscores.
        assert_eq!(
            "tree_sitter_my_lang",
            format!("tree_sitter_{}", "my-lang".replace(['-', '.'], "_"))
        );
    }

    #[test]
    fn a_tilde_means_home() {
        std::env::set_var("HOME", "/home/someone");
        assert_eq!(expand("~/g/x.so"), PathBuf::from("/home/someone/g/x.so"));
        assert_eq!(expand("/abs/x.so"), PathBuf::from("/abs/x.so"));
    }
}
