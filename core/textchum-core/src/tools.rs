//! Known tools: the command line each formatter and fixer takes.
//!
//! A save preprocessor is a command that reads the document on standard
//! input and writes it back on standard output, and every tool spells
//! that differently: `black` wants a trailing `-`, `prettier` wants
//! `--stdin-filepath`, `rustfmt` wants to be told the edition, `rubocop`
//! needs four flags before it stops writing its report over the source.
//! None of that is knowledge a person setting up an editor should have
//! to go and find. This is the table of it, so a settings screen can
//! offer "black" and write the line.
//!
//! The lines use this editor's placeholders (`{path}`, `{filename}`,
//! `{edition}`) and respect its rule that a chain link failing means
//! nothing is applied: a tool that exits non-zero when it merely has
//! something left to complain about is told not to, and one that
//! cannot be told is not in the table.

use serde_json::Value;

use crate::i18n::tr;

/// What a tool does to the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Lays the code out again.
    Formatter,
    /// Applies the fixes its linter knows how to make.
    Fixer,
    /// Sorts and tidies the imports.
    Imports,
}

impl Kind {
    fn summary(self) -> String {
        match self {
            Kind::Formatter => tr("Formats the document"),
            Kind::Fixer => tr("Fixes what its linter can"),
            Kind::Imports => tr("Sorts and tidies the imports"),
        }
    }
}

/// One tool, as a save preprocessor.
pub struct ToolSpec {
    pub id: &'static str,
    /// What it is called, as its users call it.
    pub name: &'static str,
    pub kind: Kind,
    /// The languages it is offered for.
    pub languages: &'static [&'static str],
    /// The whole command line: stdin in, stdout out.
    pub command: &'static str,
}

impl ToolSpec {
    /// The program the command line runs, which is what has to be on
    /// `PATH`.
    pub fn program(&self) -> &'static str {
        self.command.split_whitespace().next().unwrap_or(self.command)
    }

    /// How the program is installed, in words; see `crate::install`.
    pub fn install(&self) -> String {
        crate::install::hint(self.program()).unwrap_or_default()
    }
}

