//! End-to-end client test against the scripted server in
//! `scripts/fake_lsp.py`: spawn, handshake, per-project instances,
//! diagnostics on open and change, clean shutdown.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use textchum_core::Event;
use textchum_lsp::{Pool, ServerConfig};

fn fake_server_config() -> ServerConfig {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/fake_lsp.py")
        .canonicalize()
        .expect("fake server script exists");
    ServerConfig {
        id: "fake".into(),
        command: "python3".into(),
        args: vec![script.to_string_lossy().into_owned()],
        languages: vec!["rust".into()],
        install_hint: "n/a".into(),
    }
}

/// Creates a marker-bearing project directory with one rust file.
fn project(name: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir()
        .join(format!("textchum-lsp-{}", std::process::id()))
        .join(name);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "").unwrap();
    let file = root.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    (root.canonicalize().unwrap(), file.canonicalize().unwrap())
}

/// Collects events until `predicate`, applied to everything received so
/// far, is satisfied. Events arrive from independent server instances in
/// arbitrary order, so waiting must accumulate rather than discard.
fn collect_until(
    events: &mpsc::Receiver<Event>,
    what: &str,
    seen: &mut Vec<Event>,
    predicate: impl Fn(&[Event]) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !predicate(seen) {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .unwrap_or_else(|| panic!("timed out waiting for {what}; saw {seen:?}"));
        let event = events
            .recv_timeout(remaining)
            .unwrap_or_else(|_| panic!("timed out waiting for {what}; saw {seen:?}"));
        seen.push(event);
    }
}

#[test]
fn per_project_instances_and_diagnostics() {
    let (tx, events) = mpsc::channel();
    let mut pool = Pool::new(tx);
    pool.add_override(fake_server_config());

    let (root_a, file_a) = project("proj-a");
    let (root_b, file_b) = project("proj-b");

    // Opening files from two projects spawns two independent instances.
    pool.did_open(&file_a, "rust", "fn main() {}\n");
    pool.did_open(&file_b, "rust", "fn main() {}\n");

    let mut seen = Vec::new();
    let is_running = |events: &[Event], root: &PathBuf| {
        events.iter().any(|event| {
            matches!(event, Event::ServerStatus { status, root: r, .. }
                if status == "running" && PathBuf::from(r) == *root)
        })
    };
    collect_until(&events, "both instances running", &mut seen, |seen| {
        is_running(seen, &root_a) && is_running(seen, &root_b)
    });
    let mut running = pool.running();
    running.sort();
    assert_eq!(running.len(), 2, "one instance per project: {running:?}");
    assert_ne!(running[0].1, running[1].1, "distinct roots");

    // Each open produced a diagnostic for its own file.
    let has_finding = |events: &[Event], file: &PathBuf, marker: &str| {
        events.iter().any(|event| {
            matches!(event, Event::Diagnostics { path, json }
                if PathBuf::from(path) == *file && json.contains(marker))
        })
    };
    collect_until(&events, "open diagnostics for both files", &mut seen, |seen| {
        has_finding(seen, &file_a, "fake finding #1") && has_finding(seen, &file_b, "fake finding #1")
    });

    // A change produces fresh diagnostics (the fake counts requests).
    pool.did_change(&file_a, "fn main() { broken }\n");
    collect_until(&events, "change diagnostics", &mut seen, |seen| {
        has_finding(seen, &file_a, "#2")
    });
    assert!(
        seen.iter().any(|event| matches!(event, Event::Diagnostics { json, .. }
            if json.contains("\"severity\":1"))),
        "severity carried through"
    );

    // Hover: the response comes back tagged with the request id.
    let request_id = pool.hover(&file_b, 0, 3);
    assert!(request_id > 2, "real request ids start above lifecycle ids");
    collect_until(&events, "hover response", &mut seen, |seen| {
        seen.iter().any(|event| matches!(event, Event::LspResponse { id, json }
            if *id == request_id && json.contains("fake hover at 0:3")))
    });
    assert_eq!(pool.hover(std::path::Path::new("/not/open.rs"), 0, 0), 0);

    // Definition: locations come back with the file's own uri.
    let definition_id = pool.definition(&file_b, 0, 5);
    collect_until(&events, "definition response", &mut seen, |seen| {
        seen.iter().any(|event| matches!(event, Event::LspResponse { id, json }
            if *id == definition_id
                && json.contains("\"character\":3")
                && json.contains("file://")))
    });

    pool.did_close(&file_a);
    // Dropping the pool must shut both instances down without hanging
    // (the test itself would time out otherwise).
    drop(pool);
}

#[test]
fn configured_servers_win_per_project_then_default() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/fake_lsp.py")
        .canonicalize()
        .unwrap();
    let (root_a, file_a) = project("cfg-a");
    let (root_b, file_b) = project("cfg-b");

    let (tx, events) = mpsc::channel();
    let mut pool = Pool::new(tx);
    // Default: a command that does not exist. Project A: the fake server.
    // Project B falls back to the broken default and reports not-found;
    // project A runs — proving the per-project entry wins.
    pool.configure(&format!(
        r#"{{
            "defaults": {{"rust": "definitely-not-a-real-binary-xyz"}},
            "projects": {{"{}": {{"rust": "python3 {}"}}}}
        }}"#,
        root_a.display(),
        script.display()
    ));

    pool.did_open(&file_a, "rust", "fn main() {}\n");
    pool.did_open(&file_b, "rust", "fn main() {}\n");

    let mut seen = Vec::new();
    collect_until(&events, "custom running + default not-found", &mut seen, |seen| {
        let a_running = seen.iter().any(|e| {
            matches!(e, Event::ServerStatus { status, server, root, .. }
                if status == "running" && server.starts_with("custom:python3")
                    && PathBuf::from(root) == root_a)
        });
        let b_missing = seen.iter().any(|e| {
            matches!(e, Event::ServerStatus { status, root, .. }
                if status == "not-found" && PathBuf::from(root) == root_b)
        });
        a_running && b_missing
    });

    // The configured instance serves real traffic.
    collect_until(&events, "diagnostics from configured server", &mut seen, |seen| {
        seen.iter().any(|e| matches!(e, Event::Diagnostics { path, .. }
            if PathBuf::from(path) == file_a))
    });
}

#[test]
fn missing_server_reports_once_with_hint() {
    let (tx, events) = mpsc::channel();
    let mut pool = Pool::new(tx);
    pool.add_override(ServerConfig {
        id: "ghost".into(),
        command: "definitely-not-a-real-binary-xyz".into(),
        args: vec![],
        languages: vec!["rust".into()],
        install_hint: "cargo install ghost".into(),
    });
    let (_root, file) = project("proj-ghost");
    pool.did_open(&file, "rust", "");
    let mut seen = Vec::new();
    collect_until(&events, "not-found status", &mut seen, |seen| {
        seen.iter()
            .any(|event| matches!(event, Event::ServerStatus { status, .. } if status == "not-found"))
    });
    assert!(
        seen.iter().any(|event| matches!(event, Event::ServerStatus { message, .. }
            if message.contains("cargo install ghost"))),
        "install hint carried: {seen:?}"
    );

    // A second open is silently ignored, not retried.
    pool.did_open(&file, "rust", "");
    assert!(
        events.recv_timeout(Duration::from_millis(300)).is_err(),
        "no repeat report"
    );
}
