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
    pub(crate) language: LanguageSource,
    pub(crate) highlights: &'static str,
    /// Patterns appended to `highlights` when the grammar's own query
    /// is wrong for us. A grammar ships one query, written against
    /// whichever highlighter its author uses, and nvim-treesitter
    /// evaluates `#match?` with Lua patterns where tree-sitter proper
    /// uses a regular expression — so a predicate can be silently false
    /// here and true there. Appended rather than replacing, because a
    /// later pattern wins and the rest of a 400-line query is fine.
    pub(crate) highlights_extra: Option<&'static str>,
    pub(crate) injections: Option<&'static str>,
}

/// Where a grammar comes from: compiled into the build, or loaded from
/// a library named in the configuration.
pub enum LanguageSource {
    Built(fn() -> Language),
    /// Kept by value: the library it came from is leaked deliberately,
    /// since a grammar that is unloaded while a tree still points into
    /// it takes the process with it.
    Loaded(Language),
}

impl LanguageSpec {
    fn language(&self) -> Language {
        match &self.language {
            LanguageSource::Built(make) => make(),
            LanguageSource::Loaded(language) => language.clone(),
        }
    }
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
            let language = self.spec.language();
            let source = match self.spec.highlights_extra {
                Some(extra) => {
                    std::borrow::Cow::Owned(format!("{}\n{extra}", self.spec.highlights))
                }
                None => std::borrow::Cow::Borrowed(self.spec.highlights),
            };
            let highlights = Query::new(&language, &source)
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

/// The last argument of the shorter forms is the **injections** query.
/// An extra-highlights query handed to one of them compiles as an
/// injections query, does nothing, and reports nothing — use the long
/// form, which names both.
macro_rules! lang {
    ($name:literal, $aliases:expr, $exts:expr, $lang:expr, $hl:expr, $inj:expr) => {
        lang!($name, $aliases, $exts, &[], $lang, $hl, $inj)
    };
    ($name:literal, $aliases:expr, $exts:expr, $files:expr, $lang:expr, $hl:expr, $inj:expr) => {
        lang!($name, $aliases, $exts, $files, $lang, $hl, None, $inj)
    };
    (
        $name:literal, $aliases:expr, $exts:expr, $files:expr,
        $lang:expr, $hl:expr, $extra:expr, $inj:expr
    ) => {
        LanguageSpec {
            name: $name,
            aliases: $aliases,
            extensions: $exts,
            filenames: $files,
            language: LanguageSource::Built(|| $lang.into()),
            highlights: $hl,
            highlights_extra: $extra,
            injections: $inj,
        }
    };
}

/// The SQL grammar captures every literal as `@string` first, then
/// narrows numbers and decimals back out with two `#match?` predicates
/// written in Lua pattern syntax (`%d`). Read as the regular expression
/// tree-sitter actually applies, `%d` is a literal per cent followed by
/// a d, so neither predicate ever holds and `42` is painted like
/// `'harbor'`. The same rules, in regex.
// The dot is a character class, not an escape: tree-sitter unescapes
// `\.` while parsing the query string, leaving the regex with a bare
// dot that matches any character — so `42` came back as a float.
const SQL_NUMBER_LITERALS: &str = r#"
((literal) @number (#match? @number "^[-+]?[0-9]+$"))
((literal) @float (#match? @float "^[-+]?[0-9]*[.][0-9]+$"))
"#;

/// `self` and `cls` are not ordinary identifiers: one is the receiver
/// and the other the class, and telling them apart from the locals
/// around them is most of what colour is for in a method body. The
/// grammar's own query says nothing about either, so this does.
///
/// Appended after the grammar's patterns, which is what lets it win: a
/// later match replaces an earlier one over the same range.
const PYTHON_RECEIVERS: &str = r#"
((identifier) @variable.builtin (#match? @variable.builtin "^(self|cls)$"))
"#;

static SPECS: &[LanguageSpec] = &[
    // TypeScript ships only what it adds to JavaScript — types,
    // interfaces, enums — and inherits the rest, so JavaScript's query
    // goes first and the additions after it.
    lang!(
        "typescript",
        &["ts"],
        &["ts", "mts", "cts"],
        &[],
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        Some(tree_sitter_typescript::HIGHLIGHTS_QUERY),
        None
    ),
    lang!(
        "tsx",
        &[],
        &["tsx"],
        &[],
        tree_sitter_typescript::LANGUAGE_TSX,
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        Some(tree_sitter_typescript::HIGHLIGHTS_QUERY),
        None
    ),
    // C++ ships only what it adds to C — templates, namespaces,
    // `auto` — and inherits the rest, so C's query goes first and the
    // additions after it, which is also the order that lets a later
    // match win.
    lang!(
        "cpp",
        &["c++"],
        &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
        &[],
        tree_sitter_cpp::LANGUAGE,
        tree_sitter_c::HIGHLIGHT_QUERY,
        Some(tree_sitter_cpp::HIGHLIGHT_QUERY),
        None
    ),
    lang!(
        "java",
        &[],
        &["java"],
        tree_sitter_java::LANGUAGE,
        tree_sitter_java::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "ruby",
        &["rb"],
        &["rb", "rake", "gemspec"],
        &["Gemfile", "Rakefile"],
        tree_sitter_ruby::LANGUAGE,
        tree_sitter_ruby::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "php",
        &[],
        &["php", "phtml"],
        tree_sitter_php::LANGUAGE_PHP,
        tree_sitter_php::HIGHLIGHTS_QUERY,
        Some(tree_sitter_php::INJECTIONS_QUERY)
    ),
    lang!(
        "csharp",
        &["c#", "cs"],
        &["cs"],
        tree_sitter_c_sharp::LANGUAGE,
        tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "lua",
        &[],
        &["lua"],
        tree_sitter_lua::LANGUAGE,
        tree_sitter_lua::HIGHLIGHTS_QUERY,
        Some(tree_sitter_lua::INJECTIONS_QUERY)
    ),
    lang!(
        "xml",
        &[],
        &["xml", "xsd", "xsl", "xslt", "svg", "plist"],
        tree_sitter_xml::LANGUAGE_XML,
        tree_sitter_xml::XML_HIGHLIGHT_QUERY,
        None
    ),
    lang!(
        "nix",
        &[],
        &["nix"],
        tree_sitter_nix::LANGUAGE,
        tree_sitter_nix::HIGHLIGHTS_QUERY,
        Some(tree_sitter_nix::INJECTIONS_QUERY)
    ),
    lang!(
        "elixir",
        &["ex"],
        &["ex", "exs"],
        tree_sitter_elixir::LANGUAGE,
        tree_sitter_elixir::HIGHLIGHTS_QUERY,
        Some(tree_sitter_elixir::INJECTIONS_QUERY)
    ),
    lang!(
        "haskell",
        &["hs"],
        &["hs"],
        tree_sitter_haskell::LANGUAGE,
        tree_sitter_haskell::HIGHLIGHTS_QUERY,
        Some(tree_sitter_haskell::INJECTIONS_QUERY)
    ),
    lang!(
        "ocaml",
        &["ml"],
        &["ml", "mli"],
        tree_sitter_ocaml::LANGUAGE_OCAML,
        tree_sitter_ocaml::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "scala",
        &[],
        &["scala", "sc", "sbt"],
        tree_sitter_scala::LANGUAGE,
        tree_sitter_scala::HIGHLIGHTS_QUERY,
        None
    ),
    lang!(
        "cmake",
        &[],
        &["cmake"],
        &["CMakeLists.txt"],
        tree_sitter_cmake::LANGUAGE,
        tree_sitter_cmake::HIGHLIGHTS_QUERY,
        None,
        Some(tree_sitter_cmake::INJECTIONS_QUERY)
    ),
    lang!(
        "r",
        &[],
        &["r"],
        tree_sitter_r::LANGUAGE,
        tree_sitter_r::HIGHLIGHTS_QUERY,
        None
    ),
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
        &[],
        tree_sitter_python::LANGUAGE,
        tree_sitter_python::HIGHLIGHTS_QUERY,
        Some(PYTHON_RECEIVERS),
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
    // Hugo layouts: Go templating inside HTML. The HTML grammar
    // carries the markup; the template actions are painted over it by
    // the document, since no maintained tree-sitter grammar for Go
    // templates ships on crates.io.
    lang!(
        "gotmpl",
        &["go-html-template", "gohtml", "hugo-template"],
        &["gohtml", "gotmpl", "tmpl"],
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
    // DerekStride's grammar, published as tree-sitter-sequel: it
    // covers the dialects' common core rather than one vendor's, which
    // is what a file called .sql usually is.
    lang!(
        "sql",
        &[],
        &["sql"],
        &[],
        tree_sitter_sequel::LANGUAGE,
        tree_sitter_sequel::HIGHLIGHTS_QUERY,
        Some(SQL_NUMBER_LITERALS),
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

fn built_in() -> &'static [RegisteredLanguage] {
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

/// Languages loaded from a library at runtime, newest first: a
/// configured grammar for a name the build already knows replaces it,
/// which is what makes a wrong or dated built-in fixable without a
/// release.
static LOADED: std::sync::RwLock<Vec<&'static RegisteredLanguage>> =
    std::sync::RwLock::new(Vec::new());

/// Adds a language loaded at runtime. The spec is leaked: a grammar
/// stays for the life of the process, since the trees that point into
/// it do too.
pub(crate) fn register_loaded(spec: LanguageSpec) {
    let entry: &'static RegisteredLanguage = Box::leak(Box::new(RegisteredLanguage {
        spec: Box::leak(Box::new(spec)),
        compiled: OnceLock::new(),
    }));
    let mut loaded = LOADED.write().expect("loaded languages lock");
    loaded.retain(|other| other.spec.name != entry.spec.name);
    loaded.insert(0, entry);
}

/// Every language, loaded ones first so that they win a name the build
/// also has.
fn all() -> Vec<&'static RegisteredLanguage> {
    let loaded = LOADED.read().expect("loaded languages lock");
    loaded
        .iter()
        .copied()
        .chain(built_in().iter())
        .collect()
}

/// Finds a language by canonical name or alias (case-insensitive).
pub fn by_name(name: &str) -> Option<&'static RegisteredLanguage> {
    let needle = name.to_ascii_lowercase();
    all().into_iter().find(|entry| {
        entry.spec.name == needle || entry.spec.aliases.iter().any(|alias| *alias == needle)
    })
}

/// Finds a language by a file path: an exact file-name match first
/// (Makefile, COMMIT_EDITMSG), then the extension.
pub fn by_path(path: &std::path::Path) -> Option<&'static RegisteredLanguage> {
    // A Hugo layout is Go templating that happens to live in an .html
    // file; the directory is what says so, since the extension cannot.
    if path.extension().and_then(|ext| ext.to_str()) == Some("html")
        && path
            .components()
            .any(|component| component.as_os_str() == "layouts")
    {
        if let Some(entry) = by_name("gotmpl") {
            return Some(entry);
        }
    }
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        if let Some(entry) = all()
            .into_iter()
            .find(|entry| entry.spec.filenames.iter().any(|file| *file == name))
        {
            return Some(entry);
        }
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    all()
        .into_iter()
        .find(|entry| entry.spec.extensions.iter().any(|ext| *ext == extension))
}

/// Every registered language name that files can map to (for UI pickers).
pub fn selectable_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = all()
        .into_iter()
        .filter(|entry| !entry.spec.extensions.is_empty())
        .map(|entry| entry.spec.name)
        .collect();
    names.dedup();
    names
}
