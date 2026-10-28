//! The Debug Adapter Protocol client: a debug session per program
//! debugged, each talking to an adapter process — debugpy for Python,
//! Delve for Go, lldb-dap for Rust and the C family.
//!
//! What a shell starts is a runnable's `launch` description (see
//! `textchum_core::runnables`): which adapter, and the program or module
//! with its arguments. For Rust the program does not exist until cargo
//! has built it, so the build runs first, its messages going to the
//! debug console, and the session starts on the executable it made.
//! Everything a session says reaches the shell as
//! [`textchum_core::Event::Debug`]; see [`session`] for what it says.

pub mod session;

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use textchum_core::{Event, EventSender};

pub use session::{Command, Launch, Session};

/// A debug adapter the editor knows how to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// The id `initialize` names it by.
    pub id: &'static str,
    /// Its command line, program first.
    pub command: Vec<String>,
    /// What the install catalog knows it by (`textchum_core::install`).
    pub package: &'static str,
    /// A command that succeeds only when the adapter can run: for an
    /// adapter that is a module its interpreter imports, starting it is
    /// no test — the interpreter is there either way.
    pub check: Option<Vec<String>>,
    /// Spoken to over TCP, at the address it prints, rather than over its
    /// standard input and output.
    pub listens: bool,
}

/// The adapter a launch description names.
pub fn adapter(name: &str) -> Option<Adapter> {
    let words = |words: &[&str]| words.iter().map(|word| word.to_string()).collect();
    match name {
        "debugpy" => Some(Adapter {
            id: "debugpy",
            command: words(&["python3", "-m", "debugpy.adapter"]),
            package: "debugpy",
            check: Some(words(&["python3", "-c", "import debugpy"])),
            listens: false,
        }),
        // Delve speaks only over TCP: it is asked to listen where it
        // likes on this machine, and says where.
        "dlv" => Some(Adapter {
            id: "go",
            command: words(&["dlv", "dap", "--listen=127.0.0.1:0"]),
            package: "dlv",
            check: None,
            listens: true,
        }),
        // Xcode's lldb-dap is not on the PATH; xcrun finds it.
        "lldb" if cfg!(target_os = "macos") => Some(Adapter {
            id: "lldb-dap",
            command: words(&["xcrun", "lldb-dap"]),
            package: "lldb-dap",
            check: Some(words(&["xcrun", "--find", "lldb-dap"])),
            listens: false,
        }),
        "lldb" => Some(Adapter {
            id: "lldb-dap",
            command: vec![lldb_dap(std::env::var_os("PATH").as_deref())],
            package: "lldb-dap",
            check: None,
            listens: false,
        }),
        // The scripted adapter the smoke tests drive, where they say it is.
        "fake" => std::env::var("TEXTCHUM_FAKE_DAP").ok().map(|script| Adapter {
            id: "fake",
            command: vec!["python3".to_owned(), script],
            package: "fake",
            check: None,
            listens: false,
        }),
        _ => None,
    }
}

/// The lldb adapter on `path`: `lldb-dap` itself, else the newest of the
/// versioned names distributions install it under (`lldb-dap-18`), else
/// its older name, `lldb-vscode`, the same way. Plain `lldb-dap` when
/// none is there, for the error to name.
fn lldb_dap(path: Option<&std::ffi::OsStr>) -> String {
    let dirs: Vec<PathBuf> = path.map(|path| std::env::split_paths(path).collect()).unwrap_or_default();
    for base in ["lldb-dap", "lldb-vscode"] {
        if dirs.iter().any(|dir| dir.join(base).is_file()) {
            return base.to_owned();
        }
        let newest = dirs
            .iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let version: u32 = name.strip_prefix(base)?.strip_prefix('-')?.parse().ok()?;
                Some((version, name))
            })
            .max();
        if let Some((_, name)) = newest {
            return name;
        }
    }
    "lldb-dap".to_owned()
}

/// The `launch` request's arguments for a launch description, once its
/// program is known.
fn launch_arguments(adapter: &str, launch: &Value, cwd: &Path, program: Option<&str>) -> Value {
    let args = launch["args"].clone();
    match adapter {
        "debugpy" => {
            let mut arguments = json!({
                "type": "python", "request": "launch", "args": args, "cwd": cwd,
                // The program's output comes to the console, not to a
                // terminal the editor does not have.
                "console": "internalConsole", "justMyCode": true,
            });
            if let Some(module) = launch["module"].as_str() {
                arguments["module"] = module.into();
            } else {
                arguments["program"] = launch["program"].clone();
            }
            arguments
        }
        "fake" => json!({"program": launch["program"], "args": args}),
        "dlv" => json!({
            "request": "launch", "mode": launch["mode"], "program": launch["program"], "args": args, "cwd": cwd,
        }),
        _ => json!({"program": program, "args": args, "cwd": cwd, "stopOnEntry": false}),
    }
}

