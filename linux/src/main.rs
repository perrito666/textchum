//! Textchum's Linux shell: GTK4 + libadwaita over the same Rust core the
//! macOS app uses — linked as a crate rather than through the C ABI,
//! since both sides are Rust. The architecture is identical: the core
//! owns every document, the shell owns presentation, and every edit
//! funnels through one choke point.

mod page;
mod shell;
mod workbench;

use adw::prelude::*;
use gtk::gio;
use workbench::Workbench;

const APP_ID: &str = "to.perri.textchum";

fn main() -> gtk::glib::ExitCode {
    let smoke_test = std::env::args().any(|argument| argument == "--smoke-test");

    let app = adw::Application::builder()
        .application_id(APP_ID)
        // GApplication gives single-instance + open-files over D-Bus for
        // free — this is what the textchum:// scheme hand-rolls on macOS.
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_activate(move |app| {
        if smoke_test {
            let code = run_smoke_test(app);
            std::process::exit(code);
        }
        let workbench = Workbench::new(app);
        workbench.open(None, None);
        workbench.window.present();
    });
    app.connect_open(|app, files, _hint| {
        let workbench = Workbench::active().unwrap_or_else(|| Workbench::new(app));
        for file in files {
            if let Some(path) = file.path() {
                workbench.open(Some(path), None);
            }
        }
        workbench.window.present();
    });

    app.set_accels_for_action("win.new-tab", &["<Ctrl>t"]);
    app.set_accels_for_action("win.new", &["<Ctrl>n"]);
    app.set_accels_for_action("win.find", &["<Ctrl>f"]);
    app.set_accels_for_action("win.find-in-project", &["<Ctrl><Shift>f"]);
    app.set_accels_for_action("win.quick-open", &["<Ctrl>p"]);
    app.set_accels_for_action("win.definition", &["F12"]);
    app.set_accels_for_action("win.sidebar", &["F9"]);
    app.set_accels_for_action("win.open", &["<Ctrl>o"]);
    app.set_accels_for_action("win.save", &["<Ctrl>s"]);
    app.set_accels_for_action("win.save-as", &["<Ctrl><Shift>s"]);
    app.set_accels_for_action("win.undo", &["<Ctrl>z"]);
    app.set_accels_for_action("win.redo", &["<Ctrl><Shift>z"]);
    app.set_accels_for_action("win.preferences", &["<Ctrl>comma"]);
    app.set_accels_for_action("win.close-tab", &["<Ctrl>w"]);
    app.set_accels_for_action("window.close", &["<Ctrl><Shift>w"]);

    // GApplication consumes argv; strip our own flag so it does not try
    // to open a file called --smoke-test.
    let arguments: Vec<String> = std::env::args()
        .filter(|argument| argument != "--smoke-test")
        .collect();
    app.run_with_args(&arguments)
}

