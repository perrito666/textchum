//! The server registry: which language server serves which language.
//!
//! Curated defaults only for now; user overrides (a `servers.json` with
//! the same escape-hatch rules as the configuration) come later. Servers
//! are found on `PATH` — Textchum does not install them, it tells the user
//! what to install when one is missing.

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
        id: "gopls",
        command: "gopls",
        args: &[],
        languages: &["go"],
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
];

/// The server responsible for a syntax language, if any is registered.
pub fn server_for_language(language: &str) -> Option<&'static ServerSpec> {
    SERVERS
        .iter()
        .find(|spec| spec.languages.contains(&language))
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
        assert!(server_for_language("markdown").is_none());
    }

    #[test]
    fn every_registered_server_is_reachable_by_its_language() {
        for spec in all() {
            for language in spec.languages {
                assert_eq!(server_for_language(language).unwrap().id, spec.id);
            }
        }
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
