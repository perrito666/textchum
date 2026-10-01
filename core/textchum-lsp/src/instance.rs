//! One running language-server process.
//!
//! Threading: a *manager* thread owns the child's stdin and processes
//! commands (open/change/close/shutdown); a *reader* thread owns the
//! child's stdout and turns server messages into core events. The
//! initialize handshake runs on the manager thread before either loop
//! starts, so document notifications can never precede it. All events
//! reach the shell through the app's single delivery channel.

use std::collections::HashMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};
use textchum_core::{Event, EventSender};

use crate::pool::ServerConfig;
use crate::transport::{read_message, write_message};
use crate::uri::{path_to_uri, uri_to_path};

/// Commands the pool sends to an instance's manager thread.
pub enum Command {
    DidOpen {
        path: PathBuf,
        language: String,
        version: i64,
        text: String,
    },
    DidChange {
        path: PathBuf,
        version: i64,
        text: String,
    },
    DidClose {
        path: PathBuf,
    },
    /// The document was written to disk. Servers that lint or check on
    /// save (cargo's lints through rust-analyzer, for one) run then.
    DidSave {
        path: PathBuf,
    },
    /// A client→server request; the response comes back asynchronously as
    /// an [`Event::LspResponse`] carrying the same id.
    Request {
        id: u64,
        method: String,
        params: Value,
    },
    Shutdown,
}

/// What the configuration gives a server to run with.
#[derive(Debug, Clone, Default)]
pub struct ServerOptions {
    /// `lsp.settings.<id>`: the settings object, keyed by section the
    /// way servers ask for it — `{"rust-analyzer": {"check": …}}`,
    /// `{"gopls": {…}}` — which is also how an nvim-lspconfig
    /// `settings` table is written. Pushed once the server is
    /// initialized, and handed out by section when it asks.
    pub settings: Value,
    /// `lsp.init_options.<id>`: sent verbatim as
    /// `initializationOptions`, for the servers that read theirs there.
    pub init_options: Value,
}

pub struct Instance {
    commands: mpsc::Sender<Command>,
    finished: mpsc::Receiver<()>,
    manager: Option<JoinHandle<()>>,
    child: Arc<Mutex<Child>>,
    /// What the server said it can do, once it has said: the
    /// `capabilities` of its initialize result.
    capabilities: Arc<Mutex<Option<Value>>>,
}