static TOOLS: &[ToolSpec] = &[
    ToolSpec {
        id: "rustfmt",
        name: "rustfmt",
        kind: Kind::Formatter,
        languages: &["rust"],
        command: "rustfmt --edition {edition}",
    },
    ToolSpec {
        id: "gofmt",
        name: "gofmt",
        kind: Kind::Formatter,
        languages: &["go"],
        command: "gofmt",
    },
    ToolSpec {
        id: "goimports",
        name: "goimports",
        kind: Kind::Imports,
        languages: &["go"],
        command: "goimports",
    },
    ToolSpec {
        id: "gofumpt",
        name: "gofumpt",
        kind: Kind::Formatter,
        languages: &["go"],
        command: "gofumpt",
    },
    ToolSpec {
        id: "golines",
        name: "golines",
        kind: Kind::Formatter,
        languages: &["go"],
        command: "golines",
    },
    ToolSpec {
        id: "ruff-format",
        name: "ruff format",
        kind: Kind::Formatter,
        languages: &["python"],
        command: "ruff format --stdin-filename {path} -",
    },
    ToolSpec {
        id: "ruff-fix",
        name: "ruff check --fix",
        kind: Kind::Fixer,
        languages: &["python"],
        command: "ruff check --fix --exit-zero --no-cache --stdin-filename {path} -",
    },
    ToolSpec {
        id: "ruff-imports",
        name: "ruff (imports)",
        kind: Kind::Imports,
        languages: &["python"],
        command: "ruff check --select I --fix --exit-zero --no-cache --stdin-filename {path} -",
    },
    ToolSpec {
        id: "black",
        name: "black",
        kind: Kind::Formatter,
        languages: &["python"],
        command: "black --quiet --stdin-filename {path} -",
    },
    ToolSpec {
        id: "isort",
        name: "isort",
        kind: Kind::Imports,
        languages: &["python"],
        command: "isort --stdout --filename {path} -",
    },
    ToolSpec {
        id: "autopep8",
        name: "autopep8",
        kind: Kind::Formatter,
        languages: &["python"],
        command: "autopep8 -",
    },
    ToolSpec {
        id: "yapf",
        name: "yapf",
        kind: Kind::Formatter,
        languages: &["python"],
        command: "yapf --quiet",
    },
    ToolSpec {
        id: "autoflake",
        name: "autoflake",
        kind: Kind::Fixer,
        languages: &["python"],
        command: "autoflake --remove-all-unused-imports --stdin-display-name {path} -",
    },
    ToolSpec {
        id: "usort",
        name: "usort",
        kind: Kind::Imports,
        languages: &["python"],
        command: "usort format -",
    },
    ToolSpec {
        id: "prettier",
        name: "prettier",
        kind: Kind::Formatter,
        languages: &["javascript", "typescript", "tsx", "css", "html", "json", "yaml", "markdown"],
        command: "prettier --stdin-filepath {path}",
    },
    ToolSpec {
        id: "prettierd",
        name: "prettierd",
        kind: Kind::Formatter,
        languages: &["javascript", "typescript", "tsx", "css", "html", "json", "yaml", "markdown"],
        command: "prettierd {path}",
    },
    ToolSpec {
        id: "biome",
        name: "biome",
        kind: Kind::Formatter,
        languages: &["javascript", "typescript", "tsx", "json", "css"],
        command: "biome format --stdin-file-path {path}",
    },
    ToolSpec {
        id: "dprint",
        name: "dprint",
        kind: Kind::Formatter,
        languages: &["javascript", "typescript", "tsx", "json", "markdown", "toml"],
        command: "dprint fmt --stdin {path}",
    },
    ToolSpec {
        id: "eslint_d",
        name: "eslint_d",
        kind: Kind::Fixer,
        languages: &["javascript", "typescript", "tsx"],
        command: "eslint_d --fix-to-stdout --stdin --stdin-filename {path}",
    },
    ToolSpec {
        id: "clang-format",
        name: "clang-format",
        kind: Kind::Formatter,
        languages: &["c", "cpp", "java", "csharp"],
        command: "clang-format --assume-filename={path}",
    },
    ToolSpec {
        id: "astyle",
        name: "astyle",
        kind: Kind::Formatter,
        languages: &["c", "cpp"],
        command: "astyle",
    },
    ToolSpec {
        id: "shfmt",
        name: "shfmt",
        kind: Kind::Formatter,
        languages: &["bash"],
        command: "shfmt -filename {path}",
    },
    ToolSpec {
        id: "beautysh",
        name: "beautysh",
        kind: Kind::Formatter,
        languages: &["bash"],
        command: "beautysh -",
    },
    ToolSpec {
        id: "stylua",
        name: "stylua",
        kind: Kind::Formatter,
        languages: &["lua"],
        command: "stylua --search-parent-directories --stdin-filepath {path} -",
    },
    ToolSpec {
        id: "zig-fmt",
        name: "zig fmt",
        kind: Kind::Formatter,
        languages: &["zig"],
        command: "zig fmt --stdin",
    },
    ToolSpec {
        id: "terraform-fmt",
        name: "terraform fmt",
        kind: Kind::Formatter,
        languages: &["hcl"],
        command: "terraform fmt -",
    },
    ToolSpec {
        id: "tofu-fmt",
        name: "tofu fmt",
        kind: Kind::Formatter,
        languages: &["hcl"],
        command: "tofu fmt -",
    },
    ToolSpec {
        id: "packer-fmt",
        name: "packer fmt",
        kind: Kind::Formatter,
        languages: &["hcl"],
        command: "packer fmt -",
    },
    ToolSpec {
        id: "nomad-fmt",
        name: "nomad fmt",
        kind: Kind::Formatter,
        languages: &["hcl"],
        command: "nomad fmt -",
    },
    ToolSpec {
        id: "jq",
        name: "jq",
        kind: Kind::Formatter,
        languages: &["json"],
        command: "jq .",
    },
    ToolSpec {
        id: "fixjson",
        name: "fixjson",
        kind: Kind::Formatter,
        languages: &["json"],
        command: "fixjson",
    },
    ToolSpec {
        id: "taplo",
        name: "taplo",
        kind: Kind::Formatter,
        languages: &["toml"],
        command: "taplo format -",
    },
    ToolSpec {
        id: "yamlfmt",
        name: "yamlfmt",
        kind: Kind::Formatter,
        languages: &["yaml"],
        command: "yamlfmt -",
    },
    ToolSpec {
        id: "mdformat",
        name: "mdformat",
        kind: Kind::Formatter,
        languages: &["markdown"],
        command: "mdformat -",
    },
    ToolSpec {
        id: "xmllint",
        name: "xmllint",
        kind: Kind::Formatter,
        languages: &["xml"],
        command: "xmllint --format -",
    },
    ToolSpec {
        id: "sql-formatter",
        name: "sql-formatter",
        kind: Kind::Formatter,
        languages: &["sql"],
        command: "sql-formatter",
    },
    ToolSpec {
        id: "sqlfmt",
        name: "sqlfmt",
        kind: Kind::Formatter,
        languages: &["sql"],
        command: "sqlfmt -",
    },
    ToolSpec {
        id: "pg_format",
        name: "pg_format",
        kind: Kind::Formatter,
        languages: &["sql"],
        command: "pg_format",
    },
    ToolSpec {
        id: "swiftformat",
        name: "swiftformat",
        kind: Kind::Formatter,
        languages: &["swift"],
        command: "swiftformat --stdinpath {path}",
    },
    ToolSpec {
        id: "rubocop",
        name: "rubocop",
        kind: Kind::Fixer,
        languages: &["ruby"],
        command: "rubocop --auto-correct-all --fail-level fatal --stderr --force-exclusion --stdin {path}",
    },
    ToolSpec {
        id: "standardrb",
        name: "standardrb",
        kind: Kind::Fixer,
        languages: &["ruby"],
        command: "standardrb --fix --fail-level fatal --stderr --stdin {path}",
    },
    ToolSpec {
        id: "nixfmt",
        name: "nixfmt",
        kind: Kind::Formatter,
        languages: &["nix"],
        command: "nixfmt",
    },
    ToolSpec {
        id: "alejandra",
        name: "alejandra",
        kind: Kind::Formatter,
        languages: &["nix"],
        command: "alejandra --quiet -",
    },
    ToolSpec {
        id: "nixpkgs-fmt",
        name: "nixpkgs-fmt",
        kind: Kind::Formatter,
        languages: &["nix"],
        command: "nixpkgs-fmt",
    },
    ToolSpec {
        id: "ormolu",
        name: "ormolu",
        kind: Kind::Formatter,
        languages: &["haskell"],
        command: "ormolu --stdin-input-file {path}",
    },
    ToolSpec {
        id: "fourmolu",
        name: "fourmolu",
        kind: Kind::Formatter,
        languages: &["haskell"],
        command: "fourmolu --stdin-input-file {path}",
    },
    ToolSpec {
        id: "stylish-haskell",
        name: "stylish-haskell",
        kind: Kind::Formatter,
        languages: &["haskell"],
        command: "stylish-haskell",
    },
    ToolSpec {
        id: "ocamlformat",
        name: "ocamlformat",
        kind: Kind::Formatter,
        languages: &["ocaml"],
        command: "ocamlformat --enable-outside-detected-project --name {path} -",
    },
    ToolSpec {
        id: "ocp-indent",
        name: "ocp-indent",
        kind: Kind::Formatter,
        languages: &["ocaml"],
        command: "ocp-indent",
    },
    ToolSpec {
        id: "mix-format",
        name: "mix format",
        kind: Kind::Formatter,
        languages: &["elixir"],
        command: "mix format --stdin-filename {path} -",
    },
    ToolSpec {
        id: "scalafmt",
        name: "scalafmt",
        kind: Kind::Formatter,
        languages: &["scala"],
        command: "scalafmt --stdin",
    },
    ToolSpec {
        id: "google-java-format",
        name: "google-java-format",
        kind: Kind::Formatter,
        languages: &["java"],
        command: "google-java-format -",
    },
    ToolSpec {
        id: "cmake-format",
        name: "cmake-format",
        kind: Kind::Formatter,
        languages: &["cmake"],
        command: "cmake-format -",
    },
    ToolSpec {
        id: "gersemi",
        name: "gersemi",
        kind: Kind::Formatter,
        languages: &["cmake"],
        command: "gersemi -",
    },
];

