//! One running language-server process.
//!
//! Threading: a *manager* thread owns the child's stdin and processes
//! commands (open/change/close/shutdown); a *reader* thread owns the
//! child's stdout and turns server messages into core events. The
//! initialize handshake runs on the manager thread before either loop
//! starts, so document notifications can never precede it. All events
//! reach the shell through the app's single delivery channel.

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
    /// A client→server request; the response comes back asynchronously as
    /// an [`Event::LspResponse`] carrying the same id.
    Request {
        id: u64,
        method: String,
        params: Value,
    },
    Shutdown,
}

pub struct Instance {
    commands: mpsc::Sender<Command>,
    finished: mpsc::Receiver<()>,
    manager: Option<JoinHandle<()>>,
    child: Arc<Mutex<Child>>,
}

impl Instance {
    /// Spawns the server process and starts its handshake. Fails only if
    /// the process cannot be started at all (e.g. binary missing).
    pub fn spawn(
        config: &ServerConfig,
        root: &Path,
        events: EventSender,
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
        let manager = {
            let child = Arc::clone(&child);
            let server_id = config.id.clone();
            let root = root.to_owned();
            std::thread::Builder::new()
                .name(format!("lsp-{server_id}"))
                .spawn(move || {
                    run_manager(child, server_id, root, commands_rx, events);
                    let _ = finished_tx.send(());
                })
                .expect("failed to spawn LSP manager thread")
        };
        Ok(Self {
            commands: commands_tx,
            finished: finished_rx,
            manager: Some(manager),
            child,
        })
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

/// The manager thread body: handshake, then the command loop. The reader
/// thread is started after the handshake succeeds.
fn run_manager(
    child: Arc<Mutex<Child>>,
    server_id: String,
    root: PathBuf,
    commands: mpsc::Receiver<Command>,
    events: EventSender,
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
    let initialize = json!({
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
            "capabilities": {
                "textDocument": {
                    "publishDiagnostics": {},
                    "synchronization": {"didSave": false}
                }
            },
        },
    });
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
                    break;
                }
                answer_if_request(&stdin, &message);
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
    status(&events, &server_id, &root, "running", "");

    // --- Reader thread -------------------------------------------------
    let reader = {
        let stdin = Arc::clone(&stdin);
        let events = events.clone();
        let server_id = server_id.clone();
        let root = root.clone();
        std::thread::Builder::new()
            .name(format!("lsp-{server_id}-reader"))
            .spawn(move || loop {
                match read_message(&mut stdout) {
                    Ok(Some(message)) => {
                        if message.get("method").and_then(Value::as_str)
                            == Some("textDocument/publishDiagnostics")
                        {
                            publish_diagnostics(&events, &message);
                        } else if message.get("method").is_none() {
                            // A response to one of our requests. Ids 1–2
                            // (initialize/shutdown) are lifecycle traffic;
                            // everything else is forwarded to the shell.
                            if let Some(id) =
                                message.get("id").and_then(Value::as_u64).filter(|id| *id > 2)
                            {
                                let result = message.get("result").cloned().unwrap_or(Value::Null);
                                let _ = events.send(Event::LspResponse {
                                    id,
                                    json: serde_json::to_string(&result)
                                        .unwrap_or_else(|_| "null".into()),
                                });
                            }
                        } else {
                            answer_if_request(&stdin, &message);
                        }
                    }
                    // EOF or a broken pipe both mean the server is gone;
                    // the shell decides what to tell the user.
                    Ok(None) | Err(_) => {
                        status(&events, &server_id, &root, "exited", "");
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

/// Replies null to any server→client request so no server ever stalls
/// waiting on a capability we do not implement yet.
fn answer_if_request(stdin: &Arc<Mutex<std::process::ChildStdin>>, message: &Value) {
    if let (Some(id), Some(_method)) = (message.get("id"), message.get("method")) {
        let reply = json!({"jsonrpc": "2.0", "id": id, "result": null});
        let _ = write_message(&mut *stdin.lock().unwrap(), &reply);
    }
}

/// Converts a publishDiagnostics notification into a compact core event.
fn publish_diagnostics(events: &EventSender, message: &Value) {
    let params = &message["params"];
    let Some(path) = params["uri"].as_str().and_then(uri_to_path) else {
        return;
    };
    let diagnostics: Vec<Value> = params["diagnostics"]
        .as_array()
        .map(|items| {
            items
                .iter()
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
                .collect()
        })
        .unwrap_or_default();
    let _ = events.send(Event::Diagnostics {
        path: path.to_string_lossy().into_owned(),
        json: serde_json::to_string(&diagnostics).unwrap_or_else(|_| "[]".into()),
    });
}
