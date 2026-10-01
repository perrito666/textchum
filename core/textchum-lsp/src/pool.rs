//! The instance pool: one language-server process per (server, project).
//!
//! This is the project's defining behavior. Instances are keyed by
//! `(server id, project root)`: opening files from two different Python
//! projects yields two independent server processes, each initialized
//! with its own root, each seeing only its own project's documents. The
//! root comes from the workspace model (nearest root marker); files
//! outside any project get a per-directory instance, so loose files
//! never leak into someone else's workspace.
//!
//! A server binary that is missing or fails to start is reported once per
//! (server, root) through the event channel — with its install hint — and
//! not retried until the application restarts.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use textchum_core::workspace::{self, WorkspaceSettings};
use textchum_core::{Event, EventSender};

use crate::instance::{Command, Instance};
use crate::registry::{server_for_language, ServerSpec};

/// Owned server description; built from the registry or supplied directly
/// (tests use this to point at a scripted server).
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub languages: Vec<String>,
    pub install_hint: String,
}

impl From<&ServerSpec> for ServerConfig {
    fn from(spec: &ServerSpec) -> Self {
        Self {
            id: spec.id.into(),
            command: spec.command.into(),
            args: spec.args.iter().map(|s| s.to_string()).collect(),
            languages: spec.languages.iter().map(|s| s.to_string()).collect(),
            install_hint: spec.install_hint.into(),
        }
    }
}

type InstanceKey = (String, PathBuf);

pub struct Pool {
    events: EventSender,
    instances: HashMap<InstanceKey, Instance>,
    /// Which instances each open document talks to, in the order the
    /// configuration names them. A request goes to the first that
    /// provides what is asked; notifications go to all.
    documents: HashMap<PathBuf, Vec<InstanceKey>>,
    /// Which instance last offered code actions for a document, since
    /// resolving one or running its command is that server's business.
    acting: HashMap<PathBuf, InstanceKey>,
    /// Per-document LSP version counter.
    versions: HashMap<PathBuf, i64>,
    /// (server, root) pairs that failed to start; not retried.
    failed: HashSet<InstanceKey>,
    /// Test/override servers, consulted before the built-in registry.
    overrides: Vec<ServerConfig>,
    /// User configuration: `{"defaults": {lang: cmdline}, "projects":
    /// {root: {lang: cmdline}}}`, consulted before everything else.
    configured: serde_json::Value,
    /// Workspace behavior (manifest-project and recursive-config flags).
    workspace_settings: WorkspaceSettings,
    /// When each instance last did anything, for the idle sweep.
    last_activity: HashMap<InstanceKey, std::time::Instant>,
    /// What the servers last published about each file, as they
    /// published it — see [`crate::instance::PublishedDiagnostics`].
    published: crate::instance::PublishedDiagnostics,
    /// Next client→server request id. Starts above the lifecycle ids
    /// (1 = initialize, 2 = shutdown).
    next_request_id: u64,
}

impl Pool {
    pub fn new(events: EventSender) -> Self {
        Self {
            events,
            instances: HashMap::new(),
            documents: HashMap::new(),
            acting: HashMap::new(),
            versions: HashMap::new(),
            failed: HashSet::new(),
            overrides: Vec::new(),
            configured: serde_json::Value::Null,
            workspace_settings: WorkspaceSettings::default(),
            last_activity: HashMap::new(),
            published: Default::default(),
            next_request_id: 100,
        }
    }

    /// Applies the user's configuration. Accepts either the `lsp` section
    /// alone (`{"defaults": …, "projects": …}`) or a combined
    /// `{"lsp": …, "workspace": …}` object carrying the workspace flags
    /// too. Takes effect for instances spawned afterwards; call
    /// [`Self::shutdown_all`] to also retire running ones. Also clears
    /// the not-retried failure memory, since a fixed command deserves a
    /// fresh chance.
    pub fn configure(&mut self, json: &str) {
        let parsed: serde_json::Value =
            serde_json::from_str(json).unwrap_or(serde_json::Value::Null);
        if parsed.get("lsp").is_some() || parsed.get("workspace").is_some() {
            self.workspace_settings = WorkspaceSettings::from_json(
                &parsed.get("workspace").map(|v| v.to_string()).unwrap_or_default(),
            );
            self.configured = parsed.get("lsp").cloned().unwrap_or(serde_json::Value::Null);
        } else {
            self.workspace_settings = WorkspaceSettings::default();
            self.configured = parsed;
        }
        self.failed.clear();
        let keys = |section: &str| -> Vec<String> {
            self.configured[section]
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default()
        };
        crate::log::log(&format!(
            "configure: default languages {:?}, project entries {:?}",
            keys("defaults"),
            keys("projects"),
        ));
    }

    /// Shuts down every running instance and forgets open-document
    /// routing; the shell re-announces documents to respawn under the
    /// current configuration.
    pub fn shutdown_all(&mut self) {
        self.instances.clear();
        self.documents.clear();
        self.acting.clear();
        if let Ok(mut published) = self.published.lock() {
            published.clear();
        }
        self.versions.clear();
        self.last_activity.clear();
    }

