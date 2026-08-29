//! Captures the bundled grammars emit that the theme's table did not
//! carry. Each has a style of its own now; before, they rendered as
//! plain text.

fn styles_of(source: &str, language: &str) -> Vec<(String, u32)> {
    let mut doc = textchum_core::Document::new();
    doc.replace_utf16(0, 0, source).unwrap();
    doc.set_language(Some(language));
    let units: Vec<u16> = source.encode_utf16().collect();
    doc.highlights(0, units.len())
        .unwrap()
        .iter()
        .map(|s| {
            (
                String::from_utf16_lossy(&units[s.start_utf16..s.end_utf16]),
                s.style,
            )
        })
        .collect()
}

#[test]
fn toml_booleans_are_coloured() {
    let spans = styles_of("enabled = true\nname = \"harbor\"\n", "toml");
    let boolean = spans.iter().find(|(text, _)| text == "true");
    assert!(
        boolean.is_some(),
        "`true` produced no span at all: {spans:?}"
    );
    assert_eq!(
        boolean.unwrap().1,
        textchum_core::theme::resolve("boolean").unwrap(),
        "a TOML boolean paints with the theme's boolean style"
    );
}

#[test]
fn yaml_booleans_are_coloured() {
    let spans = styles_of("enabled: true\nport: 8080\n", "yaml");
    assert!(
        spans.iter().any(|(text, _)| text == "true"),
        "`true` produced no span at all: {spans:?}"
    );
}

/// Every language in the table parses and colours something.
///
/// A grammar whose query fails to compile against it, or whose captures
/// the theme has no colour for, renders as plain text and says nothing
/// about it. This is the guard: a language that ships has to paint.
#[test]
fn every_language_colours_a_sample_of_itself() {
    let samples: &[(&str, &str)] = &[
        ("rust", "fn main() { let s = \"hi\"; }"),
        ("python", "def greet(name):\n    return \"hi\"\n"),
        ("go", "package main\nfunc main() {}\n"),
        ("c", "int main(void) { return 0; }"),
        ("cpp", "#include <vector>\nint main() { return 0; }"),
        ("javascript", "const x = \"hi\";"),
        ("typescript", "const x: string = \"hi\";"),
        ("tsx", "const x = <div className=\"a\" />;"),
        ("java", "class A { void b() {} }"),
        ("csharp", "class A { void B() {} }"),
        ("ruby", "def greet(name)\n  \"hi\"\nend\n"),
        ("php", "<?php function greet($name) { return \"hi\"; }"),
        ("lua", "local function greet(name) return \"hi\" end"),
        ("nix", "{ pkgs }: { name = \"hi\"; }"),
        ("elixir", "defmodule A do\n  def b, do: \"hi\"\nend\n"),
        ("haskell", "main :: IO ()\nmain = putStrLn \"hi\"\n"),
        ("ocaml", "let greet name = \"hi\"\n"),
        ("scala", "object A { def b = \"hi\" }"),
        ("cmake", "project(demo)\nset(NAME \"hi\")\n"),
        ("r", "greet <- function(name) { \"hi\" }"),
        ("xml", "<a href=\"b\">c</a>"),
        ("json", "{\"a\": 1}"),
        ("html", "<p>hi</p>"),
        ("css", "a { color: red; }"),
        ("toml", "a = 1\n"),
        ("yaml", "a: 1\n"),
        ("sql", "select 1;"),
        ("bash", "echo hi\n"),
        ("swift", "let a = \"hi\"\n"),
        ("zig", "const a = \"hi\";\n"),
        ("markdown", "# hi\n"),
        ("make", "all:\n\techo hi\n"),
        ("gotmpl", "{{ if .Name }}hi{{ end }}\n"),
    ];
    for (language, source) in samples {
        let spans = styles_of(source, language);
        assert!(
            !spans.is_empty(),
            "{language} coloured nothing at all — its query names captures \
             the theme has no colour for, or does not compile against the \
             grammar"
        );
    }
}

/// And every selectable language has a sample above, so a grammar added
/// without one is caught rather than left unchecked.
#[test]
fn every_selectable_language_is_covered_by_that_sample() {
    let covered = [
        "rust", "python", "go", "c", "cpp", "javascript", "typescript", "tsx", "java",
        "csharp", "ruby", "php", "lua", "nix", "elixir", "haskell", "ocaml", "scala",
        "cmake", "r", "xml", "json", "html", "css", "toml", "yaml", "sql", "bash",
        "swift", "zig", "markdown", "make", "gotmpl",
    ];
    let missing: Vec<&str> = textchum_core::syntax::languages::selectable_names()
        .into_iter()
        .filter(|name| !covered.contains(name))
        .collect();
    assert!(missing.is_empty(), "no colour sample for {missing:?}");
}
