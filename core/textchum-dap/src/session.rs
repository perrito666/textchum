//! One debug session: an adapter process, spoken to over its standard
//! input and output, and the program it debugs.
//!
//! A manager thread owns everything about the session — the adapter's
//! input, the requests waiting for answers, what is stopped where — and
//! a reader thread only hands it what the adapter says, the way the
//! language-server client is built. The handshake is the protocol's:
//! `initialize`, then `launch`; when the adapter says `initialized`, the
//! breakpoints and `configurationDone`. A stop is followed by the stack
//! of the thread that stopped, asked for here, so a shell is told where
//! the program is rather than having to ask.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};
use textchum_core::framing::{read_message, write_message};
use textchum_core::{Event, EventSender};

/// What a session is asked to do.
#[derive(Debug, Clone)]
pub enum Command {
    Continue,
    Next,
    StepIn,
    StepOut,
    Pause,
    /// The variables of a frame of the stopped thread, by scope.
    Variables { frame: i64 },
    /// What a structured variable holds.
    Children { reference: i64 },
    /// The lines of `path` with a breakpoint now, zero-based.
    SetBreakpoints { path: PathBuf, lines: Vec<usize> },
    /// An expression in a frame's context.
    Evaluate { frame: i64, expression: String },
    Stop,
}

impl Command {
    /// A command as a shell sends it: `{command}` — continue, next,
    /// stepIn, stepOut, pause, stop — with `frame` for variables,
    /// `reference` for children, `frame` and `expression` for evaluate.
    pub fn from_json(json: &str) -> Option<Command> {
        let value: Value = serde_json::from_str(json).ok()?;
        Some(match value["command"].as_str()? {
            "continue" => Command::Continue,
            "next" => Command::Next,
            "stepIn" => Command::StepIn,
            "stepOut" => Command::StepOut,
            "pause" => Command::Pause,
            "stop" => Command::Stop,
            "variables" => Command::Variables { frame: value["frame"].as_i64()? },
            "children" => Command::Children { reference: value["reference"].as_i64()? },
            "evaluate" => Command::Evaluate {
                frame: value["frame"].as_i64()?,
                expression: value["expression"].as_str()?.to_owned(),
            },
            _ => return None,
        })
    }
}

/// What the manager waits on: a command, or something the adapter said.
enum Input {
    Command(Command),
    Adapter(Value),
    AdapterGone,
}

/// What a request was for, so its answer can be acted on.
enum Pending {
    Initialize,
    Launch,
    Breakpoints { path: PathBuf },
    ConfigurationDone,
    Stack { thread: i64, reason: String, description: String },
    Scopes { frame: i64 },
    Variables { frame: i64, scope: usize },
    Children { reference: i64 },
    Evaluate { expression: String },
    Resume,
    Disconnect,
    Other,
}

/// How a session is started.
#[derive(Debug, Clone)]
pub struct Launch {
    /// The adapter's command line: program first.
    pub adapter: Vec<String>,
    /// The adapter's id, as `initialize` names it.
    pub adapter_id: String,
    /// The `launch` request's arguments.
    pub arguments: Value,
    /// Where the adapter runs.
    pub cwd: PathBuf,
    /// The breakpoints there are when it starts, zero-based lines.
    pub breakpoints: Vec<(PathBuf, Vec<usize>)>,
    /// What installs the adapter, said when it goes before answering —
    /// which is how a missing Python module shows itself.
    pub install: String,
    /// The adapter is spoken to over a TCP connection rather than its
    /// standard input and output: it prints `… listening at: host:port`
    /// once it is ready, as Delve does.
    pub listens: bool,
}

pub struct Session {
    input: mpsc::Sender<Input>,
    child: Arc<Mutex<Child>>,
    manager: Option<JoinHandle<()>>,
}