    /// Forgets one instance (and its document routing) — the cleanup
    /// half of crash recovery. The shell re-announces the affected
    /// documents afterwards to spawn a replacement; the crash memory is
    /// intentionally not marked "failed", because a crash is not a
    /// missing binary.
    pub fn retire(&mut self, server: &str, root: &str) {
        let key = (server.to_owned(), PathBuf::from(root));
        if self.instances.remove(&key).is_some() {
            crate::log::log(&format!("retired {server} at {root}"));
        }
        self.last_activity.remove(&key);
        self.acting.retain(|_, acting| *acting != key);
        let mut orphaned = Vec::new();
        self.documents.retain(|path, routed| {
            if !routed.contains(&key) {
                return true;
            }
            routed.retain(|other| *other != key);
            orphaned.push(path.clone());
            if routed.is_empty() {
                self.versions.remove(path);
                false
            } else {
                true
            }
        });
        // What the server said about those files went with it; the
        // shells are told what is left, or its marks would stay up
        // with nobody behind them.
        if let Ok(mut published) = self.published.lock() {
            for path in orphaned {
                if published.remove(&(key.0.clone(), path.clone())).is_some() {
                    let _ = self.events.send(crate::instance::findings_event(&published, &path));
                }
            }
        }
    }

    /// Instances no document has needed for a while are shut down; the
    /// next open starts a fresh one. Swept lazily from open/close, which
    /// is exactly when the population changes.
    const IDLE_SHUTDOWN: std::time::Duration = std::time::Duration::from_secs(300);

    fn sweep_idle(&mut self) {
        let now = std::time::Instant::now();
        let in_use: HashSet<&InstanceKey> = self.documents.values().flatten().collect();
        let idle: Vec<InstanceKey> = self
            .instances
            .keys()
            .filter(|key| !in_use.contains(key))
            .filter(|key| {
                self.last_activity
                    .get(*key)
                    .map(|last| now.duration_since(*last) > Self::IDLE_SHUTDOWN)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        for key in idle {
            crate::log::log(&format!(
                "idle shutdown: {} at {} (no open documents)",
                key.0,
                key.1.display()
            ));
            self.instances.remove(&key);
            self.last_activity.remove(&key);
        }
    }

    fn touch(&mut self, key: &InstanceKey) {
        self.last_activity
            .insert(key.clone(), std::time::Instant::now());
    }

    /// A user-configured command line for (root, language): the exact
    /// project entry, else an ancestor's project entry when that ancestor
    /// opted into recursive configuration (for nested projects), else the
    /// defaults entry.
    fn configured_command(&self, root: &Path, language: &str) -> Option<String> {
        let project_entry = |dir: &Path| -> Option<String> {
            self.configured["projects"][dir.to_string_lossy().as_ref()][language]
                .as_str()
                .map(str::to_owned)
        };
        let mut value = project_entry(root);
        if value.is_none() {
            let mut ancestor = root.parent();
            while let Some(dir) = ancestor {
                if self.workspace_settings.recursive_config(dir) {
                    if let Some(found) = project_entry(dir) {
                        value = Some(found);
                        break;
                    }
                }
                ancestor = dir.parent();
            }
        }
        let value =
            value.or_else(|| self.configured["defaults"][language].as_str().map(str::to_owned))?;
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    /// The server with this id, from configuration first and the
    /// built-in registry second.
    ///
    /// `lsp.servers` holds entries of the same shape as the registry's,
    /// so a server the build does not know about can be defined without
    /// a code change, and one it does know about can be redefined by
    /// reusing its id:
    ///
    /// ```json
    /// {"lsp": {"servers": {"basedpyright": {
    ///    "command": "{project}/.venv/bin/basedpyright-langserver",
    ///    "args": ["--stdio"],
    ///    "languages": ["python"],
    ///    "install": "uv tool install basedpyright"}}}}
    /// ```
    ///
    /// The built-in table stays available, so a configuration that says
    /// nothing still has servers, and a build that learns a new one
    /// offers it without the configuration being rewritten.
    fn server_spec(&self, id: &str) -> Option<ServerConfig> {
        if let Some(entry) = self.configured["servers"][id].as_object() {
            let command = entry.get("command")?.as_str()?.to_owned();
            let args = entry
                .get("args")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()
                })
                .unwrap_or_default();
            let languages = entry
                .get("languages")
                .and_then(|l| l.as_array())
                .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            let install_hint = entry
                .get("install")
                .and_then(|v| v.as_str())
                .unwrap_or("configured in config.json under lsp.servers")
                .to_owned();
            return Some(ServerConfig {
                id: id.to_owned(),
                command,
                args,
                languages,
                install_hint,
            });
        }
        crate::registry::server_by_id(id).map(ServerConfig::from)
    }

    /// What the configuration gives a server to run with:
    /// `lsp.settings.<id>` and `lsp.init_options.<id>`.
    ///
    /// ```json
    /// {"lsp": {"settings": {
    ///    "rust-analyzer": {"rust-analyzer": {"check": {"command": "clippy"}}},
    ///    "gopls": {"gopls": {"staticcheck": true}}}}}
    /// ```
    ///
    /// A command line typed by hand has no id of its own, so its
    /// settings are found under the name of the command it runs.
    fn options_for(&self, config: &ServerConfig) -> crate::instance::ServerOptions {
        let key = match config.id.strip_prefix("custom:") {
            Some(command) => Path::new(command)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| command.to_owned()),
            None => config.id.clone(),
        };
        crate::instance::ServerOptions {
            settings: self.configured["settings"][&key].clone(),
            init_options: self.configured["init_options"][&key].clone(),
        }
    }

