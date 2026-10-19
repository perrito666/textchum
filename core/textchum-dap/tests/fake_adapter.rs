//! End-to-end client test against the scripted adapter in
//! `scripts/fake_dap.py`: handshake, breakpoints, a stop with its stack,
//! variables and their children, a step, an end with its exit code, and
//! a refused launch.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};
use textchum_core::Event;
use textchum_dap::{Command, Launch, Session};

fn fake(arguments: Value, breakpoints: Vec<(PathBuf, Vec<usize>)>) -> Launch {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/fake_dap.py")
        .canonicalize()
        .expect("fake adapter script exists");
    Launch {
        adapter: vec!["python3".into(), script.to_string_lossy().into_owned()],
        adapter_id: "fake".into(),
        arguments,
        cwd: std::env::temp_dir(),
        breakpoints,
        install: "nothing to install".into(),
        listens: false,
    }
}

/// Waits for the session's next event of `kind`, keeping the others.
fn next(events: &mpsc::Receiver<Event>, kind: &str, seen: &mut Vec<(String, Value)>) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(at) = seen.iter().position(|(seen_kind, _)| seen_kind == kind) {
            return seen.remove(at).1;
        }
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .unwrap_or_else(|| panic!("timed out waiting for {kind}; saw {seen:?}"));
        match events.recv_timeout(remaining) {
            Ok(Event::Debug { kind, json, .. }) => seen.push((kind, serde_json::from_str(&json).unwrap())),
            Ok(_) => {}
            Err(_) => panic!("timed out waiting for {kind}; saw {seen:?}"),
        }
    }
}

#[test]
fn a_session_stops_at_its_breakpoint_shows_where_and_what_and_steps_to_the_end() {
    let (tx, events) = mpsc::channel();
    let path = PathBuf::from("/project/main.py");
    let session = Session::start(7, fake(json!({"program": "main.py"}), vec![(path.clone(), vec![4])]), tx).unwrap();
    let mut seen = Vec::new();

    let breakpoints = next(&events, "breakpoints", &mut seen);
    assert_eq!(breakpoints["breakpoints"][0], json!({"line": 4, "verified": true, "message": null}));
    let output = next(&events, "output", &mut seen);
    assert_eq!(output["text"], "hello from the program\n");
    let stopped = next(&events, "stopped", &mut seen);
    assert_eq!(stopped["reason"], "breakpoint");
    assert_eq!(stopped["frames"][0]["line"], 4, "zero-based, as the editor counts");
    assert_eq!(stopped["frames"][0]["path"], "/project/main.py");
    assert_eq!(stopped["frames"].as_array().map(Vec::len), Some(2));

    // The variables of the top frame, by scope; the expensive scope is
    // not asked for until someone opens it.
    assert!(session.send(Command::Variables { frame: 1 }));
    let variables = next(&events, "variables", &mut seen);
    assert_eq!(variables["scopes"].as_array().map(Vec::len), Some(1));
    assert_eq!(variables["scopes"][0]["variables"][1]["name"], "shop");
    let reference = variables["scopes"][0]["variables"][1]["reference"].as_i64().unwrap();
    assert!(session.send(Command::Children { reference }));
    let children = next(&events, "children", &mut seen);
    assert_eq!(children["variables"][0]["value"], "\"corner\"");

    assert!(session.send(Command::Evaluate { frame: 1, expression: "count + 1".into() }));
    assert_eq!(next(&events, "evaluated", &mut seen)["result"], "2");

    assert!(session.send(Command::Next));
    let stepped = next(&events, "stopped", &mut seen);
    assert_eq!((stepped["reason"].as_str(), stepped["frames"][0]["line"].as_u64()), (Some("step"), Some(5)));

    assert!(session.send(Command::Continue));
    assert_eq!(next(&events, "exited", &mut seen)["exitCode"], 0);
    next(&events, "ended", &mut seen);
    assert!(
        !seen.iter().any(|(kind, body)| kind == "output" && body["category"] == "telemetry"),
        "telemetry is not for the console"
    );
    drop(session);
}

#[test]
fn a_refused_launch_says_why() {
    let (tx, events) = mpsc::channel();
    let session = Session::start(8, fake(json!({"program": "fail"}), vec![]), tx).unwrap();
    let mut seen = Vec::new();
    let failed = next(&events, "failed", &mut seen);
    assert_eq!(failed["message"], "no such program: fail");
    drop(session);
}

#[test]
fn an_adapter_that_is_not_there_names_what_installs_it() {
    let (tx, _events) = mpsc::channel();
    let mut debuggers = textchum_dap::Debuggers::new(tx);
    let error = debuggers.start(&json!({"adapter": "nothing"}), std::path::Path::new("/"), vec![]).unwrap_err();
    assert!(error.contains("test or a main"), "{error}");
}

/// Against the real lldb-dap and cargo, which CI does not have: run with
/// `cargo test -p textchum-dap -- --ignored` where both are installed.
#[test]
#[ignore]
fn a_rust_test_is_built_and_stopped_in_by_lldb_dap() {
    let package = std::env::temp_dir().join(format!("textchum-dap-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&package);
    std::fs::create_dir_all(package.join("src")).unwrap();
    std::fs::write(package.join("Cargo.toml"), "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let source = package.join("src/main.rs");
    std::fs::write(
        &source,
        "fn main() {}\n\n#[test]\nfn adds() {\n    let count = 41;\n    let total = count + 1;\n    assert_eq!(total, 42);\n}\n",
    )
    .unwrap();
    let launch = json!({
        "adapter": "lldb",
        "cargo": ["test", "--no-run", "--message-format=json"],
        "pick": {"kind": "bin"},
        "args": ["adds", "--exact"],
    });
    let (tx, events) = mpsc::channel();
    let mut debuggers = textchum_dap::Debuggers::new(tx);
    let id = debuggers.start(&launch, &package, vec![(source.clone(), vec![5])]).unwrap();
    let mut seen = Vec::new();
    let stopped = {
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(at) = seen.iter().position(|(kind, _): &(String, Value)| kind == "stopped" || kind == "failed") {
                break seen.remove(at);
            }
            match events.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
                Ok(Event::Debug { kind, json, .. }) => seen.push((kind, serde_json::from_str(&json).unwrap())),
                Ok(_) => {}
                Err(_) => panic!("no stop; saw {seen:?}"),
            }
        }
    };
    assert_eq!(stopped.0, "stopped", "{:?}", stopped.1);
    let frame = &stopped.1["frames"][0];
    assert_eq!(frame["line"], 5, "{frame}");
    assert!(debuggers.send(id, Command::Variables { frame: frame["id"].as_i64().unwrap() }));
    let variables = next(&events, "variables", &mut seen);
    assert!(variables.to_string().contains("\"count\""), "{variables}");
    debuggers.forget(id);
    let _ = std::fs::remove_dir_all(&package);
}

#[test]
fn an_adapter_that_listens_is_reached_over_tcp() {
    let (tx, events) = mpsc::channel();
    let mut launch = fake(json!({"program": "main.py"}), vec![(PathBuf::from("/project/main.py"), vec![1])]);
    launch.adapter.push("--listen".into());
    launch.listens = true;
    let session = Session::start(9, launch, tx).unwrap();
    let mut seen = Vec::new();
    let stopped = next(&events, "stopped", &mut seen);
    assert_eq!(stopped["frames"][0]["line"], 1);
    assert!(session.send(Command::Continue));
    next(&events, "ended", &mut seen);
    drop(session);
}

