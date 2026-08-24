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
        install_hint: "npm install -g pyright",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_languages_resolve() {
        assert_eq!(server_for_language("rust").unwrap().id, "rust-analyzer");
        assert_eq!(server_for_language("c").unwrap().id, "clangd");
        assert!(server_for_language("markdown").is_none());
    }
}