/// The executable a cargo build made for `pick` (`{kind, name?}`), from
/// its `--message-format=json` lines: the last artifact of that kind and
/// name with an executable.
pub fn picked_executable(lines: &[Value], pick: &Value) -> Option<String> {
    let kind = pick["kind"].as_str()?;
    lines
        .iter()
        .filter(|line| line["reason"] == "compiler-artifact")
        .filter(|line| {
            line["target"]["kind"].as_array().is_some_and(|kinds| kinds.iter().any(|known| known == kind))
                && pick["name"].as_str().is_none_or(|name| line["target"]["name"] == name)
        })
        .filter_map(|line| line["executable"].as_str().map(str::to_owned))
        .next_back()
}

/// Every debug session, by id.
pub struct Debuggers {
    events: EventSender,
    next: u64,
    sessions: Arc<Mutex<HashMap<u64, Session>>>,
}

impl Debuggers {
    pub fn new(events: EventSender) -> Debuggers {
        Debuggers { events, next: 1, sessions: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Debugs a runnable: `launch` is its launch description, `cwd` its
    /// directory, `breakpoints` the zero-based lines of each file with
    /// any. The id names the session in its events; an error says why it
    /// could not start, and what would make it.
    pub fn start(&mut self, launch: &Value, cwd: &Path, breakpoints: Vec<(PathBuf, Vec<usize>)>) -> Result<u64, String> {
        let name = launch["adapter"].as_str().unwrap_or_default();
        let Some(adapter) = adapter(name) else {
            return Err(textchum_core::i18n::tr("Nothing here can be debugged: put the caret in a test or a main."));
        };
        // Found in the project's environment first, as its language
        // server is: debugging a project means its interpreter.
        let command = textchum_core::install::resolve_command(&adapter.command, Some(cwd));
        if let Some(check) = &adapter.check {
            let check = textchum_core::install::resolve_command(check, Some(cwd));
            let ran = std::process::Command::new(&check[0])
                .args(&check[1..])
                .current_dir(cwd)
                .envs(textchum_core::install::environment(cwd))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if !ran.is_ok_and(|status| status.success()) {
                return Err(missing(&adapter, cwd));
            }
        }
        let id = self.next;
        self.next += 1;
        let install = textchum_core::install::hint(adapter.package).unwrap_or_default();
        let session_launch = |program: Option<&str>| Launch {
            adapter: command.clone(),
            adapter_id: adapter.id.to_owned(),
            arguments: launch_arguments(name, launch, cwd, program),
            cwd: cwd.to_path_buf(),
            breakpoints: breakpoints.clone(),
            install: install.clone(),
            listens: adapter.listens,
        };
        let Some(cargo) = launch["cargo"].as_array() else {
            let session = Session::start(id, session_launch(None), self.events.clone())
                .map_err(|error| not_installed(&adapter, cwd, &error))?;
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.insert(id, session);
            }
            return Ok(id);
        };
        // The program is made first, off the main thread: a build takes
        // as long as it takes, and says what it is doing as it goes.
        let cargo: Vec<String> = cargo.iter().filter_map(|word| word.as_str().map(str::to_owned)).collect();
        let pick = launch["pick"].clone();
        let (events, sessions, cwd) = (self.events.clone(), Arc::clone(&self.sessions), cwd.to_path_buf());
        let adapter = adapter.clone();
        let starting = session_launch(None);
        std::thread::spawn(move || {
            let say = |kind: &str, body: Value| {
                let _ = events.send(Event::Debug { session: id, kind: kind.to_owned(), json: body.to_string() });
            };
            say("output", json!({"category": "build", "text": format!("cargo {}\n", cargo.join(" "))}));
            let lines = match build(&cargo, &cwd, &|text| say("output", json!({"category": "build", "text": text}))) {
                Ok(lines) => lines,
                Err(why) => {
                    say("failed", json!({"message": why}));
                    return;
                }
            };
            let Some(program) = picked_executable(&lines, &pick) else {
                say("failed", json!({"message": textchum_core::i18n::tr("The build made no program to debug.")}));
                return;
            };
            let mut launch = starting;
            launch.arguments["program"] = program.into();
            match Session::start(id, launch, events.clone()) {
                Ok(session) => {
                    if let Ok(mut sessions) = sessions.lock() {
                        sessions.insert(id, session);
                    }
                }
                Err(error) => say("failed", json!({"message": not_installed(&adapter, &cwd, &error)})),
            }
        });
        Ok(id)
    }

    /// False when there is no such session, or it is over.
    pub fn send(&self, id: u64, command: Command) -> bool {
        self.sessions.lock().ok().and_then(|sessions| sessions.get(&id).map(|session| session.send(command))).unwrap_or(false)
    }

    /// A file's breakpoints changed: every session is told.
    pub fn set_breakpoints(&self, path: &Path, lines: &[usize]) {
        if let Ok(sessions) = self.sessions.lock() {
            for session in sessions.values() {
                session.send(Command::SetBreakpoints { path: path.to_path_buf(), lines: lines.to_vec() });
            }
        }
    }

    /// Ends a session and lets go of it.
    pub fn forget(&mut self, id: u64) {
        let session = self.sessions.lock().ok().and_then(|mut sessions| sessions.remove(&id));
        drop(session);
    }
}

/// Why an adapter could not be run, and what installs it.
fn not_installed(adapter: &Adapter, cwd: &Path, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        missing(adapter, cwd)
    } else {
        format!("{}: {error}", adapter.command.join(" "))
    }
}

