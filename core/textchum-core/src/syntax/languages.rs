//! The language registry: which languages exist, how files map to them,
//! and their compiled grammars and queries.
//!
//! Grammars come from crates.io packages (cargo compiles their C sources),
//! which covers the mainstream languages with zero build machinery. A
//! vendoring pipeline can join later for the long tail without changing
//! this interface.
//!
//! Queries are compiled lazily, once per language, on first use.

use std::sync::OnceLock;

use tree_sitter::{Language, Query};

use crate::syntax::theme;

/// Static description of a supported language.
pub struct LanguageSpec {
    /// Canonical name, also used for injection matching and UI display.
    pub name: &'static str,
    /// Alternative names accepted for injections (fence info strings, etc.).
    pub aliases: &'static [&'static str],
    /// File extensions (lowercase, no dot) that select this language.
    pub extensions: &'static [&'static str],
    /// Exact file names that select this language — for the files whose
    /// identity is their name, not an extension (Makefile, git's
    /// COMMIT_EDITMSG).
    pub filenames: &'static [&'static str],
    language: fn() -> Language,
    highlights: &'static str,
    injections: Option<&'static str>,
}

/// A language with its grammar loaded and queries compiled.
pub struct CompiledLanguage {
    pub spec: &'static LanguageSpec,
    pub language: Language,
    pub highlights: Query,
    /// Style id for each capture index of `highlights` (None = unstyled).
    pub capture_styles: Vec<Option<u32>>,
    pub injections: Option<Query>,
}

/// Registry entry: spec plus its lazily compiled artifacts.
pub struct RegisteredLanguage {
    pub spec: &'static LanguageSpec,
    compiled: OnceLock<CompiledLanguage>,
}