impl Instance {
    /// Spawns the server process and starts its handshake. Fails only if
    /// the process cannot be started at all (e.g. binary missing).
    pub fn spawn(
        config: &ServerConfig,
        root: &Path,
        events: EventSender,
        published: PublishedDiagnostics,
        waiters: Waiters,
        options: ServerOptions,
    ) -> std::io::Result<Self> {
        let mut child = ProcessCommand::new(&config.command)
            .args(&config.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // The server's own complaints are the best diagnostics there
        // are for "exited during initialize" — capture stderr into the
        // debug log, capped, but always drained so a chatty server
        // never blocks on a full pipe.
        if let Some(stderr) = child.stderr.take() {
            let server_id = config.id.clone();
            let _ = std::thread::Builder::new()
                .name(format!("lsp-{server_id}-stderr"))
                .spawn(move || {
                    use std::io::BufRead;
                    for (count, line) in BufReader::new(stderr).lines().enumerate() {
                        let Ok(line) = line else { break };
                        if count < 50 {
                            crate::log::log(&format!("stderr {server_id}: {line}"));
                        }
                    }
                });
        }
        let child = Arc::new(Mutex::new(child));

        let (commands_tx, commands_rx) = mpsc::channel::<Command>();
        let (finished_tx, finished_rx) = mpsc::channel::<()>();
        let capabilities = Arc::new(Mutex::new(None));
        let manager = {
            let child = Arc::clone(&child);
            let capabilities = Arc::clone(&capabilities);
            let server_id = config.id.clone();
            let root = root.to_owned();
            std::thread::Builder::new()
                .name(format!("lsp-{server_id}"))
                .spawn(move || {
                    run_manager(
                        child, server_id, root, options, capabilities, commands_rx, events,
                        published, waiters,
                    );
                    let _ = finished_tx.send(());
                })
                .expect("failed to spawn LSP manager thread")
        };
        Ok(Self {
            commands: commands_tx,
            finished: finished_rx,
            manager: Some(manager),
            child,
            capabilities,
        })
    }

    /// Whether the server said it provides `provider` (`hoverProvider`,
    /// `documentFormattingProvider`, …). `None` while it has not
    /// answered initialize yet, which is not a no.
    pub fn provides(&self, provider: &str) -> Option<bool> {
        let capabilities = self.capabilities.lock().ok()?;
        let said = capabilities.as_ref()?;
        Some(!matches!(said.get(provider), None | Some(Value::Null) | Some(Value::Bool(false))))
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        // A healthy server shuts down within the grace period; a wedged
        // one (or one stuck mid-handshake) gets killed so quitting the
        // app can never hang on a misbehaving process.
        if self.finished.recv_timeout(Duration::from_secs(2)).is_err() {
            if let Ok(mut child) = self.child.lock() {
                let _ = child.kill();
            }
            let _ = self.finished.recv_timeout(Duration::from_secs(2));
        }
        if let Some(handle) = self.manager.take() {
            let _ = handle.join();
        }
    }
}

fn status(events: &EventSender, server: &str, root: &Path, status: &str, message: &str) {
    crate::log::log(&format!(
        "status {server} [{}]: {status} {message}",
        root.display()
    ));
    let _ = events.send(Event::ServerStatus {
        server: server.to_owned(),
        root: root.to_string_lossy().into_owned(),
        status: status.to_owned(),
        message: message.to_owned(),
    });
}

/// What a server says it is busy with: indexing, checking, loading a
/// workspace. A server that looks idle and answers nothing for a minute
/// after a project opens is usually doing exactly this, and saying so
/// in `$/progress` notifications nobody was reading.
#[derive(Default)]
struct Progress {
    /// The title each running piece of work began with, by its token.
    titles: HashMap<String, String>,
    last: Option<std::time::Instant>,
}

impl Progress {
    /// Reports arrive many times a second; one in a while is plenty for
    /// a line of text.
    const QUIET: Duration = Duration::from_millis(150);

    fn busy(&self) -> bool {
        !self.titles.is_empty()
    }

    /// Folds one `$/progress` notification in. Returns the line to show
    /// — empty when the last piece of work ended — or `None` when this
    /// notification changes nothing worth redrawing.
    fn take(&mut self, message: &Value) -> Option<String> {
        let params = &message["params"];
        let token = match &params["token"] {
            Value::String(token) => token.clone(),
            other => other.to_string(),
        };
        let value = &params["value"];
        match value["kind"].as_str()? {
            "begin" => {
                let title = value["title"].as_str().unwrap_or_default().to_owned();
                self.titles.insert(token, title.clone());
                self.last = Some(std::time::Instant::now());
                Some(Self::line(&title, value))
            }
            "report" => {
                let title = self.titles.get(&token)?.clone();
                if self.last.is_some_and(|last| last.elapsed() < Self::QUIET) {
                    return None;
                }
                self.last = Some(std::time::Instant::now());
                Some(Self::line(&title, value))
            }
            "end" => {
                self.titles.remove(&token)?;
                // Other work still running keeps its line until its
                // next report; the last to end clears it.
                self.titles.is_empty().then(String::new)
            }
            _ => None,
        }
    }