/// The adapter is not there: noted where the editor offers to install
/// what is missing, and said so.
fn missing(adapter: &Adapter, cwd: &Path) -> String {
    if !textchum_core::install::knows(adapter.package) {
        return textchum_core::i18n::fill(
            &textchum_core::i18n::tr("{} is not installed"),
            &[&adapter.command.join(" ")],
        );
    }
    textchum_core::install::note_missing(adapter.package, cwd, &textchum_core::i18n::tr("debugger"));
    textchum_core::install::not_installed_message(adapter.package, Some(cwd))
}

/// Runs a cargo build in `cwd`, handing each message a person would read
/// to `say` and keeping the machine-readable lines; an error says why it
/// failed.
fn build(cargo: &[String], cwd: &Path, say: &dyn Fn(String)) -> Result<Vec<Value>, String> {
    let mut child = ProcessCommand::new("cargo")
        .args(cargo)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cargo: {error}"))?;
    let stderr = child.stderr.take().expect("piped stderr");
    let progress = std::thread::spawn(move || BufReader::new(stderr).lines().map_while(Result::ok).collect::<Vec<_>>());
    let mut lines = Vec::new();
    for line in BufReader::new(child.stdout.take().expect("piped stdout")).lines().map_while(Result::ok) {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(rendered) = message["message"]["rendered"].as_str() {
            say(rendered.to_owned());
        }
        lines.push(message);
    }
    for line in progress.join().unwrap_or_default() {
        say(format!("{line}\n"));
    }
    let status = child.wait().map_err(|error| format!("cargo: {error}"))?;
    if status.success() {
        Ok(lines)
    } else {
        Err(textchum_core::i18n::tr("The build failed; its messages are in the console."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_is_the_artifact_of_the_kind_and_name_asked_for() {
        let lines: Vec<Value> = [
            r#"{"reason":"compiler-artifact","target":{"kind":["lib"],"name":"shop"},"executable":null}"#,
            r#"{"reason":"compiler-artifact","target":{"kind":["lib"],"name":"shop"},"executable":"/t/shop-1"}"#,
            r#"{"reason":"compiler-artifact","target":{"kind":["test"],"name":"api"},"executable":"/t/api-2"}"#,
            r#"{"reason":"compiler-message","message":{"rendered":"warning"}}"#,
        ]
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
        assert_eq!(picked_executable(&lines, &json!({"kind": "lib"})).as_deref(), Some("/t/shop-1"));
        assert_eq!(picked_executable(&lines, &json!({"kind": "test", "name": "api"})).as_deref(), Some("/t/api-2"));
        assert_eq!(picked_executable(&lines, &json!({"kind": "bin"})), None);
    }

    #[test]
    fn lldb_dap_is_found_under_the_names_distributions_give_it() {
        let dir = std::env::temp_dir().join(format!("textchum-lldb-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = std::ffi::OsString::from(dir.as_os_str());
        assert_eq!(lldb_dap(Some(&path)), "lldb-dap", "nothing there: the name the error gives");
        std::fs::write(dir.join("lldb-vscode-14"), "").unwrap();
        assert_eq!(lldb_dap(Some(&path)), "lldb-vscode-14");
        std::fs::write(dir.join("lldb-dap-17"), "").unwrap();
        std::fs::write(dir.join("lldb-dap-18"), "").unwrap();
        assert_eq!(lldb_dap(Some(&path)), "lldb-dap-18", "the newest, and before the older name");
        std::fs::write(dir.join("lldb-dap"), "").unwrap();
        assert_eq!(lldb_dap(Some(&path)), "lldb-dap");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_adapter_gets_the_launch_it_reads() {
        let python = launch_arguments("debugpy", &json!({"module": "pytest", "args": ["-v", "t.py::x"]}), Path::new("/p"), None);
        assert_eq!((python["module"].as_str(), python["console"].as_str()), (Some("pytest"), Some("internalConsole")));
        let go = launch_arguments("dlv", &json!({"mode": "test", "program": "/p/shop", "args": []}), Path::new("/p"), None);
        assert_eq!(go["mode"], "test");
        let rust = launch_arguments("lldb", &json!({"args": ["tests::adds", "--exact"]}), Path::new("/p"), Some("/t/x"));
        assert_eq!((rust["program"].as_str(), rust["args"][0].as_str()), (Some("/t/x"), Some("tests::adds")));
        assert!(adapter("nothing").is_none());
    }
}