impl Session {
    /// Starts the adapter and the handshake; an error says why the
    /// adapter could not be run.
    pub fn start(id: u64, launch: Launch, events: EventSender) -> std::io::Result<Session> {
        let (program, args) = launch
            .adapter
            .split_first()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no adapter command"))?;
        let mut child = ProcessCommand::new(program)
            .args(args)
            .current_dir(&launch.cwd)
            .envs(textchum_core::install::environment(&launch.cwd))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let child = Arc::new(Mutex::new(child));
        let (input, inputs) = mpsc::channel();
        {
            // What the adapter says on its own error stream is the
            // console's too: it is often why nothing happened.
            let events = events.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    send(&events, id, "output", json!({"category": "adapter", "text": format!("{line}\n")}));
                }
            });
        }
        let manager = {
            let (child, input) = (Arc::clone(&child), input.clone());
            std::thread::Builder::new().name(format!("dap-{id}")).spawn(move || {
                // Connected here rather than before returning: an adapter
                // that takes its time to listen does not hold the editor.
                let writer: Box<dyn Write + Send> = if launch.listens {
                    match connect(stdout, &events, id, &input) {
                        Ok(stream) => stream,
                        Err(why) => {
                            send(&events, id, "failed", json!({"message": why}));
                            send(&events, id, "ended", json!({}));
                            if let Ok(mut child) = child.lock() {
                                let _ = child.kill();
                                let _ = child.wait();
                            }
                            return;
                        }
                    }
                } else {
                    forward_messages(BufReader::new(stdout), input);
                    Box::new(stdin)
                };
                Manager::new(id, launch, writer, events).run(inputs, child)
            })?
        };
        Ok(Session { input, child, manager: Some(manager) })
    }

    /// False once the session is over.
    pub fn send(&self, command: Command) -> bool {
        self.input.send(Input::Command(command)).is_ok()
    }
}

impl Drop for Session {
    /// Nothing the editor started outlives it: asked to stop, then made.
    fn drop(&mut self) {
        let _ = self.input.send(Input::Command(Command::Stop));
        if let Some(manager) = self.manager.take() {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !manager.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if !manager.is_finished() {
                if let Ok(mut child) = self.child.lock() {
                    let _ = child.kill();
                }
            }
            let _ = manager.join();
        }
    }
}

/// How long a launch may take before it is said to be held up.
const LAUNCH_PATIENCE: Duration = Duration::from_secs(15);

/// Hands every message read from `reader` to the manager, then says the
/// adapter is gone.
fn forward_messages(mut reader: impl BufRead + Send + 'static, input: mpsc::Sender<Input>) {
    std::thread::spawn(move || {
        while let Ok(Some(message)) = read_message(&mut reader) {
            if input.send(Input::Adapter(message)).is_err() {
                return;
            }
        }
        let _ = input.send(Input::AdapterGone);
    });
}

/// For an adapter that listens: waits for it to say where, connects, and
/// hands what comes over the connection to the manager. What else the
/// adapter prints goes to the console.
fn connect(
    stdout: std::process::ChildStdout,
    events: &EventSender,
    id: u64,
    input: &mpsc::Sender<Input>,
) -> Result<Box<dyn Write + Send>, String> {
    let (address_found, address) = mpsc::channel::<String>();
    {
        let events = events.clone();
        std::thread::spawn(move || {
            let mut waiting = Some(address_found);
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(found) = waiting.take() {
                    match line.split("listening at:").nth(1) {
                        Some(at) => {
                            let _ = found.send(at.trim().to_owned());
                            continue;
                        }
                        None => waiting = Some(found),
                    }
                }
                send(&events, id, "output", json!({"category": "adapter", "text": format!("{line}\n")}));
            }
        });
    }
    let address = address
        .recv_timeout(LAUNCH_PATIENCE)
        .map_err(|_| textchum_core::i18n::tr("The adapter did not say where to reach it."))?;
    let stream = std::net::TcpStream::connect(&address).map_err(|error| format!("{address}: {error}"))?;
    let reading = stream.try_clone().map_err(|error| format!("{address}: {error}"))?;
    forward_messages(BufReader::new(reading), input.clone());
    Ok(Box::new(stream))
}

fn send(events: &EventSender, session: u64, kind: &str, body: Value) {
    let _ = events.send(Event::Debug { session, kind: kind.to_owned(), json: body.to_string() });
}

