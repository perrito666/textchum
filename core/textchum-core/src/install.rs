//! Installing the programs the editor runs: language servers, save
//! formatters and debug adapters.
//!
//! The editor does not tell the person to install something it can
//! install itself. For each program it knows, this module knows the
//! ways it is distributed — a Python package, an npm package, a Go
//! module, a rustup component, a Homebrew formula, … — and turns them
//! into the commands that would install it here: only through the
//! installers this machine has, into the project's own environment
//! where the ecosystem has one, and for the user otherwise. A shell
//! offers them side by side and runs the one chosen where runs are
//! shown.
//!
//! The same table answers "how is this installed" wherever the editor
//! used to print a hint, so the sentence in Settings and the command
//! that runs cannot disagree.
//!
//! What it installs into a project has to be found again, so
//! [`resolve`] looks where those installs land — the project's
//! `.venv/bin`, `node_modules/.bin`, the module's `go tool` list —
//! before `PATH`, and after `PATH` in the per-user folders installers
//! write to and a desktop session's `PATH` often lacks.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::i18n::{fill, tr};

/// Where an install puts a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Into the project: its environment, recorded in its manifest or not.
    Project,
    /// For this user, outside any one project.
    User,
    /// Through the system's package manager, which needs an
    /// administrator; shown, never run.
    System,
}

impl Scope {
    fn name(self) -> &'static str {
        match self {
            Scope::Project => "project",
            Scope::User => "user",
            Scope::System => "system",
        }
    }
}

/// One way to install a program, ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Method {
    /// The program that does the installing: `uv`, `npm`, `go`, …
    pub installer: &'static str,
    pub scope: Scope,
    /// Where it goes, in words.
    pub title: String,
    /// The shell command line.
    pub command: String,
    /// Where it runs: the project for a project install, else home.
    pub directory: PathBuf,
    /// False for what needs an administrator's password: a run panel
    /// has no terminal to type it into, so the command is handed over
    /// instead.
    pub runs: bool,
}

