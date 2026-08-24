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
    /// Which instance each open document talks to.
    documents: HashMap<PathBuf, InstanceKey>,
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
            versions: HashMap::new(),
            failed: HashSet::new(),
            overrides: Vec::new(),
            configured: serde_json::Value::Null,
            workspace_settings: WorkspaceSettings::default(),
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
    }

    /// Shuts down every running instance and forgets open-document
    /// routing; the shell re-announces documents to respawn under the
    /// current configuration.
    pub fn shutdown_all(&mut self) {
        self.instances.clear();
        self.documents.clear();
        self.versions.clear();
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

    /// Registers a server consulted before the built-in registry. Used by
    /// tests (scripted servers) and, later, by user configuration.
    pub fn add_override(&mut self, config: ServerConfig) {
        self.overrides.push(config);
    }

    /// Resolves the server for a (root, language): user project entry →
    /// user default → programmatic override → built-in registry.
    fn config_for(&self, root: &Path, language: &str) -> Option<ServerConfig> {
        if let Some(command_line) = self.configured_command(root, language) {
            let mut parts = command_line.split_whitespace().map(str::to_owned);
            let command = parts.next()?;
            return Some(ServerConfig {
                id: format!("custom:{command}"),
                command,
                args: parts.collect(),
                languages: vec![language.to_owned()],
                install_hint: format!("configured in Settings for {language}"),
            });
        }
        self.overrides
            .iter()
            .find(|c| c.languages.iter().any(|l| l == language))
            .cloned()
            .or_else(|| server_for_language(language).map(ServerConfig::from))
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
        let root = self.root_for(path);
        let Some(config) = self.config_for(&root, language) else {
            return;
        };
        let key = (config.id.clone(), root.clone());
        if self.failed.contains(&key) {
            return;
        }
        if !self.instances.contains_key(&key) {
            match Instance::spawn(&config, &root, self.events.clone()) {
                Ok(instance) => {
                    self.instances.insert(key.clone(), instance);
                }
                Err(_) => {
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
                    return;
                }
            }
        }
        let version = 1;
        self.versions.insert(path.to_owned(), version);
        self.documents.insert(path.to_owned(), key.clone());
        self.instances[&key].send(Command::DidOpen {
            path: path.to_owned(),
            language: language.to_owned(),
            version,
            text: text.to_owned(),
        });
    }

    /// Announces new document contents (full-text sync).
    pub fn did_change(&mut self, path: &Path, text: &str) {
        let Some(key) = self.documents.get(path) else {
            return;
        };
        let version = self
            .versions
            .entry(path.to_owned())
            .and_modify(|v| *v += 1)
            .or_insert(1);
        self.instances[key].send(Command::DidChange {
            path: path.to_owned(),
            version: *version,
            text: text.to_owned(),
        });
    }

    /// Announces a closed document. The instance stays warm for the next
    /// open (idle shutdown is a later refinement).
    pub fn did_close(&mut self, path: &Path) {
        if let Some(key) = self.documents.remove(path) {
            self.versions.remove(path);
            self.instances[&key].send(Command::DidClose {
                path: path.to_owned(),
            });
        }
    }

    /// Requests hover information at an LSP position (zero-based line,
    /// UTF-16 column). Returns the request id whose
    /// [`Event::LspResponse`] will carry the answer, or 0 when the
    /// document has no server.
    pub fn hover(&mut self, path: &Path, line: u32, character: u32) -> u64 {
        self.request(
            path,
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
            "textDocument/completion",
            serde_json::json!({
                "textDocument": {"uri": crate::uri::path_to_uri(path)},
                "position": {"line": line, "character": character},
            }),
        )
    }

    /// Sends a request to the document's instance; the response arrives as
    /// an [`Event::LspResponse`] with the returned id (0 = no instance).
    fn request(&mut self, path: &Path, method: &str, params: serde_json::Value) -> u64 {
        let Some(key) = self.documents.get(path) else {
            return 0;
        };
        let id = self.next_request_id;
        self.next_request_id += 1;
        self.instances[key].send(Command::Request {
            id,
            method: method.to_owned(),
            params,
        });
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