/// Headless end-to-end check (run under xvfb in CI): typing through the
/// buffer reaches the core, undo replays, a save round-trips through
/// disk, tabs open, deduplicate, and close — and the language-server
/// path works against the scripted server: diagnostics arrive as
/// squiggle tags and a problem count.
fn run_smoke_test(app: &adw::Application) -> i32 {
    use gtk::glib;

    let directory = std::env::temp_dir().join(format!("textchum-gtk-{}", std::process::id()));
    if std::fs::create_dir_all(&directory).is_err() {
        eprintln!("FAIL: temp dir");
        return 1;
    }
    let path = directory.join("smoke.rs");
    if std::fs::write(&path, "fn main() {}\n").is_err() {
        eprintln!("FAIL: seed file");
        return 1;
    }
    let second = directory.join("second.md");
    let _ = std::fs::write(&second, "# hello\n");

    // Route rust at the scripted server (a repo checkout is present in
    // CI and development alike).
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/fake_lsp.py");
    let have_fake_server = script.exists();
    if have_fake_server {
        shell::Shell::instance()
            .pool
            .borrow_mut()
            .add_override(textchum_lsp::ServerConfig {
                id: "fake".into(),
                command: "python3".into(),
                args: vec![script.to_string_lossy().into_owned()],
                languages: vec!["rust".into()],
                install_hint: "n/a".into(),
            });
    }

    let workbench = Workbench::new(app);
    workbench.open(Some(path.clone()), None);
    workbench.open(Some(second.clone()), None);
    workbench.window.present();
    if workbench.tab_view.n_pages() != 2 {
        eprintln!("FAIL: expected 2 tabs, have {}", workbench.tab_view.n_pages());
        return 1;
    }
    // Re-opening focuses the existing tab instead of duplicating it.
    workbench.open(Some(path.clone()), None);
    if workbench.tab_view.n_pages() != 2 {
        eprintln!("FAIL: reopen duplicated a tab");
        return 1;
    }
    let key = path.to_string_lossy().into_owned();
    let Some(handles) = shell::Shell::instance().pages.borrow().get(&key).cloned() else {
        eprintln!("FAIL: page not registered with the shell");
        return 1;
    };
    let buffer = handles.buffer.clone();

    // Type through the buffer; the signals must carry it into the core.
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, "// typed on linux\n");
    let expected = "fn main() {}\n// typed on linux\n";
    if buffer.text(&buffer.start_iter(), &buffer.end_iter(), true) != expected {
        eprintln!("FAIL: unexpected buffer text");
        return 1;
    }
    let fire = |name: &str| {
        gtk::prelude::WidgetExt::activate_action(&workbench.window, name, None).is_ok()
    };
    if !fire("win.undo") {
        eprintln!("FAIL: undo action");
        return 1;
    }
    if buffer.text(&buffer.start_iter(), &buffer.end_iter(), true) != "fn main() {}\n" {
        eprintln!("FAIL: undo did not replay");
        return 1;
    }
    if !fire("win.redo") || !fire("win.save") {
        eprintln!("FAIL: redo/save actions");
        return 1;
    }
    if std::fs::read_to_string(&path).unwrap_or_default() != expected {
        eprintln!("FAIL: save round trip");
        return 1;
    }
    if buffer.iter_at_offset(0).tags().is_empty() {
        eprintln!("FAIL: no highlight tag at offset 0");
        return 1;
    }

    // Diagnostics from the scripted server, delivered through the pump.
    if have_fake_server {
        let context = glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            context.iteration(true);
            if !handles.problems.borrow().is_empty() {
                break;
            }
            if std::time::Instant::now() > deadline {
                eprintln!("FAIL: no diagnostics arrived");
                return 1;
            }
        }
        let squiggled = buffer
            .iter_at_offset(0)
            .tags()
            .iter()
            .any(|tag| tag.name().is_some_and(|name| name.starts_with("diag-")));
        if !squiggled {
            eprintln!("FAIL: diagnostics did not tag the text");
            return 1;
        }

        // Completion and hover ride the same request/response plumbing.
        use std::cell::RefCell;
        use std::rc::Rc;
        let completion_labels: Rc<RefCell<Option<Vec<String>>>> =
            Rc::new(RefCell::new(None));
        {
            let shell = shell::Shell::instance();
            let id = shell
                .pool
                .borrow_mut()
                .completion(&path, 0, 3);
            let sink = Rc::clone(&completion_labels);
            shell.expect_response(id, move |json| {
                *sink.borrow_mut() = Some(
                    page::parse_completion_items(json)
                        .into_iter()
                        .map(|(label, _)| label)
                        .collect(),
                );
            });
        }
        let hover: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        {
            let shell = shell::Shell::instance();
            let id = shell.pool.borrow_mut().hover(&path, 0, 3);
            let sink = Rc::clone(&hover);
            shell.expect_response(id, move |json| {
                *sink.borrow_mut() = page::hover_text(json);
            });
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            context.iteration(true);
            if completion_labels.borrow().is_some() && hover.borrow().is_some() {
                break;
            }
            if std::time::Instant::now() > deadline {
                eprintln!("FAIL: completion/hover responses did not arrive");
                return 1;
            }
        }
        let labels = completion_labels.borrow().clone().unwrap_or_default();
        if !labels.iter().any(|label| label == "fake_function") {
            eprintln!("FAIL: completion items missing: {labels:?}");
            return 1;
        }
        if !hover.borrow().as_deref().unwrap_or("").contains("fake hover") {
            eprintln!("FAIL: hover text missing");
            return 1;
        }
        println!("gtk smoke test passed (with language server)");
    } else {
        println!("gtk smoke test passed (no fake server available)");
    }
    let _ = std::fs::remove_dir_all(&directory);
    0
}