    /// The ids a language has servers for, configuration first.
    pub fn servers_for_language(&self, language: &str) -> Vec<String> {
        let mut ids: Vec<String> = self.configured["servers"]
            .as_object()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|(_, entry)| {
                        entry["languages"]
                            .as_array()
                            .map(|l| l.iter().any(|v| v.as_str() == Some(language)))
                            .unwrap_or(false)
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();
        for spec in crate::registry::servers_for_language(language) {
            if !ids.iter().any(|id| id == spec.id) {
                ids.push(spec.id.to_owned());
            }
        }
        ids
    }

    /// Expands the placeholders a configured command line may use.
    ///
    /// `{project}` is the root the instance is keyed on, so a server
    /// kept inside a checkout can be named without an absolute path
    /// that only works on one machine:
    ///
    /// ```text
    /// {project}/.venv/bin/basedpyright-langserver --stdio
    /// ```
    ///
    /// `{home}` is the user's home directory, for tooling installed per
    /// user and shared configuration that has to find it.
    ///
    /// Expansion happens per argument, after the command line is split,
    /// so a project path containing spaces stays one argument.
    fn expand_placeholders(value: &str, root: &Path) -> String {
        let mut out = value.replace("{project}", &root.to_string_lossy());
        if out.contains("{home}") {
            let home = std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_default();
            out = out.replace("{home}", &home.to_string_lossy());
        }
        out
    }

    /// Registers a server consulted before the built-in registry. Used by
    /// tests (scripted servers) and, later, by user configuration.
    pub fn add_override(&mut self, config: ServerConfig) {
        self.overrides.push(config);
    }

    /// Resolves the server for a (root, language): user project entry →
    /// user default → programmatic override → built-in registry.
    #[cfg(test)]
    fn config_for(&self, root: &Path, language: &str) -> Option<ServerConfig> {
        self.configs_for(root, language).into_iter().next()
    }

    /// Every server for a (root, language), in the order the
    /// configuration names them. An entry lists more than one with
    /// ` ;; ` between them — `"python": "pyright ;; ruff"` — the
    /// separator a preprocessor chain uses for its links. A language is
    /// often a type checker and a linter, and each is its own server.
    fn configs_for(&self, root: &Path, language: &str) -> Vec<ServerConfig> {
        match self.configured_command(root, language) {
            Some(entry) => entry
                .split(";;")
                .map(str::trim)
                .filter(|one| !one.is_empty())
                .filter_map(|one| self.config_from(one, root, language))
                .collect(),
            None => self
                .overrides
                .iter()
                .find(|c| c.languages.iter().any(|l| l == language))
                .cloned()
                .or_else(|| server_for_language(language).map(ServerConfig::from))
                .into_iter()
                .collect(),
        }
    }

    /// One configured server: an id the registry or `lsp.servers`
    /// knows, or a command line.
    fn config_from(&self, command_line: &str, root: &Path, language: &str) -> Option<ServerConfig> {
        {
            // A value naming a server the registry knows takes that
            // server's command and its required arguments. Several
            // servers serve one language and only the first is reachable
            // by language alone, so naming one is how the others are
            // asked for.
            if let Some(spec) = self.server_spec(command_line.trim()) {
                return Some(ServerConfig {
                    id: spec.id,
                    command: Self::expand_placeholders(&spec.command, root),
                    args: spec
                        .args
                        .iter()
                        .map(|arg| Self::expand_placeholders(arg, root))
                        .collect(),
                    languages: vec![language.to_owned()],
                    install_hint: spec.install_hint,
                });
            }
            let mut parts = command_line
                .split_whitespace()
                .map(|part| Self::expand_placeholders(part, root));
            let command = parts.next()?;
            let args: Vec<String> = parts.collect();
            // A custom command that is the registry's server minus its
            // required arguments is the classic "exited during
            // initialize" — say so where the user will look.
            if let Some(spec) = server_for_language(language) {
                let same_binary = Path::new(&command)
                    .file_name()
                    .map(|name| name == std::ffi::OsStr::new(spec.command))
                    .unwrap_or(false);
                let missing: Vec<_> = spec
                    .args
                    .iter()
                    .filter(|required| !args.iter().any(|arg| arg == *required))
                    .collect();
                if same_binary && !missing.is_empty() {
                    crate::log::log(&format!(
                        "note: the built-in registry runs {} with {:?}; the \
                         configured command omits {:?} — most servers exit \
                         immediately without them",
                        spec.command, spec.args, missing
                    ));
                }
            }
            Some(ServerConfig {
                id: format!("custom:{command}"),
                command,
                args,
                languages: vec![language.to_owned()],
                install_hint: format!("configured in Settings for {language}"),
            })
        }
    }

    /// The root whose server instance looks after the project at `root`.
    ///
    /// A project with an entry of its own for the language is its own.
    /// Otherwise the nearest ancestor that configures it — one that has
    /// recursive configuration on, and either an entry for the language
    /// or the standing of being the project `root` would belong to if
    /// manifests did not split it (the repository, as a rule). That
    /// ancestor's instance serves the nested project too, unless it asks
    /// for separate servers, in which case the nested project keeps the
    /// inherited configuration and runs it for itself.
    ///
    /// One server at the top is what a workspace with one environment
    /// wants — a uv workspace has a single `.venv`, and a server started
    /// inside a member cannot see it, nor the other members. Separate
    /// servers are for members that each carry their own.
    fn serving_root(&self, root: &Path, language: &str) -> PathBuf {
        let has_entry = |dir: &Path| {
            self.configured["projects"][dir.to_string_lossy().as_ref()][language].is_string()
        };
        if has_entry(root) {
            return root.to_owned();
        }
        // Where the project would be rooted with manifests left alone:
        // an explicit `.textchum.json`, or the outermost repository.
        let base = workspace::project_root_with(root, &WorkspaceSettings::default());
        if base.as_deref() == Some(root) {
            // Not nested in anything: a directory further up may lend a
            // repository its configuration, never its server.
            return root.to_owned();
        }
        let mut ancestor = root.parent();
        while let Some(dir) = ancestor {
            let is_base = base.as_deref() == Some(dir);
            if self.workspace_settings.recursive_config(dir) && (has_entry(dir) || is_base) {
                return if self.workspace_settings.separate_nested_servers(dir) {
                    root.to_owned()
                } else {
                    dir.to_owned()
                };
            }
            if is_base {
                break;
            }
            ancestor = dir.parent();
        }
        root.to_owned()
    }

    /// The project root that scopes `path`'s server instance: the
    /// workspace model's answer (under the configured workspace
    /// settings), or the containing directory for loose files.
    fn root_for(&self, path: &Path) -> PathBuf {
        workspace::project_root_with(path, &self.workspace_settings)
            .or_else(|| path.parent().map(Path::to_owned))
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    /// Announces an opened document, spawning the (server, root) instance
    /// on first use.
    pub fn did_open(&mut self, path: &Path, language: &str, text: &str) {
        let project = self.root_for(path);
        let root = self.serving_root(&project, language);
        crate::log::log(&format!(
            "open {} language={language} root={}{}",
            path.display(),
            root.display(),
            if root == project {
                String::new()
            } else {
                format!(" (serving the nested project {})", project.display())
            }
        ));
        let configs = self.configs_for(&root, language);
        if configs.is_empty() {
            crate::log::log(&format!(
                "no server for language {language}: not configured and not in the registry"
            ));
            return;
        }
        let mut serving = Vec::new();
        for config in configs {
            crate::log::log(&format!(
                "server for {language}: {} ({} {:?})",
                config.id, config.command, config.args
            ));
            let key = (config.id.clone(), root.clone());
            if self.failed.contains(&key) {
                crate::log::log(&format!(
                    "{} at {} failed earlier this session; not retrying until \
                     restart or configuration change",
                    config.id,
                    root.display()
                ));
                continue;
            }
            if !self.instances.contains_key(&key) {
                match Instance::spawn(
                    &config,
                    &root,
                    self.events.clone(),
                    std::sync::Arc::clone(&self.published),
                    self.options_for(&config),
                ) {
                    Ok(instance) => {
                        self.instances.insert(key.clone(), instance);
                    }
                    Err(error) => {
                        crate::log::log(&format!(
                            "spawn failed for {} ({}): {error}; PATH={}",
                            config.id,
                            config.command,
                            std::env::var("PATH").unwrap_or_default()
                        ));
                        self.failed.insert(key);
                        let _ = self.events.send(Event::ServerStatus {
                            server: config.id.clone(),
                            root: root.to_string_lossy().into_owned(),
                            status: "not-found".into(),
                            message: format!(
                                "{} is not installed (install with: {})",
                                config.command, config.install_hint
                            ),
                        });
                        // One server missing is one feature missing:
                        // the others still start.
                        continue;
                    }
                }
            }
            serving.push(key);
        }
        if serving.is_empty() {
            return;
        }
        let version = 1;
        self.versions.insert(path.to_owned(), version);
        for key in &serving {
            self.instances[key].send(Command::DidOpen {
                path: path.to_owned(),
                language: language.to_owned(),
                version,
                text: text.to_owned(),
            });
            self.touch(key);
        }
        self.documents.insert(path.to_owned(), serving);
        self.sweep_idle();
    }

    /// Announces new document contents (full-text sync).
    pub fn did_change(&mut self, path: &Path, text: &str) {
        let Some(keys) = self.documents.get(path).cloned() else {
            return;
        };
        let version = *self
            .versions
            .entry(path.to_owned())
            .and_modify(|v| *v += 1)
            .or_insert(1);
        for key in &keys {
            if let Some(instance) = self.instances.get(key) {
                instance.send(Command::DidChange {
                    path: path.to_owned(),
                    version,
                    text: text.to_owned(),
                });
            }
            self.touch(key);
        }
    }

    /// Announces a closed document. The instance stays warm for the next
    /// open (idle shutdown is a later refinement).
    pub fn did_close(&mut self, path: &Path) {
        self.acting.remove(path);
        if let Some(keys) = self.documents.remove(path) {
            self.versions.remove(path);
            for key in &keys {
                if let Some(instance) = self.instances.get(key) {
                    instance.send(Command::DidClose {
                        path: path.to_owned(),
                    });
                }
                self.touch(key);
            }
        }
        self.sweep_idle();
    }

    /// Requests hover information at an LSP position (zero-based line,
    /// UTF-16 column). Returns the request id whose
    /// [`Event::LspResponse`] will carry the answer, or 0 when the
    /// document has no server.
    /// Tells the document's server it was written to disk. The text is
    /// not sent again: the server has it from the last change.
    pub fn did_save(&mut self, path: &Path) {
        let Some(keys) = self.documents.get(path).cloned() else {
            return;
        };
        for key in &keys {
            if let Some(instance) = self.instances.get(key) {
                instance.send(Command::DidSave {
                    path: path.to_owned(),
                });
            }
            self.touch(key);
        }
    }

    pub fn hover(&mut self, path: &Path, line: u32, character: u32) -> u64 {
        self.request(
            path,
            "hoverProvider",
            "textDocument/hover",
            serde_json::json!({
                "textDocument": {"uri": crate::uri::path_to_uri(path)},
                "position": {"line": line, "character": character},
            }),
        )
    }

    /// Requests the definition location(s) of the symbol at an LSP
    /// position; same contract as [`Self::hover`].
    pub fn definition(&mut self, path: &Path, line: u32, character: u32) -> u64 {
        self.request(
            path,
            "definitionProvider",
            "textDocument/definition",
            serde_json::json!({
                "textDocument": {"uri": crate::uri::path_to_uri(path)},
                "position": {"line": line, "character": character},
            }),
        )
    }

    /// Requests completions at an LSP position; same contract as
    /// [`Self::hover`]. The response's `result` is an LSP
    /// `CompletionItem[]` or `CompletionList`.
    pub fn completion(&mut self, path: &Path, line: u32, character: u32) -> u64 {
        self.request(
            path,
            "completionProvider",
            "textDocument/completion",
            serde_json::json!({
                "textDocument": {"uri": crate::uri::path_to_uri(path)},
                "position": {"line": line, "character": character},
            }),
        )
    }

    /// Requests every reference to the symbol at an LSP position, the
    /// declaration included; same contract as [`Self::hover`]. The
    /// response's `result` is an LSP `Location[]`.
    pub fn references(&mut self, path: &Path, line: u32, character: u32) -> u64 {
        self.request(
            path,
            "referencesProvider",
            "textDocument/references",
            serde_json::json!({
                "textDocument": {"uri": crate::uri::path_to_uri(path)},
                "position": {"line": line, "character": character},
                "context": {"includeDeclaration": true},
            }),
        )
    }

    /// Requests a workspace-wide rename of the symbol at an LSP position;
    /// same contract as [`Self::hover`]. The response's `result` is an
    /// LSP `WorkspaceEdit`.
    pub fn rename(&mut self, path: &Path, line: u32, character: u32, new_name: &str) -> u64 {
        self.request(
            path,
            "renameProvider",
            "textDocument/rename",
            serde_json::json!({
                "textDocument": {"uri": crate::uri::path_to_uri(path)},
                "position": {"line": line, "character": character},
                "newName": new_name,
            }),
        )
    }

    /// Requests the code actions offered at an LSP position — the
    /// quick fixes and refactorings a server has for it.
    ///
    /// The findings under the caret go with the request, as the server
    /// itself published them. That is what turns the answer into quick
    /// fixes: a server given no diagnostics offers the refactorings it
    /// has for the range and nothing about the problem there, and one
    /// given a reconstructed diagnostic does not recognize it.
    ///
    /// Same contract as [`Self::hover`]. The response's `result` is an
    /// array of `Command` and `CodeAction`.
    pub fn code_action(&mut self, path: &Path, line: u32, character: u32) -> u64 {
        // The server that reported the finding under the caret is the
        // one to ask how to fix it: with a type checker and a linter on
        // one document, the first to offer code actions would otherwise
        // be asked about the other's findings and know nothing of them.
        let keys = self.documents.get(path).cloned().unwrap_or_default();
        let found_by = |key: &InstanceKey| -> serde_json::Value {
            self.published
                .lock()
                .ok()
                .and_then(|published| published.get(&(key.0.clone(), path.to_owned())).cloned())
                .map(|found| {
                    textchum_core::code_action::diagnostics_at(&found.to_string(), line, character)
                })
                .unwrap_or_else(|| serde_json::Value::Array(Vec::new()))
        };
        let offers = |key: &&InstanceKey| {
            self.instances
                .get(*key)
                .is_some_and(|instance| instance.provides("codeActionProvider") != Some(false))
        };
        let chosen = keys
            .iter()
            .filter(offers)
            .find(|key| found_by(key).as_array().is_some_and(|found| !found.is_empty()))
            .or_else(|| keys.iter().find(offers))
            .cloned();
        let Some(key) = chosen else {
            return 0;
        };
        let diagnostics = found_by(&key);
        let position = serde_json::json!({"line": line, "character": character});
        self.acting.insert(path.to_owned(), key.clone());
        self.request_to(
            &key,
            "textDocument/codeAction",
            serde_json::json!({
                "textDocument": {"uri": crate::uri::path_to_uri(path)},
                "range": {"start": position, "end": position},
                "context": {"diagnostics": diagnostics},
            }),
        )
    }

    /// Asks the server that offered `action` to fill in its edit.
    pub fn resolve_code_action(&mut self, path: &Path, action: serde_json::Value) -> u64 {
        match self.acting.get(path).cloned() {
            Some(key) => self.request_to(&key, "codeAction/resolve", action),
            None => self.request(path, "codeActionProvider", "codeAction/resolve", action),
        }
    }

    /// Runs a command a code action named, on the server that named it.
    pub fn execute_command(
        &mut self,
        path: &Path,
        command: &str,
        arguments: serde_json::Value,
    ) -> u64 {
        let params = serde_json::json!({"command": command, "arguments": arguments});
        match self.acting.get(path).cloned() {
            Some(key) => self.request_to(&key, "workspace/executeCommand", params),
            None => self.request(path, "executeCommandProvider", "workspace/executeCommand", params),
        }
    }

    /// Requests whole-document formatting; same contract as
    /// [`Self::hover`]. The response's `result` is an LSP `TextEdit[]`.
    pub fn formatting(&mut self, path: &Path, tab_size: u32, insert_spaces: bool) -> u64 {
        self.request(
            path,
            "documentFormattingProvider",
            "textDocument/formatting",
            serde_json::json!({
                "textDocument": {"uri": crate::uri::path_to_uri(path)},
                "options": {"tabSize": tab_size, "insertSpaces": insert_spaces},
            }),
        )
    }

    /// Requests the document's symbol tree; same contract as
    /// [`Self::hover`]. The response's `result` is an LSP
    /// `DocumentSymbol[]` (hierarchical) or `SymbolInformation[]` (flat).
    pub fn document_symbols(&mut self, path: &Path) -> u64 {
        self.request(
            path,
            "documentSymbolProvider",
            "textDocument/documentSymbol",
            serde_json::json!({
                "textDocument": {"uri": crate::uri::path_to_uri(path)},
            }),
        )
    }

    /// Sends a request to the document's instance; the response arrives as
    /// an [`Event::LspResponse`] with the returned id (0 = no instance).
    /// Sends a request about `path` to the first of its servers that
    /// provides `provider`. One that has not answered initialize yet
    /// has not said no, and is asked.
    fn request(
        &mut self,
        path: &Path,
        provider: &str,
        method: &str,
        params: serde_json::Value,
    ) -> u64 {
        let Some(keys) = self.documents.get(path) else {
            return 0;
        };
        // A retired (crashed) instance may still be routed until the
        // shell re-announces; a request to it has no one to answer.
        let chosen = keys
            .iter()
            .find(|key| {
                self.instances
                    .get(*key)
                    .is_some_and(|instance| instance.provides(provider) != Some(false))
            })
            .cloned();
        match chosen {
            Some(key) => self.request_to(&key, method, params),
            None => 0,
        }
    }

    fn request_to(&mut self, key: &InstanceKey, method: &str, params: serde_json::Value) -> u64 {
        let Some(instance) = self.instances.get(key) else {
            return 0;
        };
        let id = self.next_request_id;
        self.next_request_id += 1;
        instance.send(Command::Request {
            id,
            method: method.to_owned(),
            params,
        });
        self.touch(key);
        id
    }

    /// Live instances as (server id, root) pairs, for status display.
    pub fn running(&self) -> Vec<(String, String)> {
        self.instances
            .keys()
            .map(|(id, root)| (id.clone(), root.to_string_lossy().into_owned()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pool with nowhere to send events: these tests ask it which
    /// server a language resolves to and never start one.
    fn pool_with(config: &str) -> Pool {
        let (events, _receiver) = std::sync::mpsc::channel();
        let mut pool = Pool::new(events);
        pool.configure(config);
        pool
    }

    /// A repository with two members, each with a manifest of its own.
    fn workspace_on_disk(name: &str) -> PathBuf {
        let repo = std::env::temp_dir()
            .join(format!("textchum-pool-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        for member in ["svc-a", "svc-b"] {
            std::fs::create_dir_all(repo.join(member).join("src")).unwrap();
            std::fs::write(repo.join(member).join("pyproject.toml"), "").unwrap();
            std::fs::write(repo.join(member).join("src/app.py"), "").unwrap();
        }
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        // Temporary directories are reached through a symlink on a Mac.
        repo.canonicalize().unwrap()
    }

    fn settings(repo: &Path, flags: &str, lsp: &str) -> String {
        format!(
            r#"{{"workspace": {{"projects": {{"{root}": {{{flags}}}}}}}, "lsp": {lsp}}}"#,
            root = repo.display()
        )
    }

    #[test]
    fn a_recursive_project_serves_its_nested_projects_from_one_instance() {
        let repo = workspace_on_disk("shared");
        let lsp = format!(
            r#"{{"projects": {{"{}": {{"python": "{{project}}/.venv/bin/server --stdio"}}}}}}"#,
            repo.display()
        );
        let pool = pool_with(&settings(
            &repo,
            r#""manifest_projects": true, "recursive_config": true"#,
            &lsp,
        ));
        let file = repo.join("svc-a/src/app.py");
        let project = pool.root_for(&file);
        assert_eq!(project, repo.join("svc-a"), "the member is a project of its own");
        let serving = pool.serving_root(&project, "python");
        assert_eq!(serving, repo, "and the repository's server looks after it");
        // One environment at the top: the command names it, not the member's.
        let config = pool.config_for(&serving, "python").expect("a server");
        assert_eq!(config.command, format!("{}/.venv/bin/server", repo.display()));
        // Both members, one instance.
        assert_eq!(pool.serving_root(&repo.join("svc-b"), "python"), repo);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn separate_nested_servers_share_the_configuration_and_nothing_else() {
        let repo = workspace_on_disk("separate");
        let lsp = format!(
            r#"{{"projects": {{"{}": {{"python": "{{project}}/.venv/bin/server --stdio"}}}}}}"#,
            repo.display()
        );
        let pool = pool_with(&settings(
            &repo,
            r#""manifest_projects": true, "recursive_config": true,
               "separate_nested_servers": true"#,
            &lsp,
        ));
        let member = repo.join("svc-a");
        let serving = pool.serving_root(&member, "python");
        assert_eq!(serving, member, "a server of its own");
        let config = pool.config_for(&serving, "python").expect("a server");
        assert_eq!(
            config.command,
            format!("{}/.venv/bin/server", member.display()),
            "running the repository's command line, in the member"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn nesting_without_recursion_changes_nothing() {
        let repo = workspace_on_disk("plain");
        let pool = pool_with(&settings(&repo, r#""manifest_projects": true"#, "{}"));
        let member = repo.join("svc-a");
        assert_eq!(pool.serving_root(&member, "python"), member);
        // Recursive, and no entry anywhere: the defaults are the shared
        // configuration, and the repository's instance still serves.
        let pool = pool_with(&settings(
            &repo,
            r#""manifest_projects": true, "recursive_config": true"#,
            "{}",
        ));
        assert_eq!(pool.serving_root(&member, "python"), repo);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn an_entry_of_its_own_keeps_a_nested_project_to_itself() {
        let repo = workspace_on_disk("own");
        let member = repo.join("svc-a");
        let lsp = format!(
            r#"{{"projects": {{"{}": {{"python": "pylsp"}}, "{}": {{"python": "pyright"}}}}}}"#,
            repo.display(),
            member.display()
        );
        let pool = pool_with(&settings(
            &repo,
            r#""manifest_projects": true, "recursive_config": true"#,
            &lsp,
        ));
        assert_eq!(pool.serving_root(&member, "python"), member);
        assert_eq!(pool.config_for(&member, "python").unwrap().id, "pyright");
        // Its sibling says nothing, and goes to the repository's.
        assert_eq!(pool.serving_root(&repo.join("svc-b"), "python"), repo);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_directory_above_a_repository_lends_its_configuration_not_its_server() {
        let repo = workspace_on_disk("above");
        let above = repo.parent().unwrap().to_owned();
        let config = format!(
            r#"{{"workspace": {{"recursive_config": true}},
                "lsp": {{"projects": {{"{}": {{"python": "pylsp"}}}}}}}}"#,
            above.display()
        );
        let pool = pool_with(&config);
        assert_eq!(pool.serving_root(&repo, "python"), repo);
        assert_eq!(pool.config_for(&repo, "python").unwrap().id, "pylsp");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_command_line_can_name_the_project_directory() {
        let pool = pool_with(
            r#"{"lsp": {"defaults":
               {"python": "{project}/.venv/bin/basedpyright-langserver --stdio"}}}"#,
        );
        let config = pool
            .config_for(Path::new("/work/service"), "python")
            .expect("a server");
        assert_eq!(config.command, "/work/service/.venv/bin/basedpyright-langserver");
        assert_eq!(config.args, vec!["--stdio".to_owned()]);
    }

    #[test]
    fn a_project_path_with_spaces_stays_one_argument() {
        let pool = pool_with(
            r#"{"lsp": {"defaults": {"python": "{project}/bin/server --stdio"}}}"#,
        );
        let config = pool
            .config_for(Path::new("/work/two words"), "python")
            .expect("a server");
        assert_eq!(config.command, "/work/two words/bin/server");
        assert_eq!(config.args, vec!["--stdio".to_owned()]);
    }

    #[test]
    fn naming_a_registered_server_takes_its_command_and_arguments() {
        let pool = pool_with(r#"{"lsp": {"defaults": {"python": "basedpyright"}}}"#);
        let config = pool
            .config_for(Path::new("/work/service"), "python")
            .expect("a server");
        assert_eq!(config.id, "basedpyright");
        assert_eq!(config.command, "basedpyright-langserver");
        // The arguments the server needs come with it; a command line
        // that omits them is the classic "exited during initialize".
        assert_eq!(config.args, vec!["--stdio".to_owned()]);
        assert!(config.install_hint.contains("basedpyright"));
    }

    #[test]
    fn an_unregistered_name_is_still_a_command_line() {
        let pool = pool_with(r#"{"lsp": {"defaults": {"python": "my-server --lsp"}}}"#);
        let config = pool
            .config_for(Path::new("/work/service"), "python")
            .expect("a server");
        assert_eq!(config.command, "my-server");
        assert_eq!(config.args, vec!["--lsp".to_owned()]);
    }

    #[test]
    fn a_language_with_no_configuration_gets_the_first_registered_server() {
        let pool = pool_with("{}");
        let config = pool
            .config_for(Path::new("/work/service"), "python")
            .expect("a server");
        assert_eq!(config.id, "pyright");
    }

    #[test]
    fn configuration_can_define_a_server_the_build_does_not_know() {
        let pool = pool_with(
            r#"{"lsp": {
                 "servers": {"mylsp": {
                    "command": "{project}/tools/mylsp",
                    "args": ["--lsp"],
                    "languages": ["mylang"],
                    "install": "make tools"}},
                 "defaults": {"mylang": "mylsp"}}}"#,
        );
        let config = pool
            .config_for(Path::new("/work/service"), "mylang")
            .expect("a server");
        assert_eq!(config.id, "mylsp");
        assert_eq!(config.command, "/work/service/tools/mylsp");
        assert_eq!(config.args, vec!["--lsp".to_owned()]);
        assert_eq!(config.install_hint, "make tools");
    }

    #[test]
    fn a_configured_server_redefines_the_built_in_of_the_same_id() {
        let pool = pool_with(
            r#"{"lsp": {
                 "servers": {"basedpyright": {
                    "command": "{project}/.venv/bin/basedpyright-langserver",
                    "args": ["--stdio"],
                    "languages": ["python"]}},
                 "defaults": {"python": "basedpyright"}}}"#,
        );
        let config = pool
            .config_for(Path::new("/work/service"), "python")
            .expect("a server");
        assert_eq!(
            config.command,
            "/work/service/.venv/bin/basedpyright-langserver"
        );
    }

    #[test]
    fn the_built_in_table_survives_a_configuration_that_adds_to_it() {
        // A configuration defining one server must not hide the rest,
        // or a build that learns a new one could never offer it.
        let pool = pool_with(
            r#"{"lsp": {"servers": {"mylsp": {
                 "command": "mylsp", "languages": ["python"]}}}}"#,
        );
        let ids = pool.servers_for_language("python");
        assert!(ids.contains(&"mylsp".to_owned()), "{ids:?}");
        assert!(ids.contains(&"pyright".to_owned()), "{ids:?}");
        assert!(ids.contains(&"basedpyright".to_owned()), "{ids:?}");
        // Defining one does not change which server a language gets
        // when nothing names one.
        assert_eq!(
            pool.config_for(Path::new("/work"), "python").unwrap().id,
            "pyright"
        );
    }

    #[test]
    fn a_configured_server_missing_its_command_is_ignored() {
        let pool = pool_with(
            r#"{"lsp": {"servers": {"broken": {"languages": ["python"]}},
                        "defaults": {"python": "broken"}}}"#,
        );
        // Nothing usable under that id, so the value falls back to
        // being read as a command line.
        let config = pool.config_for(Path::new("/work"), "python").expect("a server");
        assert_eq!(config.command, "broken");
    }

    #[test]
    fn placeholders_left_alone_when_nothing_asks_for_them() {
        assert_eq!(
            Pool::expand_placeholders("plain-server", Path::new("/work")),
            "plain-server"
        );
    }
}
