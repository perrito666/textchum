//! The server registry: which language server serves which language.
//!
//! Servers are found on `PATH` — Textchum does not install them, it
//! tells the user what to install when one is missing. Configuration
//! can add servers this table does not know, and redefine the ones it
//! does, under `lsp.servers`.
//!
//! Every command and argument list here comes from that server's own
//! documentation, and the install hints are ours. A server started
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
    /// Shown when the executable is missing.
    pub install_hint: &'static str,
}

static SERVERS: &[ServerSpec] = &[
    ServerSpec {
        id: "rust-analyzer",
        command: "rust-analyzer",
        args: &[],
        languages: &["rust"],
        install_hint: "rustup component add rust-analyzer",
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
        install_hint: "npm install -g pyright (or: uv tool install pyright). \
                       To use python-lsp-server instead, install it and set \
                       python to pylsp in Preferences → Language Servers",
    },
    ServerSpec {
        id: "basedpyright",
        command: "basedpyright-langserver",
        args: &["--stdio"],
        languages: &["python"],
        install_hint: "uv tool install basedpyright (or: npm install -g basedpyright)",
    },
    ServerSpec {
        id: "pylsp",
        command: "pylsp",
        args: &[],
        languages: &["python"],
        install_hint: "uv tool install python-lsp-server \
                       (or: pipx install python-lsp-server)",
    },
    ServerSpec {
        id: "ruff",
        command: "ruff",
        args: &["server"],
        languages: &["python"],
        install_hint: "uv tool install ruff (or: pipx install ruff)",
    },
    ServerSpec {
        id: "jedi",
        command: "jedi-language-server",
        args: &[],
        languages: &["python"],
        install_hint: "uv tool install jedi-language-server \
                       (or: pipx install jedi-language-server)",
    },
    ServerSpec {
        id: "gopls",
        command: "gopls",
        args: &[],
        languages: &["go", "gotmpl"],
        install_hint: "go install golang.org/x/tools/gopls@latest",
    },
    ServerSpec {
        id: "clangd",
        command: "clangd",
        args: &[],
        languages: &["c"],
        install_hint: "xcode-select --install (or brew install llvm)",
    },
    ServerSpec {
        id: "typescript-language-server",
        command: "typescript-language-server",
        args: &["--stdio"],
        languages: &["javascript"],
        install_hint: "npm install -g typescript-language-server typescript",
    },
    ServerSpec {
        id: "sourcekit-lsp",
        command: "sourcekit-lsp",
        args: &[],
        languages: &["swift"],
        install_hint: "ships with the Xcode toolchain",
    },
    ServerSpec {
        id: "zls",
        command: "zls",
        args: &[],
        languages: &["zig"],
        install_hint: "brew install zls",
    },
    ServerSpec {
        id: "bash-language-server",
        command: "bash-language-server",
        args: &["start"],
        languages: &["bash"],
        install_hint: "npm install -g bash-language-server",
    },
    ServerSpec {
        id: "ty",
        command: "ty",
        args: &["server"],
        languages: &["python"],
        install_hint: "uv tool install ty",
    },
    ServerSpec {
        id: "pyrefly",
        command: "pyrefly",
        args: &["lsp"],
        languages: &["python"],
        install_hint: "uv tool install pyrefly",
    },
    ServerSpec {
        id: "vtsls",
        command: "vtsls",
        args: &["--stdio"],
        languages: &["javascript"],
        install_hint: "npm install -g @vtsls/language-server",
    },
    ServerSpec {
        id: "deno",
        command: "deno",
        args: &["lsp"],
        languages: &["javascript"],
        install_hint: "brew install deno",
    },
    ServerSpec {
        id: "biome",
        command: "biome",
        args: &["lsp-proxy"],
        languages: &["javascript"],
        install_hint: "npm install -g @biomejs/biome",
    },
    ServerSpec {
        id: "vscode-json-language-server",
        command: "vscode-json-language-server",
        args: &["--stdio"],
        languages: &["json"],
        install_hint: "npm install -g vscode-langservers-extracted",
    },
    ServerSpec {
        id: "vscode-html-language-server",
        command: "vscode-html-language-server",
        args: &["--stdio"],
        languages: &["html"],
        install_hint: "npm install -g vscode-langservers-extracted",
    },
    ServerSpec {
        id: "vscode-css-language-server",
        command: "vscode-css-language-server",
        args: &["--stdio"],
        languages: &["css"],
        install_hint: "npm install -g vscode-langservers-extracted",
    },
    ServerSpec {
        id: "yaml-language-server",
        command: "yaml-language-server",
        args: &["--stdio"],
        languages: &["yaml"],
        install_hint: "npm install -g yaml-language-server",
    },
    ServerSpec {
        id: "taplo",
        command: "taplo",
        args: &["lsp", "stdio"],
        languages: &["toml"],
        install_hint: "brew install taplo \
                       (or: cargo install taplo-cli --features lsp)",
    },
    ServerSpec {
        id: "marksman",
        command: "marksman",
        args: &["server"],
        languages: &["markdown"],
        install_hint: "brew install marksman",
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
/// a bare name found on `PATH`. Settings uses it to say whether a
/// configured command would actually start.
pub fn executable_exists(command: &str) -> bool {
    // The stored value is a command line; the pool splits it on
    // whitespace and runs the first word, so that is what has to exist.
    let Some(program) = command.split_whitespace().next() else {
        return false;
    };
    let program = std::path::Path::new(program);
    if program.components().count() > 1 {
        return program.is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
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
    fn install_hints_name_the_command_they_install() {
        // The pyright hint used to offer python-lsp-server in the same
        // sentence, and a reader who installed only that got nothing:
        // the command Textchum looks for is still pyright-langserver.
        let hint = server_for_language("python").unwrap().install_hint;
        let pyright = hint.find("pyright").expect("names pyright");
        let alternative = hint.find("python-lsp-server").expect("names the alternative");
        assert!(pyright < alternative);
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