    fn line(title: &str, value: &Value) -> String {
        let mut line = title.to_owned();
        if let Some(detail) = value["message"].as_str().filter(|detail| !detail.is_empty()) {
            line.push_str(": ");
            line.push_str(detail);
        }
        if let Some(percentage) = value["percentage"].as_u64() {
            line.push_str(&format!(" {percentage}%"));
        }
        line
    }
}

/// Progress is not a transition, so it goes to the shell without a
/// line in the log: `status` is `progress`, and an empty message means
/// the work is done.
fn send_progress(events: &EventSender, server: &str, root: &Path, text: &str) {
    let _ = events.send(Event::ServerStatus {
        server: server.to_owned(),
        root: root.to_string_lossy().into_owned(),
        status: "progress".to_owned(),
        message: text.to_owned(),
    });
}

/// The manager thread body: handshake, then the command loop. The reader
/// thread is started after the handshake succeeds.
fn run_manager(
    child: Arc<Mutex<Child>>,
    server_id: String,
    root: PathBuf,
    options: ServerOptions,
    capabilities: Arc<Mutex<Option<Value>>>,
    commands: mpsc::Receiver<Command>,
    events: EventSender,
    published: PublishedDiagnostics,
    waiters: Waiters,
) {
    status(&events, &server_id, &root, "starting", "");
    let (stdin, stdout) = {
        let mut child = child.lock().expect("child mutex");
        (
            child.stdin.take().expect("piped stdin"),
            child.stdout.take().expect("piped stdout"),
        )
    };
    let stdin = Arc::new(Mutex::new(stdin));
    let mut stdout = BufReader::new(stdout);

    // --- Handshake -----------------------------------------------------
    let mut initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": std::process::id(),
            "rootUri": path_to_uri(&root),
            "workspaceFolders": [{
                "uri": path_to_uri(&root),
                "name": root.file_name().map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "root".into()),
            }],
            // Each entry is something the shells already do with the
            // answer. Servers gate richer answers on these: without
            // snippetSupport rust-analyzer completes a function as its
            // bare name, and without the code-action literals it can
            // only offer commands.
            "capabilities": {
                "textDocument": {
                    "publishDiagnostics": {},
                    "synchronization": {"didSave": true},
                    "completion": {"completionItem": {
                        "snippetSupport": true,
                        "insertReplaceSupport": true
                    }},
                    "hover": {"contentFormat": ["markdown", "plaintext"]},
                    "signatureHelp": {"signatureInformation": {
                        "documentationFormat": ["markdown", "plaintext"],
                        "parameterInformation": {"labelOffsetSupport": true},
                        "activeParameterSupport": true
                    }},
                    "definition": {"linkSupport": true},
                    "documentHighlight": {},
                    "codeAction": {
                        "codeActionLiteralSupport": {"codeActionKind": {"valueSet": [
                            "", "quickfix", "refactor", "refactor.extract",
                            "refactor.inline", "refactor.rewrite", "source",
                            "source.organizeImports"
                        ]}},
                        "dataSupport": true,
                        "resolveSupport": {"properties": ["edit"]}
                    }
                },
                // Said because it is now true: a server that pulls its
                // settings only asks a client that declares it answers.
                "workspace": {
                    "configuration": true,
                    "workspaceFolders": true,
                    "didChangeConfiguration": {"dynamicRegistration": false}
                },
                "window": {"workDoneProgress": true}
            },
        },
    });
    if !options.init_options.is_null() {
        initialize["params"]["initializationOptions"] = options.init_options.clone();
    }
    if write_message(&mut *stdin.lock().unwrap(), &initialize).is_err() {
        status(&events, &server_id, &root, "failed", "could not write initialize");
        return;
    }
    // Read until the initialize response; servers may emit notifications
    // (logs) and requests first — requests get a null reply so nothing
    // stalls.
    loop {
        match read_message(&mut stdout) {
            Ok(Some(message)) => {
                if message.get("id") == Some(&json!(1)) && message.get("method").is_none() {
                    if let Some(error) = message.get("error") {
                        status(&events, &server_id, &root, "failed", &error.to_string());
                        return;
                    }
                    if let Ok(mut said) = capabilities.lock() {
                        *said = Some(message["result"]["capabilities"].clone());
                    }
                    break;
                }
                answer_if_request(&stdin, &root, &options.settings, &message);
            }
            Ok(None) => {
                status(&events, &server_id, &root, "exited", "during initialize");
                return;
            }
            Err(e) => {
                status(&events, &server_id, &root, "failed", &e.to_string());
                return;
            }
        }
    }
    let initialized = json!({"jsonrpc": "2.0", "method": "initialized", "params": {}});
    let _ = write_message(&mut *stdin.lock().unwrap(), &initialized);
    // Servers split on how they take settings: some read what is pushed
    // here, some ask `workspace/configuration` and are answered from the
    // same object. Doing both is what nvim's client does, and neither
    // kind minds the other half.
    if !options.settings.is_null() {
        let changed = json!({
            "jsonrpc": "2.0",
            "method": "workspace/didChangeConfiguration",
            "params": {"settings": options.settings},
        });
        let _ = write_message(&mut *stdin.lock().unwrap(), &changed);
    }
    status(&events, &server_id, &root, "running", "");

    // --- Reader thread -------------------------------------------------
    // Set before the orderly shutdown sequence, so the reader can tell
    // "we closed it" (status `closed`) from "it died" (status `exited`)
    // — the restart logic must only chase the latter.
    let orderly = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = {
        let stdin = Arc::clone(&stdin);
        let events = events.clone();
        let server_id = server_id.clone();
        let root = root.clone();
        let orderly = Arc::clone(&orderly);
        let published = Arc::clone(&published);
        let settings = options.settings.clone();
        let mut progress = Progress::default();
        std::thread::Builder::new()
            .name(format!("lsp-{server_id}-reader"))
            .spawn(move || loop {
                match read_message(&mut stdout) {
                    Ok(Some(message)) => {
                        let method = message.get("method").and_then(Value::as_str);
                        if method == Some("textDocument/publishDiagnostics") {
                            publish_diagnostics(&events, &published, &server_id, &message);
                        } else if method == Some("$/progress") {
                            if let Some(text) = progress.take(&message) {
                                send_progress(&events, &server_id, &root, &text);
                            }
                        } else if message.get("method").is_none() {
                            // A response to one of our requests. Ids 1–2
                            // (initialize/shutdown) are lifecycle traffic;
                            // everything else is forwarded to the shell.
                            if let Some(id) =
                                message.get("id").and_then(Value::as_u64).filter(|id| *id > 2)
                            {
                                let result = message.get("result").cloned().unwrap_or(Value::Null);
                                let json = serde_json::to_string(&result)
                                    .unwrap_or_else(|_| "null".into());
                                let waiter =
                                    waiters.lock().ok().and_then(|mut waiting| waiting.remove(&id));
                                match waiter {
                                    Some(waiter) => {
                                        let _ = waiter.send(json);
                                    }
                                    None => {
                                        let _ = events.send(Event::LspResponse { id, json });
                                    }
                                }
                            }
                        } else {
                            answer_if_request(&stdin, &root, &settings, &message);
                        }
                    }
                    // EOF or a broken pipe both mean the server is gone;
                    // the shell decides what to tell the user.
                    Ok(None) | Err(_) => {
                        // Whatever it was in the middle of, it is not
                        // doing it any more.
                        if progress.busy() {
                            send_progress(&events, &server_id, &root, "");
                        }
                        let expected = orderly.load(std::sync::atomic::Ordering::SeqCst);
                        status(
                            &events,
                            &server_id,
                            &root,
                            if expected { "closed" } else { "exited" },
                            "",
                        );
                        return;
                    }
                }
            })
            .expect("failed to spawn LSP reader thread")
    };

    // --- Command loop --------------------------------------------------
    while let Ok(command) = commands.recv() {
        let message = match command {
            Command::DidOpen {
                path,
                language,
                version,
                text,
            } => json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {"textDocument": {
                    "uri": path_to_uri(&path),
                    "languageId": language,
                    "version": version,
                    "text": text,
                }},
            }),
            Command::DidChange { path, version, text } => json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": {"uri": path_to_uri(&path), "version": version},
                    // Full-document sync: correct everywhere, and plenty
                    // fast at editor scale. Incremental sync is a later
                    // optimization the interface already permits.
                    "contentChanges": [{"text": text}],
                },
            }),
            Command::DidClose { path } => json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didClose",
                "params": {"textDocument": {"uri": path_to_uri(&path)}},
            }),
            Command::DidSave { path } => json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didSave",
                "params": {"textDocument": {"uri": path_to_uri(&path)}},
            }),
            Command::Request { id, method, params } => json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }),
            Command::Shutdown => break,
        };
        if write_message(&mut *stdin.lock().unwrap(), &message).is_err() {
            break;
        }
    }

    // --- Orderly exit --------------------------------------------------
    orderly.store(true, std::sync::atomic::Ordering::SeqCst);
    let shutdown = json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null});
    let exit = json!({"jsonrpc": "2.0", "method": "exit", "params": null});
    {
        let mut stdin = stdin.lock().unwrap();
        let _ = write_message(&mut *stdin, &shutdown);
        let _ = write_message(&mut *stdin, &exit);
    }
    drop(stdin);
    let _ = reader.join();
    if let Ok(mut child) = child.lock() {
        let _ = child.wait();
    }
}