struct Manager {
    id: u64,
    launch: Launch,
    writer: Box<dyn Write + Send>,
    events: EventSender,
    seq: i64,
    pending: HashMap<i64, Pending>,
    capabilities: Value,
    /// The thread that stopped last, which the step commands act on.
    thread: Option<i64>,
    /// The scopes of a frame being gathered, until every one has its
    /// variables.
    gathering: HashMap<i64, Vec<(String, Option<Value>)>>,
    breakpoints: HashMap<PathBuf, Vec<usize>>,
    configured: bool,
    ended: bool,
    /// The adapter answered `initialize`: it is there and speaking.
    answered: bool,
    /// When `launch` was sent, until the program is seen to start; a
    /// launch that hangs is said, once.
    launching: Option<std::time::Instant>,
}

impl Manager {
    fn new(id: u64, launch: Launch, writer: Box<dyn Write + Send>, events: EventSender) -> Manager {
        let breakpoints = launch.breakpoints.iter().cloned().collect();
        Manager {
            id,
            launch,
            writer,
            events,
            seq: 0,
            pending: HashMap::new(),
            capabilities: Value::Null,
            thread: None,
            gathering: HashMap::new(),
            breakpoints,
            configured: false,
            ended: false,
            answered: false,
            launching: None,
        }
    }

    fn event(&self, kind: &str, body: Value) {
        send(&self.events, self.id, kind, body);
    }

    fn request(&mut self, command: &str, arguments: Value, pending: Pending) {
        self.seq += 1;
        let message = json!({"seq": self.seq, "type": "request", "command": command, "arguments": arguments});
        if write_message(&mut self.writer, &message).is_ok() {
            self.pending.insert(self.seq, pending);
        }
    }

    fn reply(&mut self, request: &Value, success: bool, message: &str) {
        self.seq += 1;
        let reply = json!({
            "seq": self.seq, "type": "response", "request_seq": request["seq"],
            "command": request["command"], "success": success, "message": message,
        });
        let _ = write_message(&mut self.writer, &reply);
    }

    fn run(mut self, inputs: mpsc::Receiver<Input>, child: Arc<Mutex<Child>>) {
        self.request(
            "initialize",
            json!({
                "clientID": "textchum", "clientName": "Textchum", "adapterID": self.launch.adapter_id,
                "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path",
                "supportsVariableType": true, "supportsRunInTerminalRequest": false,
            }),
            Pending::Initialize,
        );
        // An adapter that never answers is said, the same as a launch
        // that never starts.
        self.launching = Some(std::time::Instant::now());
        loop {
            let input = match inputs.recv_timeout(Duration::from_millis(250)) {
                Ok(input) => input,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.check_launch();
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            match input {
                Input::Command(Command::Stop) => {
                    if self.ended {
                        break;
                    }
                    self.request("disconnect", json!({"terminateDebuggee": true}), Pending::Disconnect);
                    // Answered or not, the adapter goes; the reader's end
                    // of the stream is what says it is gone.
                    let deadline = std::time::Instant::now() + Duration::from_secs(2);
                    while let Ok(input) = inputs.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
                        if matches!(input, Input::AdapterGone) {
                            break;
                        }
                    }
                    break;
                }
                Input::Command(command) => self.command(command),
                Input::Adapter(message) => self.message(message),
                Input::AdapterGone => break,
            }
        }
        if let Ok(mut child) = child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if !self.answered {
            let why = textchum_core::i18n::fill(
                &textchum_core::i18n::tr("The {} adapter stopped before it answered; if it is not installed: {}"),
                &[&self.launch.adapter_id, &self.launch.install],
            );
            self.event("failed", json!({"message": why}));
        }
        if !self.ended {
            self.event("ended", json!({}));
        }
    }

    /// A launch the adapter has neither answered nor started after a
    /// while is said, with what usually holds it up — on macOS, the
    /// system asking whether a debugger may take control of a program.
    fn check_launch(&mut self) {
        let Some(sent) = self.launching else { return };
        if sent.elapsed() < LAUNCH_PATIENCE {
            return;
        }
        self.launching = None;
        let why = if cfg!(target_os = "macos") {
            textchum_core::i18n::tr(
                "The program has not started after 15 seconds. macOS may be asking whether a debugger can take control of it: approve the request, or turn on developer mode with DevToolsSecurity -enable.",
            )
        } else {
            textchum_core::i18n::tr(
                "The program has not started after 15 seconds; the adapter may be waiting for something it cannot ask for here.",
            )
        };
        self.event("waiting", json!({"message": why}));
    }