impl RegisteredLanguage {
    /// Compiles the grammar's queries on first call; cheap afterwards.
    pub fn compiled(&'static self) -> &'static CompiledLanguage {
        self.compiled.get_or_init(|| {
            let language = (self.spec.language)();
            let highlights = Query::new(&language, self.spec.highlights)
                .unwrap_or_else(|e| panic!("bad highlights query for {}: {e}", self.spec.name));
            let capture_styles = highlights
                .capture_names()
                .iter()
                .map(|name| theme::resolve(name))
                .collect();
            let injections = self.spec.injections.map(|source| {
                Query::new(&language, source)
                    .unwrap_or_else(|e| panic!("bad injections query for {}: {e}", self.spec.name))
            });
            CompiledLanguage {
                spec: self.spec,
                language,
                highlights,
                capture_styles,
                injections,
            }
        })
    }
}

macro_rules! lang {
    ($name:literal, $aliases:expr, $exts:expr, $lang:expr, $hl:expr, $inj:expr) => {
        lang!($name, $aliases, $exts, &[], $lang, $hl, $inj)
    };
    ($name:literal, $aliases:expr, $exts:expr, $files:expr, $lang:expr, $hl:expr, $inj:expr) => {
        LanguageSpec {
            name: $name,
            aliases: $aliases,
            extensions: $exts,
            filenames: $files,
            language: || $lang.into(),
            highlights: $hl,
            injections: $inj,
        }
    };
}

static SPECS: &[LanguageSpec] = &[
    lang!(
        "rust",
        &["rs"],
        &["rs"],
        tree_sitter_rust::LANGUAGE,
        tree_sitter_rust::HIGHLIGHTS_QUERY,
        Some(tree_sitter_rust::INJECTIONS_QUERY)
    ),
    lang!(
        "python",
        &["py", "python3"],
        &["py", "pyi"],
        tree_sitter_python::LANGUAGE,
        tree_sitter_python::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "go",
        &["golang"],
        &["go"],
        tree_sitter_go::LANGUAGE,
        tree_sitter_go::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "c",
        &[],
        &["c", "h"],
        tree_sitter_c::LANGUAGE,
        tree_sitter_c::HIGHLIGHT_QUERY,
        None
    ),
    lang!(
        "javascript",
        &["js", "jsx", "node"],
        &["js", "jsx", "mjs", "cjs"],
        tree_sitter_javascript::LANGUAGE,
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        Some(tree_sitter_javascript::INJECTIONS_QUERY)
    ),
    lang!(
        "json",
        &[],
        &["json", "jsonc"],
        tree_sitter_json::LANGUAGE,
        tree_sitter_json::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "make",
        &["makefile", "gnumakefile"],
        &["mk", "mak"],
        &["Makefile", "makefile", "GNUmakefile"],
        tree_sitter_make::LANGUAGE,
        tree_sitter_make::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "gitcommit",
        &["git-commit"],
        &[],
        &["COMMIT_EDITMSG", "MERGE_MSG", "TAG_EDITMSG"],
        tree_sitter_gitcommit::LANGUAGE,
        tree_sitter_gitcommit::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "bash",
        &["sh", "shell", "zsh"],
        &["sh", "bash", "zsh"],
        tree_sitter_bash::LANGUAGE,
        tree_sitter_bash::HIGHLIGHT_QUERY,
        None
    ),
    lang!(
        "html",
        &["htm"],
        &["html", "htm"],
        tree_sitter_html::LANGUAGE,
        tree_sitter_html::HIGHLIGHTS_QUERY,
        Some(tree_sitter_html::INJECTIONS_QUERY)
    ),
    lang!(
        "css",
        &[],
        &["css"],
        tree_sitter_css::LANGUAGE,
        tree_sitter_css::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "toml",
        &[],
        &["toml"],
        tree_sitter_toml_ng::LANGUAGE,
        tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "yaml",
        &["yml"],
        &["yaml", "yml"],
        tree_sitter_yaml::LANGUAGE,
        tree_sitter_yaml::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "swift",
        &[],
        &["swift"],
        tree_sitter_swift::LANGUAGE,
        tree_sitter_swift::HIGHLIGHTS_QUERY,
        Some(tree_sitter_swift::INJECTIONS_QUERY)
    ),
    lang!(
        "zig",
        &[],
        &["zig"],
        tree_sitter_zig::LANGUAGE,
        tree_sitter_zig::HIGHLIGHTS_QUERY,
        Some(tree_sitter_zig::INJECTIONS_QUERY)
    ),
    lang!(
        "markdown",
        &["md"],
        &["md", "markdown"],
        tree_sitter_md::LANGUAGE,
        tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
        Some(tree_sitter_md::INJECTION_QUERY_BLOCK)
    ),
    // The markdown inline grammar is reached only through injection from
    // the block grammar (never by file extension), and is where emphasis,
    // links, and inline code live.
    lang!(
        "markdown-inline",
        &["markdown_inline"],
        &[],
        tree_sitter_md::INLINE_LANGUAGE,
        tree_sitter_md::HIGHLIGHT_QUERY_INLINE,
        None
    ),
];

fn registry() -> &'static [RegisteredLanguage] {
    static REGISTRY: OnceLock<Vec<RegisteredLanguage>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        SPECS
            .iter()
            .map(|spec| RegisteredLanguage {
                spec,
                compiled: OnceLock::new(),
            })
            .collect()
    })
}

/// Finds a language by canonical name or alias (case-insensitive).
pub fn by_name(name: &str) -> Option<&'static RegisteredLanguage> {
    let needle = name.to_ascii_lowercase();
    registry().iter().find(|entry| {
        entry.spec.name == needle || entry.spec.aliases.iter().any(|alias| *alias == needle)
    })
}

/// Finds a language by a file path: an exact file-name match first
/// (Makefile, COMMIT_EDITMSG), then the extension.
pub fn by_path(path: &std::path::Path) -> Option<&'static RegisteredLanguage> {
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        if let Some(entry) = registry()
            .iter()
            .find(|entry| entry.spec.filenames.iter().any(|file| *file == name))
        {
            return Some(entry);
        }
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    registry()
        .iter()
        .find(|entry| entry.spec.extensions.iter().any(|ext| *ext == extension))
}

/// Every registered language name that files can map to (for UI pickers).
pub fn selectable_names() -> Vec<&'static str> {
    registry()
        .iter()
        .filter(|entry| !entry.spec.extensions.is_empty())
        .map(|entry| entry.spec.name)
        .collect()
}