/// Answers a server→client request, so no server ever stalls waiting
/// on a capability the editor does not implement.
///
/// The answer has to be the shape the protocol names for the method,
/// not merely *an* answer: taplo's JSON-RPC layer reads a bare `null`
/// result as "no result and no error" and panics on it, and it asks
/// `workspace/configuration` the moment it is initialized. That question
/// gets an array with one entry per item — the configured settings'
/// section the item names, or the null the specification prescribes
/// when there is none — and `workspace/workspaceFolders` gets the root
/// the instance was started for. Everything else the editor cannot
/// answer is void, and null is its answer.
fn answer_if_request(
    stdin: &Arc<Mutex<std::process::ChildStdin>>,
    root: &Path,
    settings: &Value,
    message: &Value,
) {
    let (Some(id), Some(method)) = (
        message.get("id"),
        message.get("method").and_then(Value::as_str),
    ) else {
        return;
    };
    let reply = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": answer_for(method, message.get("params"), root, settings),
    });
    let _ = write_message(&mut *stdin.lock().unwrap(), &reply);
}

/// The part of `settings` a server asks for by `section`.
///
/// No section means all of it. A section is looked for whole first —
/// `rust-analyzer` is a key, not a path — and then walked by its dots,
/// the way pyright asks for `python.analysis` out of
/// `{"python": {"analysis": …}}`. What is not there is null, which
/// every server reads as "use your default".
fn section_of(settings: &Value, section: Option<&str>) -> Value {
    let Some(section) = section.filter(|section| !section.is_empty()) else {
        return settings.clone();
    };
    if let Some(whole) = settings.get(section) {
        return whole.clone();
    }
    let mut at = settings;
    for part in section.split('.') {
        match at.get(part) {
            Some(next) => at = next,
            None => return Value::Null,
        }
    }
    at.clone()
}

