//! Commands run from the editor, their output piped into the run panel.
//!
//! Not a terminal: a build, a test run or a program's own output needs
//! a pipe, not an emulator. What a terminal would have given is asked
//! for or made up for here. Colour is asked for through the
//! environment, since a tool writing into a pipe turns it off otherwise,
//! and the output is read for it by [`crate::output`]. Output a runtime
//! would hold back until its buffer fills is asked for line by line. A
//! program that goes quiet while still running may be waiting for
//! input; that is said, once, and a line can be sent to it. Stopping
//! takes the whole process group down, so what the command started —
//! the test binary cargo ran — goes with it.
//!
//! Events go to the shells through the app's channel: a run says
//! `output` when there is something new to read (once, until it is
//! read), `quiet` when it has said nothing for a while, and `exited`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::app::{Event, EventSender};
use crate::output::{changes_json, Place, Transcript};

/// How long a running command may say nothing before it is said that it
/// may be waiting for input.
pub const QUIET_AFTER: Duration = Duration::from_secs(15);

/// How long a command has to stop after being asked before it is made to.
const GRACE: Duration = Duration::from_secs(3);

/// What makes a tool writing into a pipe behave as it would in a
/// terminal: colour on, output line by line.
const ENVIRONMENT: [(&str, &str); 6] = [
    ("CLICOLOR_FORCE", "1"),
    ("FORCE_COLOR", "1"),
    ("CARGO_TERM_COLOR", "always"),
    ("PYTHONUNBUFFERED", "1"),
    ("NODE_DISABLE_COLORS", ""),
    ("TERM", "xterm-256color"),
];

struct State {
    transcript: Transcript,
    command: String,
    stdin: Option<ChildStdin>,
    /// The process group: the command's own id.
    group: i32,
    /// None while running; the exit code, or None inside, once done.
    finished: Option<Option<i32>>,
    last_output: Instant,
    quiet_said: bool,
    /// An `output` event was sent and not yet answered by a read.
    owed: bool,
}

/// Every run, by id.
pub struct Runs {
    sender: EventSender,
    next: u64,
    runs: HashMap<u64, Arc<Mutex<State>>>,
}

impl Runs {
    pub fn new(sender: EventSender) -> Runs {
        Runs { sender, next: 1, runs: HashMap::new() }
    }