    fn command(&mut self, command: Command) {
        let thread = json!({"threadId": self.thread.unwrap_or(0)});
        match command {
            Command::Continue => self.request("continue", thread, Pending::Resume),
            Command::Next => self.request("next", thread, Pending::Resume),
            Command::StepIn => self.request("stepIn", thread, Pending::Resume),
            Command::StepOut => self.request("stepOut", thread, Pending::Resume),
            Command::Pause => self.request("pause", thread, Pending::Other),
            Command::Variables { frame } => self.request("scopes", json!({"frameId": frame}), Pending::Scopes { frame }),
            Command::Children { reference } => self.request(
                "variables",
                json!({"variablesReference": reference}),
                Pending::Children { reference },
            ),
            Command::SetBreakpoints { path, lines } => {
                self.breakpoints.insert(path.clone(), lines);
                if self.configured {
                    self.set_breakpoints(&path);
                }
            }
            Command::Evaluate { frame, expression } => self.request(
                "evaluate",
                json!({"expression": expression, "frameId": frame, "context": "repl"}),
                Pending::Evaluate { expression },
            ),
            Command::Stop => {}
        }
    }

    fn set_breakpoints(&mut self, path: &PathBuf) {
        let lines = self.breakpoints.get(path).cloned().unwrap_or_default();
        let breakpoints: Vec<Value> = lines.iter().map(|line| json!({"line": line + 1})).collect();
        self.request(
            "setBreakpoints",
            json!({"source": {"path": path}, "breakpoints": breakpoints}),
            Pending::Breakpoints { path: path.clone() },
        );
    }

    fn message(&mut self, message: Value) {
        match message["type"].as_str() {
            Some("response") => {
                let Some(pending) = message["request_seq"].as_i64().and_then(|seq| self.pending.remove(&seq)) else {
                    return;
                };
                self.response(pending, &message);
            }
            Some("event") => self.adapter_event(&message),
            Some("request") => {
                // What the adapter could ask of the editor — a terminal
                // for the program, a child session — the editor does not
                // have; saying so lets it carry on another way.
                self.reply(&message, false, "Textchum has no terminal or child sessions to offer.");
            }
            _ => {}
        }
    }

