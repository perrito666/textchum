//! The server registry: which language server serves which language.
//!
//! Servers are found in the project's environment, on `PATH` and in
//! the per-user folders installers write to; when one is missing the
//! editor offers to install it (see `textchum_core::install`). Configuration
//! can add servers this table does not know, and redefine the ones it
//! does, under `lsp.servers`.
//!
//! Every command and argument list here comes from that server's own
//! documentation. A server started
//! without its transport flag exits without saying why, so an entry
//! that is a guess is worse than no entry.

/// A language server Textchum knows how to talk to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSpec {
    /// Stable identifier, also part of the instance-pool key.
    pub id: &'static str,
    /// Executable name (resolved on PATH) and arguments.
    pub command: &'static str,
    pub args: &'static [&'static str],
    /// Syntax-language names (as the core reports them) this server
    /// handles.
    pub languages: &'static [&'static str],
}

impl ServerSpec {
    /// How the server is installed, in words, for where it is described
    /// rather than installed; the catalog in `textchum_core::install`
    /// is the one place that knows.
    pub fn install_hint(&self) -> String {
        textchum_core::install::hint(self.command).unwrap_or_default()
    }
}

static SERVERS: &[ServerSpec] = &[
    ServerSpec {
        id: "rust-analyzer",
        command: "rust-analyzer",
        args: &[],
        languages: &["rust"],
    },
    ServerSpec {
        id: "pyright",
        command: "pyright-langserver",
        args: &["--stdio"],
        languages: &["python"],
        // Name the package that provides *this* command first and on
        // its own: an alternative server mentioned in the same breath
        // reads as a second way to satisfy the same request, and
        // installing it alone changes nothing, because the command
        // Textchum looks for is still pyright-langserver.
    },
    ServerSpec {
        id: "basedpyright",
        command: "basedpyright-langserver",
        args: &["--stdio"],
        languages: &["python"],
    },
    ServerSpec {
        id: "pylsp",
        command: "pylsp",
        args: &[],
        languages: &["python"],
    },
    ServerSpec {
        id: "ruff",
        command: "ruff",
        args: &["server"],
        languages: &["python"],
    },
    ServerSpec {
        id: "jedi",
        command: "jedi-language-server",
        args: &[],
        languages: &["python"],
    },
    ServerSpec {
        id: "gopls",
        command: "gopls",
        args: &[],
        languages: &["go", "gotmpl"],
    },
    ServerSpec {
        id: "clangd",
        command: "clangd",
        args: &[],
        languages: &["c", "cpp"],
    },
    ServerSpec {
        id: "typescript-language-server",
        command: "typescript-language-server",
        args: &["--stdio"],
        languages: &["javascript", "typescript", "tsx"],
    },
    ServerSpec {
        id: "jdtls",
        command: "jdtls",
        args: &[],
        languages: &["java"],
    },
    ServerSpec {
        id: "ruby-lsp",
        command: "ruby-lsp",
        args: &[],
        languages: &["ruby"],
    },
    ServerSpec {
        id: "solargraph",
        command: "solargraph",
        args: &["stdio"],
        languages: &["ruby"],
    },
    ServerSpec {
        id: "csharp-ls",
        command: "csharp-ls",
        args: &[],
        languages: &["csharp"],
    },
    ServerSpec {
        id: "lua-language-server",
        command: "lua-language-server",
        args: &[],
        languages: &["lua"],
    },
    ServerSpec {
        id: "nil",
        command: "nil",
        args: &[],
        languages: &["nix"],
    },
    ServerSpec {
        id: "haskell-language-server",
        command: "haskell-language-server-wrapper",
        args: &["--lsp"],
        languages: &["haskell"],
    },
    ServerSpec {
        id: "ocamllsp",
        command: "ocamllsp",
        args: &[],
        languages: &["ocaml"],
    },
    ServerSpec {
        id: "metals",
        command: "metals",
        args: &[],
        languages: &["scala"],
    },
    ServerSpec {
        id: "cmake-language-server",
        command: "cmake-language-server",
        args: &[],
        languages: &["cmake"],
    },
    ServerSpec {
        id: "sourcekit-lsp",
        command: "sourcekit-lsp",
        args: &[],
        languages: &["swift"],
    },
    ServerSpec {
        id: "zls",
        command: "zls",
        args: &[],
        languages: &["zig"],
    },
    ServerSpec {
        id: "bash-language-server",
        command: "bash-language-server",
        args: &["start"],
        languages: &["bash"],
    },
    ServerSpec {
        id: "ty",
        command: "ty",
        args: &["server"],
        languages: &["python"],
    },
    ServerSpec {
        id: "pyrefly",
        command: "pyrefly",
        args: &["lsp"],
        languages: &["python"],
    },
    ServerSpec {
        id: "vtsls",
        command: "vtsls",
        args: &["--stdio"],
        languages: &["javascript", "typescript", "tsx"],
    },
    ServerSpec {
        id: "deno",
        command: "deno",
        args: &["lsp"],
        languages: &["javascript", "typescript", "tsx"],
    },
    ServerSpec {
        id: "biome",
        command: "biome",
        args: &["lsp-proxy"],
        languages: &["javascript", "typescript", "tsx"],
    },
    ServerSpec {
        id: "vscode-json-language-server",
        command: "vscode-json-language-server",
        args: &["--stdio"],
        languages: &["json"],
    },
    ServerSpec {
        id: "vscode-html-language-server",
        command: "vscode-html-language-server",
        args: &["--stdio"],
        languages: &["html"],
    },
    ServerSpec {
        id: "vscode-css-language-server",
        command: "vscode-css-language-server",
        args: &["--stdio"],
        languages: &["css"],
    },
    ServerSpec {
        id: "yaml-language-server",
        command: "yaml-language-server",
        args: &["--stdio"],
        languages: &["yaml"],
    },
    ServerSpec {
        id: "taplo",
        command: "taplo",
        args: &["lsp", "stdio"],
        languages: &["toml"],
    },
    ServerSpec {
        id: "marksman",
        command: "marksman",
        args: &["server"],
        languages: &["markdown"],
    },    ServerSpec {
        id: "elixir-ls",
        command: "elixir-ls",
        args: &[],
        languages: &["elixir"],
    },
    ServerSpec {
        id: "intelephense",
        command: "intelephense",
        args: &["--stdio"],
        languages: &["php"],
    },
    ServerSpec {
        id: "phpactor",
        command: "phpactor",
        args: &["language-server"],
        languages: &["php"],
    },
    ServerSpec {
        id: "docker-langserver",
        command: "docker-langserver",
        args: &["--stdio"],
        languages: &["dockerfile"],
    },
    ServerSpec {
        id: "lemminx",
        command: "lemminx",
        args: &[],
        languages: &["xml"],
    },
    ServerSpec {
        id: "sqls",
        command: "sqls",
        args: &[],
        languages: &["sql"],
    },
    ServerSpec {
        id: "r-languageserver",
        command: "R",
        args: &["--no-echo", "-e", "languageserver::run()"],
        languages: &["r"],
    },
    // rubocop as a server is a linter, not a replacement for the two
    // above: it is asked for by id, usually beside one of them.
    ServerSpec {
        id: "rubocop",
        command: "rubocop",
        args: &["--lsp"],
        languages: &["ruby"],
    },
];

