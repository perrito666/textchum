//! Language presets: what a language needs besides a grammar, in one
//! step.
//!
//! A language is rarely one tool. Rust is rust-analyzer checking with
//! clippy and rustfmt on save; Python is a type checker and ruff; Go is
//! gopls with staticcheck and goimports. Each of those is something the
//! configuration can already say — a server under `lsp.defaults`, its
//! settings under `lsp.settings`, a chain under `preprocessors` — and a
//! preset is nothing more than the entries a person who knows the
//! language would write there. Applying one writes them, so what is in
//! force afterwards is the ordinary configuration, there to be edited:
//! a preset is a starting point and not a second store.
//!
//! The table lives here rather than in a shell because both shells
//! offer it and neither should know what rustfmt takes on its command
//! line.

use std::path::Path;

use serde_json::{json, Value};

use crate::i18n::tr;

/// A program a preset runs, and how to get it.
pub struct Tool {
    pub command: &'static str,
    pub install: &'static str,
}

/// One language's worth of configuration.
pub struct LanguagePreset {
    pub id: &'static str,
    pub name: String,
    /// What applying it sets up, in a sentence.
    pub summary: String,
    /// What it does not do that a reader might expect; empty when
    /// there is nothing to own up to.
    pub missing: String,
    /// (language, server) pairs for `lsp.defaults`. The server is a
    /// registry id, so the registry supplies its arguments.
    pub servers: Vec<(&'static str, &'static str)>,
    /// (server id, settings) pairs for `lsp.settings`.
    pub settings: Vec<(&'static str, Value)>,
    /// (language, chain) pairs for `preprocessors.defaults`. Every
    /// command reads the document on stdin and writes it on stdout.
    pub preprocessors: Vec<(&'static str, Vec<&'static str>)>,
    pub tools: Vec<Tool>,
}

/// The presets the build offers, in the order they are shown.
pub fn language_presets() -> Vec<LanguagePreset> {
    vec![
        LanguagePreset {
            id: "rust",
            name: "Rust".into(),
            summary: tr("rust-analyzer checking with clippy, and rustfmt before every save."),
            missing: String::new(),
            servers: vec![("rust", "rust-analyzer")],
            settings: vec![(
                "rust-analyzer",
                json!({"rust-analyzer": {"check": {"command": "clippy"}}}),
            )],
            // rustfmt reading stdin does not look at Cargo.toml, and
            // without an edition it assumes 2015; {edition} is the
            // nearest manifest's.
            preprocessors: vec![("rust", vec!["rustfmt --edition {edition}"])],
            tools: vec![
                Tool { command: "rust-analyzer", install: "rustup component add rust-analyzer" },
                Tool { command: "cargo-clippy", install: "rustup component add clippy" },
                Tool { command: "rustfmt", install: "rustup component add rustfmt" },
            ],
        },
        LanguagePreset {
            id: "python",
            name: "Python".into(),
            summary: tr(
                "pyright for types and navigation, and ruff fixing and formatting before every save.",
            ),
            missing: tr(
                "ruff's own findings do not show in the editor: a document has one server, and here it is pyright.",
            ),
            servers: vec![("python", "pyright")],
            settings: vec![],
            preprocessors: vec![(
                "python",
                vec![
                    "ruff check --fix --exit-zero --no-cache --stdin-filename {path} -",
                    "ruff format --stdin-filename {path} -",
                ],
            )],
            tools: vec![
                Tool { command: "pyright-langserver", install: "npm install -g pyright" },
                Tool { command: "ruff", install: "uv tool install ruff" },
            ],
        },
        LanguagePreset {
            id: "go",
            name: "Go".into(),
            summary: tr("gopls with staticcheck on, and goimports before every save."),
            missing: String::new(),
            servers: vec![("go", "gopls")],
            settings: vec![("gopls", json!({"gopls": {"staticcheck": true}}))],
            preprocessors: vec![("go", vec!["goimports"])],
            tools: vec![
                Tool { command: "gopls", install: "go install golang.org/x/tools/gopls@latest" },
                Tool {
                    command: "goimports",
                    install: "go install golang.org/x/tools/cmd/goimports@latest",
                },
            ],
        },
        LanguagePreset {
            id: "typescript",
            name: tr("TypeScript and JavaScript"),
            summary: tr("typescript-language-server, and prettier before every save."),
            missing: tr(
                "eslint's findings do not show in the editor: a document has one server.",
            ),
            servers: vec![
                ("javascript", "typescript-language-server"),
                ("typescript", "typescript-language-server"),
                ("tsx", "typescript-language-server"),
            ],
            settings: vec![],
            preprocessors: vec![
                ("javascript", vec!["prettier --stdin-filepath {path}"]),
                ("typescript", vec!["prettier --stdin-filepath {path}"]),
                ("tsx", vec!["prettier --stdin-filepath {path}"]),
            ],
            tools: vec![
                Tool {
                    command: "typescript-language-server",
                    install: "npm install -g typescript-language-server typescript",
                },
                Tool { command: "prettier", install: "npm install -g prettier" },
            ],
        },
        LanguagePreset {
            id: "c",
            name: tr("C and C++"),
            summary: tr("clangd, and clang-format before every save."),
            missing: String::new(),
            servers: vec![("c", "clangd"), ("cpp", "clangd")],
            settings: vec![],
            preprocessors: vec![
                ("c", vec!["clang-format --assume-filename={path}"]),
                ("cpp", vec!["clang-format --assume-filename={path}"]),
            ],
            tools: vec![
                Tool { command: "clangd", install: "brew install llvm, or apt install clangd" },
                Tool {
                    command: "clang-format",
                    install: "brew install clang-format, or apt install clang-format",
                },
            ],
        },
    ]
}

/// The preset with this id.
pub fn language_preset(id: &str) -> Option<LanguagePreset> {
    language_presets().into_iter().find(|preset| preset.id == id)
}

/// Whether `command` is an executable somewhere on this process's
/// `PATH` — the same place a server or a preprocessor will be looked
/// for when it is run.
pub fn on_path(command: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(command)))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The Rust edition of the crate a file belongs to, for the
/// `{edition}` a preprocessor command may name.
///
/// rustfmt run over stdin never sees `Cargo.toml`, which is where the
/// edition is written, and formats as 2015 unless told: `async` blocks
/// fail to parse, and the 2024 style differs from the 2021 one. So the
/// nearest manifest is read here — its `[package]` edition, or its
/// workspace's when it says `edition.workspace = true`. A manifest
/// with a package and no edition means 2015, as it does to cargo; no
/// manifest at all gets 2021, the least surprising guess for a loose
/// file. The manifest is read line by line, not parsed: this wants one
/// key from one table.
pub fn rust_edition(path: &Path) -> String {
    let mut inherited = false;
    for directory in path.ancestors().skip(1) {
        let Ok(manifest) = std::fs::read_to_string(directory.join("Cargo.toml")) else {
            continue;
        };
        let wanted = if inherited { "workspace.package" } else { "package" };
        let mut table = String::new();
        let mut has_table = false;
        let mut found = None;
        for line in manifest.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                table = line.trim_matches(|c| c == '[' || c == ']').trim().to_owned();
                has_table |= table == wanted;
                continue;
            }
            if table != wanted {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "edition" if value.contains("workspace") => inherited = true,
                "edition" => found = value.split('"').nth(1).map(str::to_owned),
                "edition.workspace" => inherited = true,
                _ => {}
            }
        }
        if let Some(edition) = found {
            return edition;
        }
        if has_table && !inherited {
            return "2015".into();
        }
    }
    "2021".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_every_language_is_one_the_build_knows() {
        let presets = language_presets();
        let mut ids: Vec<&str> = presets.iter().map(|preset| preset.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), presets.len());
        for preset in &presets {
            let languages = preset
                .servers
                .iter()
                .map(|(language, _)| *language)
                .chain(preset.preprocessors.iter().map(|(language, _)| *language));
            for language in languages {
                assert!(
                    crate::syntax::languages::by_name(language).is_some(),
                    "{} names a language the build has no grammar for: {language}",
                    preset.id
                );
            }
            assert!(!preset.tools.is_empty(), "{} lists no tools", preset.id);
        }
    }

    #[test]
    fn settings_are_objects_keyed_by_section() {
        for preset in language_presets() {
            for (server, settings) in &preset.settings {
                assert!(settings.is_object(), "{server}'s settings are not an object");
            }
        }
    }

    #[test]
    fn every_command_a_preset_runs_is_one_of_its_tools() {
        for preset in language_presets() {
            for (_, chain) in &preset.preprocessors {
                for command in chain {
                    let program = command.split_whitespace().next().unwrap();
                    assert!(
                        preset.tools.iter().any(|tool| tool.command == program),
                        "{} runs {program} without listing it",
                        preset.id
                    );
                }
            }
        }
    }

    fn crate_at(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let root = std::env::temp_dir()
            .join(format!("textchum-edition-{}", std::process::id()))
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        for (path, contents) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, contents).unwrap();
        }
        root
    }

    #[test]
    fn the_edition_is_the_nearest_packages() {
        let root = crate_at(
            "plain",
            &[
                ("Cargo.toml", "[package]\nname = \"a\"\nedition = \"2024\" # newest\n\n[dependencies]\nedition = \"x\"\n"),
                ("src/main.rs", ""),
            ],
        );
        assert_eq!(rust_edition(&root.join("src/main.rs")), "2024");
    }

    #[test]
    fn a_member_that_inherits_takes_the_workspaces() {
        let root = crate_at(
            "workspace",
            &[
                ("Cargo.toml", "[workspace]\nmembers = [\"member\"]\n\n[workspace.package]\nedition = \"2021\"\n"),
                ("member/Cargo.toml", "[package]\nname = \"m\"\nedition.workspace = true\n"),
                ("member/src/lib.rs", ""),
                ("other/Cargo.toml", "[package]\nname = \"o\"\nedition = { workspace = true }\n"),
                ("other/src/lib.rs", ""),
            ],
        );
        assert_eq!(rust_edition(&root.join("member/src/lib.rs")), "2021");
        assert_eq!(rust_edition(&root.join("other/src/lib.rs")), "2021");
    }

    #[test]
    fn a_package_without_an_edition_is_2015_and_no_manifest_is_2021() {
        let root = crate_at(
            "old",
            &[("Cargo.toml", "[package]\nname = \"old\"\n"), ("src/lib.rs", "")],
        );
        assert_eq!(rust_edition(&root.join("src/lib.rs")), "2015");
        let loose = std::env::temp_dir().join("textchum-edition-none-such/file.rs");
        // Nothing above a temporary directory is a crate on any machine
        // these tests run on.
        assert_eq!(rust_edition(&loose), "2021");
    }

    #[test]
    fn a_tool_is_found_where_the_shell_would_find_it() {
        assert!(on_path("sh"));
        assert!(!on_path("no-such-tool-textchum-would-run"));
    }
}