    /// Runs `command` through the shell in `cwd`. The id names it in
    /// events and in every call after; an error says why it could not
    /// start.
    pub fn start(&mut self, command: &str, cwd: &Path) -> Result<u64, String> {
        let mut child = spawn(command, cwd).map_err(|error| format!("{command}: {error}"))?;
        let id = self.next;
        self.next += 1;
        let state = Arc::new(Mutex::new(State {
            transcript: Transcript::new(cwd),
            command: command.to_owned(),
            stdin: child.stdin.take(),
            group: child.id() as i32,
            finished: None,
            last_output: Instant::now(),
            quiet_said: false,
            owed: false,
        }));
        let readers: Vec<_> = [
            child.stdout.take().map(|out| Box::new(out) as Box<dyn Read + Send>),
            child.stderr.take().map(|err| Box::new(err) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .map(|stream| {
            let state = Arc::clone(&state);
            let sender = self.sender.clone();
            thread::spawn(move || read_into(stream, &state, &sender, id))
        })
        .collect();
        {
            let state = Arc::clone(&state);
            let sender = self.sender.clone();
            thread::spawn(move || {
                let status = child.wait();
                // What was written before the end is read before it.
                for reader in readers {
                    let _ = reader.join();
                }
                let code = status.ok().and_then(|status| status.code());
                if let Ok(mut state) = state.lock() {
                    state.finished = Some(code);
                    state.stdin = None;
                    let ending = match code {
                        Some(code) => crate::i18n::fill(&crate::i18n::tr("exited with {}"), &[&code.to_string()]),
                        None => crate::i18n::tr("stopped"),
                    };
                    state.transcript.note(&format!("— {ending}"));
                }
                let _ = sender.send(Event::Run { id, status: "exited".into() });
            });
        }
        {
            let state = Arc::clone(&state);
            let sender = self.sender.clone();
            thread::spawn(move || watch_for_quiet(&state, &sender, id));
        }
        self.runs.insert(id, state);
        Ok(id)
    }

    /// What changed in the run's output since it was last read, as the
    /// JSON [`changes_json`] makes, with `running`; None for an id there
    /// is no run for. Reading answers the `output` event.
    pub fn read(&self, id: u64) -> Option<String> {
        let state = self.runs.get(&id)?;
        let mut state = state.lock().ok()?;
        state.owed = false;
        let running = state.finished.is_none();
        let mut answer = match state.transcript.take() {
            Some(changes) => changes_json(&changes),
            None => serde_json::json!({"first": state.transcript.lines().len(), "reset": false, "lines": []}),
        };
        answer["running"] = running.into();
        answer["command"] = state.command.clone().into();
        Some(answer.to_string())
    }

    /// Sends a line to the run's input, and shows it in the output so the
    /// panel reads like the exchange it was. False when the run is over
    /// or its input closed.
    pub fn write(&self, id: u64, line: &str) -> bool {
        let Some(state) = self.runs.get(&id) else { return false };
        let Ok(mut state) = state.lock() else { return false };
        let Some(stdin) = state.stdin.as_mut() else { return false };
        let sent = stdin.write_all(format!("{line}\n").as_bytes()).and_then(|_| stdin.flush()).is_ok();
        if sent {
            state.transcript.note(&format!("› {line}"));
            state.last_output = Instant::now();
            state.quiet_said = false;
            if !state.owed {
                state.owed = true;
                let _ = self.sender.send(Event::Run { id, status: "output".into() });
            }
        }
        sent
    }

    /// Asks the run to stop — an interrupt to its whole process group —
    /// and makes it a moment later if it has not. False when it is over.
    pub fn stop(&self, id: u64) -> bool {
        let Some(state) = self.runs.get(&id) else { return false };
        let group = {
            let Ok(state) = state.lock() else { return false };
            if state.finished.is_some() {
                return false;
            }
            state.group
        };
        signal_group(group, Signal::Interrupt);
        let state = Arc::clone(state);
        thread::spawn(move || {
            thread::sleep(GRACE);
            let running = state.lock().map(|state| state.finished.is_none()).unwrap_or(false);
            if running {
                signal_group(group, Signal::Kill);
            }
        });
        true
    }

    /// Whether the run is still going.
    pub fn running(&self, id: u64) -> bool {
        self.runs
            .get(&id)
            .and_then(|state| state.lock().ok().map(|state| state.finished.is_none()))
            .unwrap_or(false)
    }

    /// The place after (`forward`) or before the line and character
    /// given in the run's output, round the end.
    pub fn neighbouring_place(&self, id: u64, from: Option<(usize, usize)>, forward: bool) -> Option<(usize, Place)> {
        let state = self.runs.get(&id)?.lock().ok()?;
        state.transcript.neighbouring_place(from, forward).map(|(line, place)| (line, place.clone()))
    }

    /// Lets go of a run that is over; a running one is stopped first.
    pub fn forget(&mut self, id: u64) {
        if self.running(id) {
            self.stop(id);
        }
        self.runs.remove(&id);
    }
}

impl Drop for Runs {
    /// Nothing the editor started outlives it.
    fn drop(&mut self) {
        for state in self.runs.values() {
            if let Ok(state) = state.lock() {
                if state.finished.is_none() {
                    signal_group(state.group, Signal::Kill);
                }
            }
        }
    }
}

fn spawn(command: &str, cwd: &Path) -> std::io::Result<Child> {
    use std::os::unix::process::CommandExt;
    let mut process = Command::new("/bin/sh");
    process
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A group of its own, so stopping it reaches everything it
        // started and nothing of the editor's.
        .process_group(0);
    for (name, value) in ENVIRONMENT {
        if value.is_empty() {
            process.env_remove(name);
        } else {
            process.env(name, value);
        }
    }
    process.spawn()
}

fn read_into(mut stream: Box<dyn Read + Send>, state: &Mutex<State>, sender: &EventSender, id: u64) {
    let mut buffer = [0u8; 8192];
    loop {
        let count = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(count) => count,
        };
        let Ok(mut state) = state.lock() else { return };
        state.transcript.feed(&buffer[..count]);
        state.last_output = Instant::now();
        state.quiet_said = false;
        // One event until the shell reads: a command writing fast is
        // read in as few turns as the shell can manage.
        if !state.owed {
            state.owed = true;
            let _ = sender.send(Event::Run { id, status: "output".into() });
        }
    }
}

fn watch_for_quiet(state: &Mutex<State>, sender: &EventSender, id: u64) {
    loop {
        thread::sleep(Duration::from_millis(500));
        let Ok(mut state) = state.lock() else { return };
        if state.finished.is_some() {
            return;
        }
        if !state.quiet_said && state.last_output.elapsed() >= QUIET_AFTER && state.stdin.is_some() {
            state.quiet_said = true;
            let _ = sender.send(Event::Run { id, status: "quiet".into() });
        }
    }
}

enum Signal {
    Interrupt,
    Kill,
}

fn signal_group(group: i32, signal: Signal) {
    let signal = match signal {
        Signal::Interrupt => libc::SIGINT,
        Signal::Kill => libc::SIGKILL,
    };
    // SAFETY: kill with a negative pid signals a process group; it reads
    // no memory and is safe whatever the group is.
    unsafe {
        libc::kill(-group, signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn wait_for(receiver: &mpsc::Receiver<Event>, wanted: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(Event::Run { status, .. }) = receiver.recv_timeout(Duration::from_millis(200)) {
                if status == wanted {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn a_command_runs_with_colour_asked_for_and_its_output_read() {
        let (sender, receiver) = mpsc::channel();
        let mut runs = Runs::new(sender);
        let id = runs.start("printf 'one\\n'; echo $CARGO_TERM_COLOR >&2; exit 3", Path::new("/")).unwrap();
        assert!(wait_for(&receiver, "exited"));
        let read: serde_json::Value = serde_json::from_str(&runs.read(id).unwrap()).unwrap();
        let texts: Vec<&str> = read["lines"].as_array().unwrap().iter().filter_map(|line| line["text"].as_str()).collect();
        assert!(texts.contains(&"one"));
        assert!(texts.contains(&"always"), "colour asked for: {texts:?}");
        assert!(texts.iter().any(|text| text.contains('3')), "the exit is noted: {texts:?}");
        assert_eq!(read["running"], false);
        assert!(!runs.running(id));
    }

    #[test]
    fn a_line_sent_reaches_its_input() {
        let (sender, receiver) = mpsc::channel();
        let mut runs = Runs::new(sender);
        let id = runs.start("read name; echo \"hello $name\"", Path::new("/")).unwrap();
        assert!(runs.write(id, "world"));
        assert!(wait_for(&receiver, "exited"));
        let read = runs.read(id).unwrap();
        assert!(read.contains("hello world"), "{read}");
        assert!(read.contains("› world"), "what was sent is shown: {read}");
        assert!(!runs.write(id, "late"), "nothing goes to a finished run");
    }

    #[test]
    fn stopping_takes_down_what_the_command_started() {
        let (sender, receiver) = mpsc::channel();
        let mut runs = Runs::new(sender);
        // The shell starts a child that would outlive a plain kill of
        // the shell.
        let id = runs.start("sleep 30 & wait", Path::new("/")).unwrap();
        thread::sleep(Duration::from_millis(200));
        let started = Instant::now();
        assert!(runs.stop(id));
        assert!(wait_for(&receiver, "exited"));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!runs.stop(id), "a run that is over cannot be stopped again");
    }

    #[test]
    fn a_command_that_cannot_start_says_why() {
        let (sender, _receiver) = mpsc::channel();
        let mut runs = Runs::new(sender);
        let error = runs.start("true", Path::new("/no/such/dir")).unwrap_err();
        assert!(error.contains("true"), "{error}");
    }
}