/// The chain link that is not a program: it asks the document's
/// language server to format, which is how a formatter that needs the
/// project's own settings — the crate's edition, gopls's options — gets
/// them without being told on a command line.
pub const SERVER_FORMAT: &str = "@format";

/// Every known tool, in the order they are offered.
pub fn all() -> &'static [ToolSpec] {
    TOOLS
}

/// The command line of the tool with this id. Panics on an id the
/// table does not have: the callers are other tables in this crate,
/// and a typo in one is a build problem.
pub fn command(id: &str) -> &'static str {
    TOOLS
        .iter()
        .find(|tool| tool.id == id)
        .unwrap_or_else(|| panic!("no known tool {id}"))
        .command
}

/// The tools for a settings screen: a JSON array of `{id, name,
/// summary, languages, command, program, install, found}`, `found`
/// saying whether the program is on `PATH` right now.
pub fn tool_presets_json() -> String {
    // First, and for every language: the server's own formatter.
    let server = serde_json::json!({
        "id": "server-format",
        "name": tr("Language server"),
        "summary": tr("Formats with the server's own formatter"),
        "languages": [],
        "command": SERVER_FORMAT,
        "program": "",
        "install": "",
        "found": true,
    });
    let tools: Vec<Value> = std::iter::once(server)
        .chain(TOOLS.iter().map(|tool| {
            serde_json::json!({
                "id": tool.id,
                "name": tool.name,
                "summary": tool.kind.summary(),
                "languages": tool.languages,
                "command": tool.command,
                "program": tool.program(),
                "install": tool.install(),
                "found": crate::install::resolve(tool.program(), None).is_some(),
            })
        }))
        .collect();
    Value::Array(tools).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn ids_are_unique_and_every_language_is_one_the_build_knows() {
        let mut ids: Vec<&str> = TOOLS.iter().map(|tool| tool.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), TOOLS.len());
        for tool in TOOLS {
            assert!(!tool.languages.is_empty(), "{} serves no language", tool.id);
            for language in tool.languages {
                assert!(
                    crate::syntax::languages::by_name(language).is_some(),
                    "{} names a language the build has no grammar for: {language}",
                    tool.id
                );
            }
        }
    }

    #[test]
    fn the_only_placeholders_are_the_ones_the_shells_expand() {
        for tool in TOOLS {
            let mut rest = tool.command;
            while let Some(open) = rest.find('{') {
                let close = rest[open..].find('}').expect("a closed placeholder") + open;
                let name = &rest[open..=close];
                assert!(
                    ["{path}", "{filename}", "{edition}"].contains(&name),
                    "{} uses {name}, which nothing expands",
                    tool.id
                );
                rest = &rest[close + 1..];
            }
        }
    }

    /// A small, valid document in `language`, not laid out the way a
    /// formatter would leave it.
    fn sample(language: &str) -> Option<&'static str> {
        Some(match language {
            "rust" => "fn main(){let x=1;println!(\"{}\",x);}\n",
            "go" => "package main\nfunc main(){\nprintln( 1 )}\n",
            "python" => "import os\nx=[1,2,\n3]\nprint( x,os.sep )\n",
            "javascript" | "typescript" | "tsx" => "const x = {a:1,\nb:2}\n",
            "json" => "{\"a\":1,\n\"b\":[1,2]}\n",
            "css" => "a{color:red}\n",
            "html" => "<p>hello</p>\n",
            "yaml" => "a:   1\nb:\n  - 2\n",
            "markdown" => "# Title\n\n* one\n* two\n",
            "c" | "cpp" => "int main(){return 0;}\n",
            "bash" => "if true;then echo hi;fi\n",
            "lua" => "local x={1,2}\nprint( x )\n",
            "toml" => "a=1\n[b]\nc =  2\n",
            "xml" => "<a><b>1</b></a>\n",
            "sql" => "select a,b from t where a=1;\n",
            "zig" => "pub fn main() void {const x=1;_=x;}\n",
            "hcl" => "a=1\nb   = 2\n",
            "nix" => "{a=1;b=[1 2];}\n",
            "haskell" => "main = do\n  print 1\n",
            "ocaml" => "let x = 1\nlet () = print_int x\n",
            "cmake" => "project( demo )\n",
            "ruby" => "x = [1,2]\nputs x\n",
            "scala" => "object A { val x=1 }\n",
            "java" => "class A{int x=1;}\n",
            _ => return None,
        })
    }

    /// Every tool that happens to be installed is run the way a save
    /// would run it. A machine with none of them proves nothing and
    /// fails nothing; one with rustfmt, jq and xmllint checks three
    /// lines of the table against the real programs.
    #[test]
    fn an_installed_tool_takes_the_line_the_table_gives_it() {
        let directory = std::env::temp_dir().join(format!("textchum-tools-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut ran = Vec::new();
        for tool in TOOLS {
            if !crate::presets::on_path(tool.program()) {
                continue;
            }
            let language = tool.languages[0];
            let Some(text) = sample(language) else { continue };
            let extension = crate::syntax::languages::by_name(language)
                .and_then(|language| language.spec.extensions.first().copied())
                .unwrap_or("txt");
            let path = directory.join(format!("sample.{extension}"));
            std::fs::write(&path, text).unwrap();
            let path = path.to_string_lossy().into_owned();
            let words: Vec<String> = tool
                .command
                .split_whitespace()
                .map(|word| {
                    word.replace("{path}", &path)
                        .replace("{filename}", &format!("sample.{extension}"))
                        .replace("{edition}", "2021")
                })
                .collect();
            let mut child = Command::new(&words[0])
                .args(&words[1..])
                .current_dir(&directory)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap_or_else(|error| panic!("{} did not start: {error}", tool.id));
            child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success() && !output.stdout.is_empty(),
                "{} ({}) failed on a {language} sample: {}{}",
                tool.id,
                tool.command,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            ran.push(tool.id);
        }
        eprintln!("tools checked against the real programs: {ran:?}");
    }
}