/// The result for a server→client `method` with `params`.
fn answer_for(method: &str, params: Option<&Value>, root: &Path, settings: &Value) -> Value {
    match method {
        "workspace/configuration" => Value::Array(
            params
                .and_then(|params| params.get("items"))
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            section_of(settings, item.get("section").and_then(Value::as_str))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
        "workspace/workspaceFolders" => json!([{
            "uri": path_to_uri(root),
            "name": root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "root".into()),
        }]),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_is_answered_with_one_null_per_item() {
        let params = json!({"items": [{"section": "a"}, {"section": "b"}, {"section": "c"}]});
        let answer = answer_for("workspace/configuration", Some(&params), Path::new("/p"), &Value::Null);
        assert_eq!(answer, json!([null, null, null]));
    }

    #[test]
    fn configuration_without_items_is_an_empty_array_not_null() {
        let answer = answer_for("workspace/configuration", None, Path::new("/p"), &Value::Null);
        assert_eq!(answer, json!([]));
    }

    #[test]
    fn workspace_folders_name_the_root() {
        let answer = answer_for("workspace/workspaceFolders", None, Path::new("/tmp/proj"), &Value::Null);
        assert_eq!(answer[0]["name"], "proj");
        assert_eq!(answer[0]["uri"], path_to_uri(Path::new("/tmp/proj")));
    }

    #[test]
    fn configuration_hands_out_the_settings_by_section() {
        let settings = json!({
            "rust-analyzer": {"check": {"command": "clippy"}},
            "python": {"analysis": {"typeCheckingMode": "strict"}},
        });
        let params = json!({"items": [
            {"section": "rust-analyzer"},
            {"section": "python.analysis"},
            {"section": "python.analysis.typeCheckingMode"},
            {"section": "nothing.here"},
            {},
        ]});
        let answer = answer_for("workspace/configuration", Some(&params), Path::new("/p"), &settings);
        assert_eq!(answer[0], json!({"check": {"command": "clippy"}}));
        assert_eq!(answer[1], json!({"typeCheckingMode": "strict"}));
        assert_eq!(answer[2], json!("strict"));
        assert_eq!(answer[3], Value::Null);
        assert_eq!(answer[4], settings);
    }

    #[test]
    fn anything_else_is_void() {
        let answer = answer_for("client/registerCapability", Some(&json!({})), Path::new("/p"), &Value::Null);
        assert_eq!(answer, Value::Null);
    }
}

/// What the servers last said about each file, as they said it.
///
/// The event the shells get is a compact shape — a range, a severity
/// and a message — which is all a squiggle needs. A code action request
/// needs the diagnostic the server actually published: `code`, `data`
/// and `source` are how a server recognizes its own finding and offers
/// the fix for it, and a reconstructed one gets a shrug.
///
/// Keyed by server as well as file: a document can have more than one
/// server, each publishing its own set, and the shells are shown all
/// of them together.
pub type PublishedDiagnostics = Arc<Mutex<HashMap<(String, PathBuf), Value>>>;

/// Requests whose answer somebody is waiting for on the spot, by id.
/// An answer goes to its waiter instead of the event channel: a save
/// that asked the server to format has to have the text before it
/// writes the file, and cannot take it from an event later.
pub type Waiters = Arc<Mutex<HashMap<u64, mpsc::Sender<String>>>>;

/// Converts a publishDiagnostics notification into a compact core
/// event, and keeps the original for the requests that need it.
fn publish_diagnostics(
    events: &EventSender,
    published: &PublishedDiagnostics,
    server: &str,
    message: &Value,
) {
    let params = &message["params"];
    let Some(path) = params["uri"].as_str().and_then(uri_to_path) else {
        return;
    };
    let Ok(mut published) = published.lock() else {
        return;
    };
    published.insert((server.to_owned(), path.clone()), params["diagnostics"].clone());
    let _ = events.send(findings_event(&published, &path));
}

/// The event that tells a shell what is known about `path`: every
/// server's findings for it together, in the compact shape a mark
/// needs. One server publishing replaces its own set and leaves the
/// others' standing.
pub(crate) fn findings_event(
    published: &HashMap<(String, PathBuf), Value>,
    path: &Path,
) -> Event {
    let mut servers: Vec<&String> = published
        .keys()
        .filter(|(_, file)| file == path)
        .map(|(server, _)| server)
        .collect();
    // An order that does not depend on the hash map's.
    servers.sort();
    let diagnostics: Vec<Value> = servers
        .into_iter()
        .filter_map(|server| published.get(&(server.clone(), path.to_owned())))
        .filter_map(Value::as_array)
        .flatten()
        .map(|d| {
            json!({
                "line": d["range"]["start"]["line"],
                "character": d["range"]["start"]["character"],
                "endLine": d["range"]["end"]["line"],
                "endCharacter": d["range"]["end"]["character"],
                "severity": d["severity"].as_u64().unwrap_or(1),
                "message": d["message"].as_str().unwrap_or(""),
            })
        })
        .collect();
    Event::Diagnostics {
        path: path.to_string_lossy().into_owned(),
        json: serde_json::to_string(&diagnostics).unwrap_or_else(|_| "[]".into()),
    }
}