/// The server responsible for a syntax language, if any is registered.
pub fn server_for_language(language: &str) -> Option<&'static ServerSpec> {
    SERVERS
        .iter()
        .find(|spec| spec.languages.contains(&language))
}

/// The server with this id, whatever language it serves.
///
/// A configuration entry that names an id gets that server's command
/// and its required arguments. Several servers now serve one language,
/// and only the first is reachable through [`server_for_language`].
pub fn server_by_id(id: &str) -> Option<&'static ServerSpec> {
    SERVERS.iter().find(|spec| spec.id == id)
}

/// The servers registered for a language, in the order they are listed.
/// The first is what a language gets when configuration says nothing.
pub fn servers_for_language(language: &str) -> Vec<&'static ServerSpec> {
    SERVERS
        .iter()
        .filter(|spec| spec.languages.contains(&language))
        .collect()
}

/// Every server the registry knows, for a settings screen that wants to
/// show what is available rather than only what has been overridden.
pub fn all() -> &'static [ServerSpec] {
    SERVERS
}

/// Whether an executable is reachable — an absolute path that exists, or
/// a bare name found on `PATH` or in the per-user folders installers
/// write to. Settings uses it to say whether a
/// configured command would actually start.
pub fn executable_exists(command: &str) -> bool {
    // The stored value is a command line; the pool splits it on
    // whitespace and runs the first word, so that is what has to exist.
    let Some(program) = command.split_whitespace().next() else {
        return false;
    };
    textchum_core::install::resolve(program, None).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_languages_resolve() {
        assert_eq!(server_for_language("rust").unwrap().id, "rust-analyzer");
        assert_eq!(server_for_language("c").unwrap().id, "clangd");
        assert_eq!(server_for_language("markdown").unwrap().id, "marksman");
        // A language nothing here serves.
        assert!(server_for_language("make").is_none());
    }

    #[test]
    fn every_registered_server_is_reachable() {
        // A language may have several servers, and only the first
        // answers to the language alone. The rest are asked for by id,
        // so every entry has to be findable that way.
        for spec in all() {
            assert_eq!(server_by_id(spec.id).unwrap().command, spec.command);
            for language in spec.languages {
                assert!(
                    servers_for_language(language).iter().any(|s| s.id == spec.id),
                    "{} is not listed for {language}",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn every_language_preset_names_servers_the_registry_has() {
        // A preset writes a server's id, and the pool resolves an id to
        // a command through this table: one that is not here would be
        // run as a command line with no arguments.
        for preset in textchum_core::presets::language_presets() {
            let named = preset
                .servers
                .iter()
                .flat_map(|(_, servers)| servers.split(";;").map(str::trim))
                .chain(preset.settings.iter().map(|(server, _)| *server));
            for server in named {
                assert!(server_by_id(server).is_some(), "{} names {server}", preset.id);
            }
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for spec in all() {
            assert!(seen.insert(spec.id), "two servers share the id {}", spec.id);
        }
    }

    #[test]
    fn a_language_with_several_servers_defaults_to_the_first() {
        let python = servers_for_language("python");
        assert!(python.len() > 1, "python has alternatives to offer");
        assert_eq!(server_for_language("python").unwrap().id, python[0].id);
    }

    #[test]
    fn every_server_has_a_way_to_install_it() {
        // A server the catalog does not know would be reported missing
        // with nothing to offer.
        for spec in all() {
            assert!(
                textchum_core::install::knows(spec.command),
                "{} ({}) is not in the install catalog",
                spec.id,
                spec.command
            );
            assert!(!spec.install_hint().is_empty(), "{} has no hint", spec.id);
        }
        assert!(server_for_language("python").unwrap().install_hint().contains("pyright"));
    }

    #[test]
    fn executables_are_looked_up_the_way_the_pool_runs_them() {
        // A program with arguments is judged by its program.
        assert!(executable_exists("sh -c true"));
        assert!(!executable_exists("textchum-no-such-server --stdio"));
        assert!(!executable_exists(""));
        // An absolute path is taken as given rather than searched for.
        assert!(!executable_exists("/nonexistent/bin/server"));
    }
}