    fn response(&mut self, pending: Pending, message: &Value) {
        let success = message["success"].as_bool().unwrap_or(false);
        let body = &message["body"];
        if !success {
            let why = message["message"].as_str().unwrap_or("the adapter refused").to_owned();
            match pending {
                Pending::Initialize | Pending::Launch => {
                    self.launching = None;
                    self.event("failed", json!({"message": why}));
                }
                Pending::Evaluate { expression } => {
                    self.event("evaluated", json!({"expression": expression, "result": why, "error": true}));
                }
                _ => {}
            }
            return;
        }
        match pending {
            Pending::Initialize => {
                self.answered = true;
                self.capabilities = body.clone();
                let arguments = self.launch.arguments.clone();
                self.request("launch", arguments, Pending::Launch);
                self.launching = Some(std::time::Instant::now());
            }
            Pending::Launch => {
                self.launching = None;
                self.event("running", json!({}));
            }
            Pending::Breakpoints { path } => {
                let lines: Vec<Value> = body["breakpoints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|breakpoint| {
                        json!({
                            "line": breakpoint["line"].as_u64().map(|line| line.saturating_sub(1)),
                            "verified": breakpoint["verified"].as_bool().unwrap_or(false),
                            "message": breakpoint["message"],
                        })
                    })
                    .collect();
                self.event("breakpoints", json!({"path": path, "breakpoints": lines}));
            }
            Pending::ConfigurationDone => {}
            Pending::Stack { thread, reason, description } => {
                let frames: Vec<Value> = body["stackFrames"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|frame| {
                        json!({
                            "id": frame["id"],
                            "name": frame["name"],
                            "path": frame["source"]["path"],
                            "line": frame["line"].as_u64().map(|line| line.saturating_sub(1)),
                            "column": frame["column"].as_u64().map(|column| column.saturating_sub(1)),
                        })
                    })
                    .collect();
                self.event(
                    "stopped",
                    json!({"reason": reason, "description": description, "thread": thread, "frames": frames}),
                );
            }
            Pending::Scopes { frame } => {
                let scopes: Vec<(String, i64)> = body["scopes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|scope| !scope["expensive"].as_bool().unwrap_or(false))
                    .map(|scope| {
                        (
                            scope["name"].as_str().unwrap_or_default().to_owned(),
                            scope["variablesReference"].as_i64().unwrap_or(0),
                        )
                    })
                    .collect();
                if scopes.is_empty() {
                    self.event("variables", json!({"frame": frame, "scopes": []}));
                    return;
                }
                self.gathering.insert(frame, scopes.iter().map(|(name, _)| (name.clone(), None)).collect());
                for (index, (_, reference)) in scopes.into_iter().enumerate() {
                    self.request(
                        "variables",
                        json!({"variablesReference": reference}),
                        Pending::Variables { frame, scope: index },
                    );
                }
            }
            Pending::Variables { frame, scope } => {
                let variables = variables_of(body);
                let Some(scopes) = self.gathering.get_mut(&frame) else { return };
                if let Some(entry) = scopes.get_mut(scope) {
                    entry.1 = Some(variables);
                }
                if scopes.iter().all(|(_, variables)| variables.is_some()) {
                    let scopes = self.gathering.remove(&frame).unwrap_or_default();
                    let scopes: Vec<Value> = scopes
                        .into_iter()
                        .map(|(name, variables)| json!({"name": name, "variables": variables}))
                        .collect();
                    self.event("variables", json!({"frame": frame, "scopes": scopes}));
                }
            }
            Pending::Children { reference } => {
                self.event("children", json!({"reference": reference, "variables": variables_of(body)}));
            }
            Pending::Evaluate { expression } => {
                self.event("evaluated", json!({"expression": expression, "result": body["result"]}));
            }
            Pending::Resume => self.event("continued", json!({})),
            Pending::Disconnect | Pending::Other => {}
        }
    }

    fn adapter_event(&mut self, message: &Value) {
        let body = &message["body"];
        match message["event"].as_str().unwrap_or_default() {
            "initialized" => {
                self.configured = true;
                let paths: Vec<PathBuf> = self.breakpoints.keys().cloned().collect();
                for path in paths {
                    self.set_breakpoints(&path);
                }
                if self.capabilities["supportsConfigurationDoneRequest"].as_bool().unwrap_or(false) {
                    self.request("configurationDone", json!({}), Pending::ConfigurationDone);
                }
            }
            "stopped" => {
                self.launching = None;
                let thread = body["threadId"].as_i64().or(self.thread).unwrap_or(0);
                self.thread = Some(thread);
                let reason = body["reason"].as_str().unwrap_or_default().to_owned();
                let description = body["description"].as_str().unwrap_or_default().to_owned();
                self.request(
                    "stackTrace",
                    json!({"threadId": thread, "startFrame": 0, "levels": 50}),
                    Pending::Stack { thread, reason, description },
                );
            }
            "continued" => self.event("continued", json!({})),
            "output" => {
                self.launching = None;
                // Telemetry is the adapter talking to its makers.
                if body["category"] != "telemetry" {
                    self.event("output", json!({"category": body["category"], "text": body["output"]}));
                }
            }
            "exited" => self.event("exited", json!({"exitCode": body["exitCode"]})),
            "terminated" => {
                self.ended = true;
                self.event("ended", json!({}));
                // The program is done; the adapter goes with it.
                self.request("disconnect", json!({"terminateDebuggee": true}), Pending::Disconnect);
            }
            _ => {}
        }
    }
}

/// A `variables` response's variables, each with what is needed to open
/// it further.
fn variables_of(body: &Value) -> Value {
    Value::Array(
        body["variables"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|variable| {
                json!({
                    "name": variable["name"],
                    "value": variable["value"],
                    "type": variable["type"],
                    "reference": variable["variablesReference"].as_i64().unwrap_or(0),
                })
            })
            .collect(),
    )
}