/// How a program is distributed.
#[derive(Debug, Clone, Copy)]
enum Source {
    /// A Python package that installs the program as a script.
    Python(&'static str),
    /// A Python package the interpreter imports — a module, not a
    /// program, so it has to go where the project's Python looks.
    PythonModule(&'static str),
    /// npm packages, space separated.
    Npm(&'static str),
    /// A Go module path, installed at its latest version.
    Go(&'static str),
    Rustup(&'static str),
    /// A crate, with any flags `cargo install` needs.
    Cargo(&'static str),
    Gem(&'static str),
    Brew(&'static str),
    Dotnet(&'static str),
    Opam(&'static str),
    Cabal(&'static str),
    Ghcup(&'static str),
    Composer(&'static str),
    Nix(&'static str),
    Coursier(&'static str),
    /// The system's package, on Linux.
    System(&'static str),
    /// macOS's command-line developer tools.
    XcodeTools,
    /// Nothing to run: where the program comes from, in words.
    Note(&'static str),
}

struct Recipe {
    /// The program as a command line names it, or for a module, the
    /// module.
    program: &'static str,
    sources: &'static [Source],
}

use Source::*;

static RECIPES: &[Recipe] = &[
    // Language servers.
    Recipe { program: "rust-analyzer", sources: &[Rustup("rust-analyzer"), Brew("rust-analyzer")] },
    Recipe { program: "pyright-langserver", sources: &[Python("pyright"), Npm("pyright")] },
    Recipe { program: "basedpyright-langserver", sources: &[Python("basedpyright"), Npm("basedpyright")] },
    Recipe { program: "pylsp", sources: &[Python("python-lsp-server")] },
    Recipe { program: "ruff", sources: &[Python("ruff"), Brew("ruff")] },
    Recipe { program: "jedi-language-server", sources: &[Python("jedi-language-server")] },
    Recipe { program: "ty", sources: &[Python("ty")] },
    Recipe { program: "pyrefly", sources: &[Python("pyrefly")] },
    Recipe { program: "cmake-language-server", sources: &[Python("cmake-language-server")] },
    Recipe { program: "gopls", sources: &[Go("golang.org/x/tools/gopls"), Brew("gopls")] },
    Recipe { program: "sqls", sources: &[Go("github.com/sqls-server/sqls")] },
    Recipe { program: "clangd", sources: &[XcodeTools, Brew("llvm"), System("clangd")] },
    Recipe { program: "sourcekit-lsp", sources: &[XcodeTools, Note("ships with the Swift toolchain")] },
    Recipe {
        program: "typescript-language-server",
        sources: &[Npm("typescript-language-server typescript")],
    },
    Recipe { program: "vtsls", sources: &[Npm("@vtsls/language-server")] },
    Recipe { program: "bash-language-server", sources: &[Npm("bash-language-server")] },
    Recipe { program: "vscode-json-language-server", sources: &[Npm("vscode-langservers-extracted")] },
    Recipe { program: "vscode-html-language-server", sources: &[Npm("vscode-langservers-extracted")] },
    Recipe { program: "vscode-css-language-server", sources: &[Npm("vscode-langservers-extracted")] },
    Recipe { program: "yaml-language-server", sources: &[Npm("yaml-language-server")] },
    Recipe { program: "intelephense", sources: &[Npm("intelephense")] },
    Recipe { program: "docker-langserver", sources: &[Npm("dockerfile-language-server-nodejs")] },
    Recipe { program: "biome", sources: &[Npm("@biomejs/biome"), Brew("biome")] },
    Recipe { program: "deno", sources: &[Brew("deno")] },
    Recipe { program: "jdtls", sources: &[Brew("jdtls")] },
    Recipe { program: "ruby-lsp", sources: &[Gem("ruby-lsp")] },
    Recipe { program: "solargraph", sources: &[Gem("solargraph")] },
    Recipe { program: "rubocop", sources: &[Gem("rubocop")] },
    Recipe { program: "csharp-ls", sources: &[Dotnet("csharp-ls")] },
    Recipe { program: "lua-language-server", sources: &[Brew("lua-language-server")] },
    Recipe { program: "nil", sources: &[Nix("nil")] },
    Recipe {
        program: "haskell-language-server-wrapper",
        sources: &[Ghcup("hls"), Brew("haskell-language-server")],
    },
    Recipe { program: "ocamllsp", sources: &[Opam("ocaml-lsp-server")] },
    Recipe { program: "metals", sources: &[Coursier("metals")] },
    Recipe { program: "zls", sources: &[Brew("zls")] },
    Recipe { program: "taplo", sources: &[Brew("taplo"), Cargo("taplo-cli --locked --features lsp")] },
    Recipe { program: "marksman", sources: &[Brew("marksman")] },
    Recipe { program: "elixir-ls", sources: &[Brew("elixir-ls")] },
    Recipe { program: "phpactor", sources: &[Composer("phpactor/phpactor")] },
    Recipe { program: "lemminx", sources: &[Brew("lemminx")] },
    Recipe {
        program: "R",
        sources: &[
            Brew("r"),
            System("r-base"),
            Note("then, in R: install.packages(\"languageserver\")"),
        ],
    },
    // Save formatters and fixers.
    Recipe { program: "rustfmt", sources: &[Rustup("rustfmt")] },
    Recipe { program: "cargo-clippy", sources: &[Rustup("clippy")] },
    Recipe { program: "gofmt", sources: &[Note("comes with Go"), Brew("go"), System("golang-go")] },
    Recipe { program: "go", sources: &[Brew("go"), System("golang-go")] },
    Recipe { program: "goimports", sources: &[Go("golang.org/x/tools/cmd/goimports")] },
    Recipe { program: "gofumpt", sources: &[Go("mvdan.cc/gofumpt")] },
    Recipe { program: "golines", sources: &[Go("github.com/segmentio/golines")] },
    Recipe { program: "black", sources: &[Python("black")] },
    Recipe { program: "isort", sources: &[Python("isort")] },
    Recipe { program: "autopep8", sources: &[Python("autopep8")] },
    Recipe { program: "yapf", sources: &[Python("yapf")] },
    Recipe { program: "autoflake", sources: &[Python("autoflake")] },
    Recipe { program: "usort", sources: &[Python("usort")] },
    Recipe { program: "beautysh", sources: &[Python("beautysh")] },
    Recipe { program: "mdformat", sources: &[Python("mdformat")] },
    Recipe { program: "sqlfmt", sources: &[Python("shandy-sqlfmt")] },
    Recipe { program: "cmake-format", sources: &[Python("cmakelang")] },
    Recipe { program: "gersemi", sources: &[Python("gersemi")] },
    Recipe { program: "prettier", sources: &[Npm("prettier")] },
    Recipe { program: "prettierd", sources: &[Npm("@fsouza/prettierd")] },
    Recipe { program: "eslint_d", sources: &[Npm("eslint_d")] },
    Recipe { program: "fixjson", sources: &[Npm("fixjson")] },
    Recipe { program: "sql-formatter", sources: &[Npm("sql-formatter")] },
    Recipe { program: "dprint", sources: &[Brew("dprint"), Cargo("dprint --locked")] },
    Recipe { program: "clang-format", sources: &[Brew("clang-format"), System("clang-format")] },
    Recipe { program: "astyle", sources: &[Brew("astyle"), System("astyle")] },
    Recipe { program: "shfmt", sources: &[Brew("shfmt"), Go("mvdan.cc/sh/v3/cmd/shfmt")] },
    Recipe { program: "stylua", sources: &[Brew("stylua"), Cargo("stylua --locked")] },
    Recipe { program: "zig", sources: &[Brew("zig"), Note("comes with Zig")] },
    Recipe { program: "terraform", sources: &[Brew("hashicorp/tap/terraform")] },
    Recipe { program: "tofu", sources: &[Brew("opentofu")] },
    Recipe { program: "packer", sources: &[Brew("hashicorp/tap/packer")] },
    Recipe { program: "nomad", sources: &[Brew("hashicorp/tap/nomad")] },
    Recipe { program: "jq", sources: &[Brew("jq"), System("jq")] },
    Recipe { program: "yamlfmt", sources: &[Go("github.com/google/yamlfmt/cmd/yamlfmt")] },
    Recipe { program: "xmllint", sources: &[Note("comes with macOS"), System("libxml2-utils")] },
    Recipe { program: "pg_format", sources: &[Brew("pgformatter"), System("pgformatter")] },
    Recipe { program: "swiftformat", sources: &[Brew("swiftformat")] },
    Recipe { program: "standardrb", sources: &[Gem("standard")] },
    Recipe { program: "nixfmt", sources: &[Nix("nixfmt-rfc-style")] },
    Recipe { program: "alejandra", sources: &[Nix("alejandra")] },
    Recipe { program: "nixpkgs-fmt", sources: &[Nix("nixpkgs-fmt")] },
    Recipe { program: "ormolu", sources: &[Cabal("ormolu")] },
    Recipe { program: "fourmolu", sources: &[Cabal("fourmolu")] },
    Recipe { program: "stylish-haskell", sources: &[Cabal("stylish-haskell")] },
    Recipe { program: "ocamlformat", sources: &[Opam("ocamlformat")] },
    Recipe { program: "ocp-indent", sources: &[Opam("ocp-indent")] },
    Recipe { program: "mix", sources: &[Brew("elixir"), Note("comes with Elixir")] },
    Recipe { program: "scalafmt", sources: &[Coursier("scalafmt")] },
    Recipe { program: "google-java-format", sources: &[Brew("google-java-format")] },
    // Debug adapters.
    Recipe { program: "debugpy", sources: &[PythonModule("debugpy")] },
    Recipe { program: "dlv", sources: &[Go("github.com/go-delve/delve/cmd/dlv"), Brew("delve")] },
    Recipe { program: "lldb-dap", sources: &[XcodeTools, Brew("llvm"), System("lldb")] },
];

fn recipe(program: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|recipe| recipe.program == program)
}

/// Whether the catalog knows how `program` is installed.
pub fn knows(program: &str) -> bool {
    recipe(program).is_some()
}

/// Whether `program` is a module the interpreter imports rather than a
/// program found on a path.
pub fn is_module(program: &str) -> bool {
    recipe(program).is_some_and(|recipe| {
        recipe.sources.iter().any(|source| matches!(source, PythonModule(_)))
    })
}

/// What the machine offers to install with. Probed once per question;
/// tests build their own.
#[derive(Debug, Clone, Default)]
pub struct Host {
    /// The installers found: `uv`, `npm`, `go`, … — and `pip` when
    /// `python3` has pip and lets it install for the user, which a
    /// Python the system manages refuses.
    pub installers: Vec<String>,
    /// Whether `go get -tool` exists (Go 1.24 and later).
    pub go_tools: bool,
    pub macos: bool,
    pub home: PathBuf,
}

const INSTALLERS: &[&str] = &[
    "uv", "pipx", "python3", "npm", "go", "rustup", "cargo", "gem", "brew", "dotnet", "opam",
    "cabal", "ghcup", "composer", "nix", "cs", "apt-get", "dnf", "pacman", "zypper",
];

impl Host {
    /// This machine, as it is now, for installing into `project`. Asks
    /// Go its version only when the project is a Go module.
    pub fn probe(project: Option<&Path>) -> Self {
        let installers = installers_here();
        let go_tools = installers.iter().any(|name| name == "go")
            && project.and_then(go_module_root).is_some()
            && go_has_tool_directive();
        Self { installers, go_tools, macos: cfg!(target_os = "macos"), home: home() }
    }

    fn has(&self, installer: &str) -> bool {
        self.installers.iter().any(|name| name == installer)
    }
}

/// The installers on this machine. Settings asks for a hint per server,
/// so the answer is kept a few seconds rather than walking `PATH` for
/// every row; an installer that arrives later is seen at the next
/// question after that.
fn installers_here() -> Vec<String> {
    static KEPT: Mutex<Option<(std::time::Instant, Vec<String>)>> = Mutex::new(None);
    let mut kept = KEPT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((at, installers)) = kept.as_ref() {
        if at.elapsed() < std::time::Duration::from_secs(5) {
            return installers.clone();
        }
    }
    let mut installers: Vec<String> = INSTALLERS
        .iter()
        .filter(|name| find_program(name, None).is_some())
        .map(|name| name.to_string())
        .collect();
    if installers.iter().any(|name| name == "python3") {
        let (pip, venv) = python_abilities();
        if pip {
            installers.push("pip".to_owned());
        }
        if venv {
            installers.push("venv".to_owned());
        }
    }
    *kept = Some((std::time::Instant::now(), installers.clone()));
    installers
}

/// What `python3` can do here: install with pip for the user — pip is
/// there, and the interpreter is not one the system manages (PEP 668),
/// as Debian's, Ubuntu's and Homebrew's are — and make a virtual
/// environment, which needs ensurepip, a separate package on Debian
/// and Ubuntu.
fn python_abilities() -> (bool, bool) {
    let Some(python) = find_program("python3", None) else { return (false, false) };
    let Ok(output) = std::process::Command::new(python)
        .args([
            "-c",
            "import importlib.util as u, os, sysconfig\n\
             managed = os.path.exists(os.path.join(sysconfig.get_path('stdlib'), 'EXTERNALLY-MANAGED'))\n\
             print(u.find_spec('pip') is not None and not managed, u.find_spec('ensurepip') is not None)",
        ])
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return (false, false);
    };
    let answer = String::from_utf8_lossy(&output.stdout);
    let mut words = answer.split_whitespace();
    (words.next() == Some("True"), words.next() == Some("True"))
}

/// `go env GOVERSION` is 1.24 or later, the release that added tool
/// dependencies to go.mod.
fn go_has_tool_directive() -> bool {
    let Some(go) = find_program("go", None) else { return false };
    let Ok(output) = std::process::Command::new(go).args(["env", "GOVERSION"]).output() else {
        return false;
    };
    let version = String::from_utf8_lossy(&output.stdout);
    let mut parts = version.trim().trim_start_matches("go").split(['.', 'r', 'b']);
    let major: u32 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    (major, minor) >= (1, 24)
}

/// The ways `program` can be installed for the project at `project`,
/// the one that suits the project first. Empty when the catalog does
/// not know the program or this machine has none of its installers.
pub fn methods(program: &str, project: Option<&Path>, host: &Host) -> Vec<Method> {
    let Some(recipe) = recipe(program) else { return Vec::new() };
    let mut project_first = false;
    let mut out = Vec::new();
    for source in recipe.sources {
        let (methods, suits) = source_methods(*source, project, host);
        project_first |= suits;
        for method in methods {
            if !out.iter().any(|known: &Method| known.command == method.command) {
                out.push(method);
            }
        }
    }
    // Stable, so each group keeps the catalog's order. A project that
    // already keeps its tools in its environment gets the next one
    // there too; one that does not is not made to start.
    out.sort_by_key(|method| (method.scope == Scope::Project) != project_first);
    out
}

/// Where the program comes from when nothing can be run for it, in
/// words, or the commands a person would run when this machine has
/// none of its installers.
pub fn notes(program: &str) -> Vec<String> {
    let Some(recipe) = recipe(program) else { return Vec::new() };
    recipe
        .sources
        .iter()
        .filter_map(|source| match source {
            Note(text) => Some(tr(text)),
            _ => None,
        })
        .collect()
}

/// The sentence that says how `program` is installed, for the places
/// that only describe it: the commands this machine can run, the first
/// with the rest as alternatives; or, with none of its installers
/// here, the canonical ones.
pub fn hint(program: &str) -> Option<String> {
    let recipe = recipe(program)?;
    let host = Host::probe(None);
    let mut commands: Vec<String> =
        methods(program, None, &host).into_iter().map(|method| method.command).collect();
    if commands.is_empty() {
        // pip and venv are abilities of python3 rather than programs on
        // PATH, so they are not in INSTALLERS; without them a Python
        // module such as debugpy would have no command to suggest.
        let everything = Host {
            installers: INSTALLERS.iter().chain(&["pip", "venv"]).map(|name| name.to_string()).collect(),
            go_tools: false,
            macos: host.macos,
            home: host.home.clone(),
        };
        commands = recipe
            .sources
            .iter()
            .filter(|source| !matches!(source, System(_)) || !host.macos)
            .flat_map(|source| source_methods(*source, None, &everything).0)
            .filter(|method| method.scope != Scope::System || method.installer == "apt-get")
            .map(|method| method.command)
            .take(2)
            .collect();
    }
    let notes = notes(program);
    match (commands.split_first(), notes.first()) {
        (Some((first, [])), _) => Some(first.clone()),
        (Some((first, rest)), _) => Some(format!("{first} (or: {})", rest.join("; "))),
        (None, Some(note)) => Some(note.clone()),
        (None, None) => None,
    }
}

fn method(
    installer: &'static str,
    scope: Scope,
    title: String,
    command: String,
    directory: &Path,
) -> Method {
    Method {
        installer,
        scope,
        title,
        command,
        directory: directory.to_owned(),
        runs: scope != Scope::System,
    }
}

/// A project's virtual environment, when it has one.
fn venv(project: &Path) -> Option<PathBuf> {
    let venv = project.join(".venv");
    venv.join("bin").join("python").exists().then_some(venv)
}

fn is_python_project(project: &Path) -> bool {
    ["pyproject.toml", "setup.py", "setup.cfg", "requirements.txt", "uv.lock", ".venv"]
        .iter()
        .any(|marker| project.join(marker).exists())
}

/// The methods one source gives, and whether the project's own should
/// come first: it already keeps its tools in its environment.
fn source_methods(source: Source, project: Option<&Path>, host: &Host) -> (Vec<Method>, bool) {
    let home = host.home.as_path();
    let user = |installer, title: String, command: String| {
        method(installer, Scope::User, title, command, home)
    };
    let brew_title = || tr("With Homebrew");
    match source {
        Python(package) | PythonModule(package) => {
            let module = matches!(source, PythonModule(_));
            let mut out = Vec::new();
            let mut suits = false;
            if let Some(project) = project {
                suits = project.join("uv.lock").exists() || venv(project).is_some();
                if host.has("uv") && project.join("pyproject.toml").exists() {
                    out.push(method(
                        "uv",
                        Scope::Project,
                        tr("Into this project, recorded in pyproject.toml"),
                        format!("uv add --dev {package}"),
                        project,
                    ));
                }
                if venv(project).is_some() {
                    out.push(if host.has("uv") {
                        method(
                            "uv",
                            Scope::Project,
                            tr("Into this project's .venv, not recorded"),
                            format!("uv pip install --python .venv {package}"),
                            project,
                        )
                    } else {
                        method(
                            "python3",
                            Scope::Project,
                            tr("Into this project's .venv, not recorded"),
                            format!(".venv/bin/python -m pip install {package}"),
                            project,
                        )
                    });
                } else if is_python_project(project) || module {
                    if host.has("uv") && !project.join("pyproject.toml").exists() {
                        out.push(method(
                            "uv",
                            Scope::Project,
                            tr("Into a new .venv for this project"),
                            format!("uv venv && uv pip install --python .venv {package}"),
                            project,
                        ));
                    } else if !host.has("uv") && host.has("venv") {
                        out.push(method(
                            "python3",
                            Scope::Project,
                            tr("Into a new .venv for this project"),
                            format!("python3 -m venv .venv && .venv/bin/python -m pip install {package}"),
                            project,
                        ));
                    }
                }
                // A module is imported by the project's Python; that is
                // where it has to be.
                suits |= module;
            }
            if !module && host.has("uv") {
                out.push(user(
                    "uv",
                    tr("For your user, in an environment of its own"),
                    format!("uv tool install {package}"),
                ));
            }
            if !module && host.has("pipx") {
                out.push(user(
                    "pipx",
                    tr("For your user, in an environment of its own"),
                    format!("pipx install {package}"),
                ));
            }
            if (module || !(host.has("uv") || host.has("pipx"))) && host.has("pip") {
                out.push(user(
                    "python3",
                    tr("For your user, with pip"),
                    format!("python3 -m pip install --user {package}"),
                ));
            }
            (out, suits)
        }
        Npm(packages) => {
            if !host.has("npm") {
                return (Vec::new(), false);
            }
            let mut out = Vec::new();
            let mut suits = false;
            if let Some(project) = project.filter(|project| project.join("package.json").exists()) {
                suits = project.join("node_modules").join(".bin").is_dir();
                out.push(method(
                    "npm",
                    Scope::Project,
                    tr("Into this project, recorded in package.json"),
                    format!("npm install --save-dev {packages}"),
                    project,
                ));
            }
            out.push(user("npm", tr("For your user, with npm"), format!("npm install -g {packages}")));
            (out, suits)
        }
        Go(module) => {
            if !host.has("go") {
                return (Vec::new(), false);
            }
            let mut out = Vec::new();
            let mut suits = false;
            if let Some(module_root) = project.and_then(go_module_root) {
                suits = !go_tools(&module_root).is_empty();
                if host.go_tools {
                    out.push(method(
                        "go",
                        Scope::Project,
                        tr("Into this module, recorded in go.mod"),
                        format!("go get -tool {module}@latest"),
                        &module_root,
                    ));
                }
            }
            out.push(user("go", tr("For your user, with go install"), format!("go install {module}@latest")));
            (out, suits)
        }
        Rustup(component) => (
            if host.has("rustup") {
                vec![user("rustup", tr("Into the Rust toolchain"), format!("rustup component add {component}"))]
            } else {
                Vec::new()
            },
            false,
        ),
        Cargo(spec) => (
            if host.has("cargo") {
                vec![user("cargo", tr("For your user, with cargo"), format!("cargo install {spec}"))]
            } else {
                Vec::new()
            },
            false,
        ),
        Brew(formula) => (
            if host.has("brew") {
                vec![user("brew", brew_title(), format!("brew install {formula}"))]
            } else {
                Vec::new()
            },
            false,
        ),
        Gem(gem) => simple(host, "gem", tr("For your user, with gem"), format!("gem install {gem}"), home),
        Dotnet(tool) => simple(
            host,
            "dotnet",
            tr("For your user, with dotnet"),
            format!("dotnet tool install --global {tool}"),
            home,
        ),
        Opam(package) => simple(host, "opam", tr("Into the opam switch"), format!("opam install -y {package}"), home),
        Cabal(package) => simple(host, "cabal", tr("For your user, with cabal"), format!("cabal install {package}"), home),
        Ghcup(tool) => simple(host, "ghcup", tr("For your user, with ghcup"), format!("ghcup install {tool}"), home),
        Composer(package) => simple(
            host,
            "composer",
            tr("For your user, with composer"),
            format!("composer global require {package}"),
            home,
        ),
        Nix(package) => simple(
            host,
            "nix",
            tr("Into your nix profile"),
            format!("nix profile install nixpkgs#{package}"),
            home,
        ),
        Coursier(application) => simple(host, "cs", tr("For your user, with coursier"), format!("cs install {application}"), home),
        System(package) => {
            if host.macos {
                return (Vec::new(), false);
            }
            let title = tr("With the system's packages (needs an administrator)");
            let line = if host.has("apt-get") {
                Some(("apt-get", format!("sudo apt install {package}")))
            } else if host.has("dnf") {
                Some(("dnf", format!("sudo dnf install {package}")))
            } else if host.has("pacman") {
                Some(("pacman", format!("sudo pacman -S {package}")))
            } else if host.has("zypper") {
                Some(("zypper", format!("sudo zypper install {package}")))
            } else {
                None
            };
            (
                line.map(|(installer, command)| method(installer, Scope::System, title, command, home))
                    .into_iter()
                    .collect(),
                false,
            )
        }
        XcodeTools => (
            if host.macos {
                vec![user(
                    "xcode-select",
                    tr("With the command-line developer tools"),
                    "xcode-select --install".to_owned(),
                )]
            } else {
                Vec::new()
            },
            false,
        ),
        Note(_) => (Vec::new(), false),
    }
}

fn simple(host: &Host, installer: &'static str, title: String, command: String, home: &Path) -> (Vec<Method>, bool) {
    if host.has(installer) {
        (vec![method(installer, Scope::User, title, command, home)], false)
    } else {
        (Vec::new(), false)
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// The nearest folder at or above `start` with a go.mod.
fn go_module_root(start: &Path) -> Option<PathBuf> {
    start.ancestors().find(|dir| dir.join("go.mod").is_file()).map(Path::to_path_buf)
}

/// The tool packages a go.mod lists: `tool path` lines, and the lines of
/// a `tool ( … )` block.
fn go_tools(module_root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(module_root.join("go.mod")) else {
        return Vec::new();
    };
    let mut tools = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if in_block {
            if line == ")" {
                in_block = false;
            } else if !line.is_empty() {
                tools.push(line.to_owned());
            }
        } else if line == "tool (" || line == "tool(" {
            in_block = true;
        } else if let Some(path) = line.strip_prefix("tool ") {
            tools.push(path.trim().to_owned());
        }
    }
    tools
}

/// The folders a project's own installs land in, nearest first: each
/// folder from `project` up to the repository's top (or home) that has
/// a `.venv/bin` or a `node_modules/.bin`. A workspace keeps one
/// environment at its top for every member, so the walk goes up.
pub fn environment_folders(project: &Path) -> Vec<PathBuf> {
    let home = home();
    let mut out = Vec::new();
    for dir in project.ancestors() {
        if dir == home || dir.parent().is_none() {
            break;
        }
        for bin in [dir.join(".venv").join("bin"), dir.join("node_modules").join(".bin")] {
            if bin.is_dir() {
                out.push(bin);
            }
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    out
}

/// The per-user folders installers write programs to, which a desktop
/// session's `PATH` often does not have.
pub fn user_folders() -> Vec<PathBuf> {
    let home = home();
    let mut out = Vec::new();
    if let Some(bin) = std::env::var_os("GOBIN").filter(|bin| !bin.is_empty()) {
        out.push(PathBuf::from(bin));
    }
    if let Some(gopath) = std::env::var_os("GOPATH") {
        out.extend(std::env::split_paths(&gopath).map(|dir| dir.join("bin")));
    }
    for relative in ["go/bin", ".local/bin", ".cargo/bin", ".dotnet/tools", ".ghcup/bin", ".cabal/bin"] {
        out.push(home.join(relative));
    }
    // Homebrew keeps its LLVM out of the PATH it manages, so a clangd or
    // lldb-dap installed through it is found only here.
    if cfg!(target_os = "macos") {
        out.push(PathBuf::from("/opt/homebrew/opt/llvm/bin"));
        out.push(PathBuf::from("/usr/local/opt/llvm/bin"));
    }
    out
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else { return false };
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

/// Where `program` is, looked for as [`resolve`] does but without the
/// `go tool` route.
fn find_program(program: &str, project: Option<&Path>) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return is_executable(path).then(|| path.to_owned());
    }
    let mut folders = project.map(environment_folders).unwrap_or_default();
    if let Some(paths) = std::env::var_os("PATH") {
        folders.extend(std::env::split_paths(&paths));
    }
    folders.extend(user_folders());
    folders.into_iter().map(|dir| dir.join(program)).find(|candidate| is_executable(candidate))
}

/// The command line that runs `program` for the project at `project`,
/// program first: from the project's environment, then `PATH`, then the
/// per-user folders, then — for a Go module that lists it as a tool —
/// through `go tool`. None when it is nowhere.
pub fn resolve(program: &str, project: Option<&Path>) -> Option<Vec<String>> {
    if let Some(found) = find_program(program, project) {
        return Some(vec![found.to_string_lossy().into_owned()]);
    }
    let module_root = project.and_then(go_module_root)?;
    let listed = go_tools(&module_root)
        .into_iter()
        .any(|tool| tool.rsplit('/').next() == Some(program));
    if !listed {
        return None;
    }
    let go = find_program("go", Some(&module_root))?;
    Some(vec![go.to_string_lossy().into_owned(), "tool".to_owned(), program.to_owned()])
}

/// `command`'s first word resolved as [`resolve`] does, the rest kept:
/// what to run for a configured command line. The line as it was when
/// the program is nowhere, for the error to name.
pub fn resolve_command(words: &[String], project: Option<&Path>) -> Vec<String> {
    let Some((program, rest)) = words.split_first() else { return Vec::new() };
    match resolve(program, project) {
        Some(mut line) => {
            line.extend(rest.iter().cloned());
            line
        }
        None => words.to_vec(),
    }
}

/// The environment a program run for `project` gets on top of the
/// editor's own: the project's environment folders ahead of `PATH` and
/// the per-user folders after it, the way activating the environment
/// would, and `VIRTUAL_ENV` when there is one. A Python language server
/// then sees the project's interpreter and its packages.
pub fn environment(project: &Path) -> Vec<(String, String)> {
    let folders = environment_folders(project);
    let mut paths: Vec<PathBuf> = folders.clone();
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    for folder in user_folders() {
        if folder.is_dir() && !paths.contains(&folder) {
            paths.push(folder);
        }
    }
    let mut out = Vec::new();
    if let Ok(joined) = std::env::join_paths(paths) {
        out.push(("PATH".to_owned(), joined.to_string_lossy().into_owned()));
    }
    if let Some(venv) = folders
        .iter()
        .find(|folder| folder.ends_with(".venv/bin"))
        .and_then(|folder| folder.parent())
    {
        out.push(("VIRTUAL_ENV".to_owned(), venv.to_string_lossy().into_owned()));
    }
    out
}

/// What was found missing this session: a program something needed, the
/// project it was needed for, and what for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub program: String,
    pub project: PathBuf,
    /// What it is, in words: "python language server", "save formatter".
    pub purpose: String,
}

static MISSING: Mutex<Vec<Missing>> = Mutex::new(Vec::new());

/// Records that `program` was needed for `project` and is not there.
/// Language servers, debug adapters and save preprocessors all report
/// here, so one place can offer to install what any of them lacked.
pub fn note_missing(program: &str, project: &Path, purpose: &str) {
    let Ok(mut missing) = MISSING.lock() else { return };
    if missing.iter().any(|entry| entry.program == program && entry.project == project) {
        return;
    }
    missing.push(Missing {
        program: program.to_owned(),
        project: project.to_owned(),
        purpose: purpose.to_owned(),
    });
}

/// Forgets `program` everywhere: it was installed.
pub fn forget(program: &str) {
    if let Ok(mut missing) = MISSING.lock() {
        missing.retain(|entry| entry.program != program);
    }
}

/// What is missing for the document at `path`: entries for its project
/// or a project around it, the ones found since dropped. A module's
/// absence cannot be seen without running Python, so it stays until it
/// is installed from here.
pub fn missing_for(path: &Path) -> Vec<Missing> {
    let Ok(mut missing) = MISSING.lock() else { return Vec::new() };
    missing.retain(|entry| is_module(&entry.program) || resolve(&entry.program, Some(&entry.project)).is_none());
    missing.iter().filter(|entry| path.starts_with(&entry.project)).cloned().collect()
}

/// What is missing for the document at `path`, with how each can be
/// installed, as JSON for the shells: `[{program, purpose, project,
/// notes, methods: [{installer, scope, title, command, directory,
/// runs}]}]`.
pub fn offers_json(path: &Path) -> String {
    let entries: Vec<Value> = missing_for(path)
        .into_iter()
        .map(|entry| {
            let host = Host::probe(Some(&entry.project));
            offer_value(&entry.program, &entry.purpose, &entry.project, &host)
        })
        .collect();
    Value::Array(entries).to_string()
}

/// How `program` can be installed for `project`, as one entry of
/// [`offers_json`].
pub fn offer_json(program: &str, project: &Path, purpose: &str) -> String {
    offer_value(program, purpose, project, &Host::probe(Some(project))).to_string()
}

/// The installers any of which would let the editor install `program`
/// here, for when none of them is.
fn wanted_installers(program: &str, macos: bool) -> Vec<&'static str> {
    let Some(recipe) = recipe(program) else { return Vec::new() };
    let mut out: Vec<&'static str> = Vec::new();
    for source in recipe.sources {
        let wanted: &[&'static str] = match source {
            Python(_) => &["uv", "pipx"],
            PythonModule(_) => &["uv", "python3-venv"],
            Npm(_) => &["npm"],
            Go(_) => &["go"],
            Rustup(_) => &["rustup"],
            Cargo(_) => &["cargo"],
            Gem(_) => &["gem"],
            Brew(_) => &["brew"],
            Dotnet(_) => &["dotnet"],
            Opam(_) => &["opam"],
            Cabal(_) => &["cabal"],
            Ghcup(_) => &["ghcup"],
            Composer(_) => &["composer"],
            Nix(_) => &["nix"],
            Coursier(_) => &["cs"],
            XcodeTools if macos => &["xcode-select"],
            XcodeTools | System(_) | Note(_) => &[],
        };
        for name in wanted {
            if !out.contains(name) {
                out.push(name);
            }
        }
    }
    out
}

fn offer_value(program: &str, purpose: &str, project: &Path, host: &Host) -> Value {
    let methods: Vec<Value> = methods(program, Some(project), host)
        .into_iter()
        .map(|method| {
            json!({
                "installer": method.installer,
                "scope": method.scope.name(),
                "title": method.title,
                "command": method.command,
                "directory": method.directory.to_string_lossy(),
                "runs": method.runs,
            })
        })
        .collect();
    let mut notes = notes(program);
    if methods.is_empty() {
        let wanted = wanted_installers(program, host.macos);
        if !wanted.is_empty() {
            notes.push(fill(
                &tr("Nothing here can install it; any of these would let the editor do it: {}"),
                &[&wanted.join(", ")],
            ));
        }
    }
    json!({
        "program": program,
        "purpose": purpose,
        "project": project.to_string_lossy(),
        "notes": notes,
        "methods": methods,
    })
}

/// The sentence for a program that is not there: what it is, and that
/// it can be installed from here — or, when nothing here can install
/// it, how it is installed.
pub fn not_installed_message(program: &str, project: Option<&Path>) -> String {
    let host = Host::probe(project);
    if !methods(program, project, &host).is_empty() {
        return fill(&tr("{} is not installed; Install… offers to install it"), &[program]);
    }
    match hint(program) {
        Some(hint) => fill(&tr("{} is not installed (install with: {})"), &[program, &hint]),
        None => fill(&tr("{} is not installed"), &[program]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(installers: &[&str]) -> Host {
        Host {
            installers: installers.iter().map(|name| name.to_string()).collect(),
            go_tools: true,
            macos: false,
            home: PathBuf::from("/home/someone"),
        }
    }

    fn commands(methods: &[Method]) -> Vec<&str> {
        methods.iter().map(|method| method.command.as_str()).collect()
    }

    /// A folder for one test, gone when the test is.
    struct Scratch(PathBuf);

    impl Scratch {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn project(files: &[&str]) -> Scratch {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "textchum-install-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    #[test]
    fn a_python_tool_goes_to_the_user_unless_the_project_keeps_its_own() {
        let loose = project(&["pyproject.toml"]);
        let found = methods("ruff", Some(loose.path()), &host(&["uv", "pipx", "python3", "pip"]));
        assert_eq!(
            commands(&found),
            ["uv tool install ruff", "pipx install ruff", "uv add --dev ruff"]
        );

        let locked = project(&["pyproject.toml", "uv.lock", ".venv/bin/python"]);
        let found = methods("ruff", Some(locked.path()), &host(&["uv", "python3"]));
        assert_eq!(
            commands(&found),
            ["uv add --dev ruff", "uv pip install --python .venv ruff", "uv tool install ruff"]
        );
        assert_eq!(found[0].directory, locked.path());
        assert_eq!(found[0].scope, Scope::Project);
        assert_eq!(found[2].directory, PathBuf::from("/home/someone"));
    }

    #[test]
    fn without_uv_a_project_gets_a_venv_and_pip() {
        let plain = project(&["requirements.txt"]);
        let found = methods("black", Some(plain.path()), &host(&["python3", "pip", "venv"]));
        assert_eq!(
            commands(&found),
            [
                "python3 -m pip install --user black",
                "python3 -m venv .venv && .venv/bin/python -m pip install black",
            ]
        );
        // A Python the system manages refuses pip for the user, so only
        // the project's own environment is offered.
        let managed = methods("black", Some(plain.path()), &host(&["python3", "venv"]));
        assert_eq!(
            commands(&managed),
            ["python3 -m venv .venv && .venv/bin/python -m pip install black"]
        );
        // Without ensurepip there is no making one either: nothing runs,
        // and the offer says what would let it.
        assert!(methods("black", Some(plain.path()), &host(&["python3"])).is_empty());
        let offer = offer_value("black", "a test", plain.path(), &host(&["python3"]));
        assert!(offer["notes"][0].as_str().unwrap().contains("uv, pipx"), "{offer}");
    }

    #[test]
    fn a_module_goes_where_the_project_imports_from() {
        let script = project(&["main.py"]);
        let found = methods("debugpy", Some(script.path()), &host(&["uv", "pipx", "python3", "pip"]));
        assert_eq!(
            commands(&found),
            [
                "uv venv && uv pip install --python .venv debugpy",
                "python3 -m pip install --user debugpy",
            ]
        );
        assert!(is_module("debugpy"));
        assert!(!is_module("ruff"));
    }

    #[test]
    fn go_tools_go_to_the_module_when_it_keeps_tools() {
        let module = project(&["go.mod"]);
        let found = methods("gopls", Some(module.path()), &host(&["go", "brew"]));
        assert_eq!(
            commands(&found),
            [
                "go install golang.org/x/tools/gopls@latest",
                "brew install gopls",
                "go get -tool golang.org/x/tools/gopls@latest",
            ]
        );
        std::fs::write(module.path().join("go.mod"), "module x\n\ntool golang.org/x/tools/cmd/stringer\n").unwrap();
        let found = methods("gopls", Some(module.path()), &host(&["go"]));
        assert_eq!(found[0].scope, Scope::Project);
        let older = Host { go_tools: false, ..host(&["go"]) };
        assert_eq!(
            commands(&methods("gopls", Some(module.path()), &older)),
            ["go install golang.org/x/tools/gopls@latest"]
        );
    }

    #[test]
    fn a_go_tool_is_run_through_go_tool() {
        let module = project(&[]);
        std::fs::write(
            module.path().join("go.mod"),
            "module x\n\ntool (\n\tgolang.org/x/tools/gopls // the server\n\tgithub.com/go-delve/delve/cmd/dlv\n)\n",
        )
        .unwrap();
        assert_eq!(
            go_tools(module.path()),
            ["golang.org/x/tools/gopls", "github.com/go-delve/delve/cmd/dlv"]
        );
        let nested = module.path().join("cmd/app");
        std::fs::create_dir_all(&nested).unwrap();
        if find_program("go", None).is_some() && find_program("dlv", None).is_none() {
            let line = resolve("dlv", Some(&nested)).unwrap();
            assert_eq!(&line[1..], ["tool", "dlv"]);
        }
    }

    #[test]
    fn a_project_install_is_found_before_the_path() {
        let dir = project(&[]);
        let bin = dir.path().join(".venv/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let program = bin.join("textchum-test-tool");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let member = dir.path().join("packages/member");
        std::fs::create_dir_all(&member).unwrap();
        assert_eq!(
            resolve("textchum-test-tool", Some(&member)),
            Some(vec![program.to_string_lossy().into_owned()])
        );
        assert_eq!(resolve("textchum-test-tool", None), None);
        let environment = environment(&member);
        let path = &environment.iter().find(|(key, _)| key == "PATH").unwrap().1;
        assert!(path.starts_with(&*bin.to_string_lossy()));
        assert!(environment.iter().any(|(key, value)| key == "VIRTUAL_ENV" && value.ends_with(".venv")));
    }

    #[test]
    fn system_packages_are_shown_not_run_and_only_off_macos() {
        let found = methods("clangd", None, &host(&["apt-get", "brew"]));
        assert_eq!(commands(&found), ["brew install llvm", "sudo apt install clangd"]);
        assert!(found[0].runs);
        assert!(!found[1].runs);
        let mac = Host { macos: true, ..host(&["apt-get"]) };
        assert_eq!(commands(&methods_for(&mac)), ["xcode-select --install"]);

        fn methods_for(host: &Host) -> Vec<Method> {
            super::methods("clangd", None, host)
        }
    }

    #[test]
    fn npm_offers_the_project_when_it_has_a_package_json() {
        let app = project(&["package.json"]);
        let found = methods("prettier", Some(app.path()), &host(&["npm"]));
        assert_eq!(commands(&found), ["npm install -g prettier", "npm install --save-dev prettier"]);
        std::fs::create_dir_all(app.path().join("node_modules/.bin")).unwrap();
        let found = methods("prettier", Some(app.path()), &host(&["npm"]));
        assert_eq!(found[0].command, "npm install --save-dev prettier");
    }

    #[test]
    fn nothing_to_run_leaves_notes_and_the_canonical_hint() {
        assert!(methods("prettier", None, &host(&[])).is_empty());
        assert_eq!(notes("gofmt"), ["comes with Go"]);
        assert!(methods("not-a-program", None, &host(&["uv"])).is_empty());
        assert!(hint("not-a-program").is_none());
        // Whatever this machine has, a known program has a hint.
        for recipe in RECIPES {
            assert!(hint(recipe.program).is_some(), "{} has no hint", recipe.program);
        }
    }

    #[test]
    fn missing_programs_are_listed_for_their_project_until_found() {
        let dir = project(&[]);
        note_missing("textchum-surely-absent", dir.path(), "a test");
        note_missing("textchum-surely-absent", dir.path(), "a test");
        let inside = dir.path().join("src/main.rs");
        let listed = missing_for(&inside);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].purpose, "a test");
        assert!(missing_for(Path::new("/elsewhere/file")).iter().all(|entry| entry.program != "textchum-surely-absent"));
        forget("textchum-surely-absent");
        assert!(missing_for(&inside).is_empty());
    }
}
