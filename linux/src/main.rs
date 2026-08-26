//! Textchum's Linux shell: GTK4 + libadwaita over the same Rust core the
//! macOS app uses — linked as a crate rather than through the C ABI,
//! since both sides are Rust. The architecture is identical: the core
//! owns every document, the shell owns presentation, and every edit
//! funnels through one choke point.

mod ctags;
mod lsp_edits;
mod page;
mod path_actions;
mod preprocessors;
mod spell;
mod session;
mod shell;
mod workbench;

use adw::prelude::*;
use gtk::gio;
use workbench::Workbench;

const APP_ID: &str = "to.perri.textchum";

fn main() -> gtk::glib::ExitCode {
    let smoke_test = std::env::args().any(|argument| argument == "--smoke-test");
    let fresh = std::env::args().any(|argument| argument == "--fresh");
    // --wait (the GIT_EDITOR contract): a private instance that stays
    // in the foreground until its windows close, instead of handing
    // the file to the running one and returning immediately. Waiting
    // implies not restoring the whole session into git's editor.
    let wait = std::env::args().any(|argument| argument == "--wait");
    let fresh = fresh || wait;

    let mut flags = gio::ApplicationFlags::HANDLES_OPEN;
    if wait {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let app = adw::Application::builder()
        .application_id(APP_ID)
        // GApplication gives single-instance + open-files over D-Bus for
        // free — this is what the textchum:// scheme hand-rolls on macOS.
        .flags(flags)
        .build();

    app.connect_activate(move |app| {
        if smoke_test {
            let code = run_smoke_test(app);
            std::process::exit(code);
        }
        let workbench = Workbench::new(app);
        // The saved session comes back unless --fresh says otherwise;
        // an empty (or absent) session still deserves a tab.
        let restored = if fresh { 0 } else { session::restore(&workbench) };
        if restored == 0 {
            workbench.open(None, None);
        }
        workbench.window.present();
    });
    app.connect_shutdown(|_| session::save());
    app.connect_open(|app, files, _hint| {
        let workbench = Workbench::active().unwrap_or_else(|| Workbench::new(app));
        // `+12` before a file jumps to that line, the editor-CLI
        // convention (chum +12 notes.md). Such an argument arrives as
        // a GFile whose basename is the +number.
        let mut pending_line: Option<i32> = None;
        for file in files {
            let Some(path) = file.path() else { continue };
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(digits) = name.strip_prefix('+') {
                if let Ok(line) = digits.parse::<i32>() {
                    if !path.is_file() {
                        pending_line = Some((line - 1).max(0));
                        continue;
                    }
                }
            }
            let at = pending_line.take().map(|line| (line, 0));
            workbench.open(Some(path), at);
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
    app.set_accels_for_action("win.preview", &["<Ctrl><Alt>p"]);
    app.set_accels_for_action("win.open", &["<Ctrl>o"]);
    app.set_accels_for_action("win.save", &["<Ctrl>s"]);
    app.set_accels_for_action("win.save-as", &["<Ctrl><Shift>s"]);
    app.set_accels_for_action("win.undo", &["<Ctrl>z"]);
    app.set_accels_for_action("win.redo", &["<Ctrl><Shift>z"]);
    app.set_accels_for_action("win.preferences", &["<Ctrl>comma"]);
    app.set_accels_for_action("win.close-tab", &["<Ctrl>w"]);
    app.set_accels_for_action("window.close", &["<Ctrl><Shift>w"]);
    app.set_accels_for_action("win.back", &["<Alt>Left"]);
    app.set_accels_for_action("win.forward", &["<Alt>Right"]);
    app.set_accels_for_action("win.references", &["<Shift>F12"]);
    app.set_accels_for_action("win.rename", &["F2"]);
    app.set_accels_for_action("win.format", &["<Ctrl><Shift>i"]);
    app.set_accels_for_action("win.outline", &["<Ctrl><Shift>o"]);
    app.set_accels_for_action("win.revert", &["<Ctrl><Alt>r"]);
    app.set_accels_for_action("win.redraw", &["<Ctrl><Alt>l"]);
    app.set_accels_for_action("win.hover", &["<Ctrl><Alt>h"]);
    app.set_accels_for_action("win.preprocess", &["<Ctrl><Alt>f"]);
    app.set_accels_for_action("win.palette", &["<Ctrl><Shift>p"]);
    app.set_accels_for_action("win.new-format-picker", &["<Ctrl><Shift>n"]);
    app.set_accels_for_action("win.paths", &["<Ctrl><Alt>t"]);
    app.set_accels_for_action("win.block-start", &["<Ctrl><Alt>Up"]);
    app.set_accels_for_action("win.block-end", &["<Ctrl><Alt>Down"]);
    // Key overrides read the configuration, which touches GTK-backed
    // state — so they wait for startup, after GTK initializes. The
    // config watcher starts here too: external edits to config.json
    // apply while running, through the same pipeline a Preferences
    // change uses.
    app.connect_startup(|app| {
        apply_key_overrides(app);
        let app = app.clone();
        shell::Shell::instance().watch_config(move || {
            apply_key_overrides(&app);
            let shell = shell::Shell::instance();
            let config = shell.config.borrow();
            let font_size = config.font_size();
            let tab_width = config.tab_width();
            let line_numbers = config.line_numbers();
            drop(config);
            workbench::apply_editor_look(font_size, tab_width, line_numbers);
            workbench::Workbench::for_each(|workbench| {
                for page in workbench.all_pages() {
                    spell::run(&page);
                }
                workbench.refresh_chrome();
            });
        });
    });

    // GApplication consumes argv; strip our own flags so it does not
    // try to open files named after them.
    let arguments: Vec<String> = std::env::args()
        .filter(|argument| {
            argument != "--smoke-test" && argument != "--fresh" && argument != "--wait"
        })
        .collect();
    app.run_with_args(&arguments)
}

/// Applies the configuration's `keys` section — the same action names
/// and `modifiers+key` specs the macOS shell understands, mapped onto
/// this shell's actions (`cmd` means the primary modifier, so it lands
/// on Ctrl here).
fn apply_key_overrides(app: &adw::Application) {
    let keys_json = shell::Shell::instance().config.borrow().keys_json();
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&keys_json) else {
        return;
    };
    let action_for = |name: &str| -> Option<&'static str> {
        Some(match name {
            "new" => "win.new-tab",
            "open" => "win.open",
            "openQuickly" => "win.quick-open",
            "save" => "win.save",
            "saveAs" => "win.save-as",
            "revertToSaved" => "win.revert",
            "close" => "win.close-tab",
            "undo" => "win.undo",
            "redo" => "win.redo",
            "find" => "win.find",
            "findInProject" => "win.find-in-project",
            "jumpToDefinition" => "win.definition",
            "goBack" => "win.back",
            "goForward" => "win.forward",
            "findReferences" => "win.references",
            "renameSymbol" => "win.rename",
            "formatDocument" => "win.format",
            "documentOutline" => "win.outline",
            "redraw" => "win.redraw",
            "showHover" => "win.hover",
            "runPreprocessors" => "win.preprocess",
            "commandPalette" => "win.palette",
            "newWithFormat" => "win.new-format-picker",
            "togglePathDisplay" => "win.paths",
            "goToBlockStart" => "win.block-start",
            "goToBlockEnd" => "win.block-end",
            "toggleNavigator" => "win.sidebar",
            "togglePreview" => "win.preview",
            "settings" => "win.preferences",
            _ => return None,
        })
    };
    for (name, spec) in parsed.as_object().into_iter().flatten() {
        let Some(action) = action_for(name) else {
            eprintln!("textchum: keys: no such action on this platform: {name}");
            continue;
        };
        let Some(spec) = spec.as_str() else { continue };
        let Some(accel) = accel_from_spec(spec) else {
            eprintln!("textchum: keys: could not parse {spec:?} for {name}");
            continue;
        };
        app.set_accels_for_action(action, &[&accel]);
    }
}

/// "cmd+shift+g" → "<Ctrl><Shift>g". `cmd` and `ctrl` both mean Ctrl
/// here; `alt` is Alt. Named keys use their GDK names.
fn accel_from_spec(spec: &str) -> Option<String> {
    let mut modifiers = String::new();
    let mut key: Option<String> = None;
    for part in spec.split('+') {
        match part.trim().to_lowercase().as_str() {
            "cmd" | "ctrl" | "control" => {
                if !modifiers.contains("<Ctrl>") {
                    modifiers.push_str("<Ctrl>");
                }
            }
            "shift" => modifiers.push_str("<Shift>"),
            "alt" | "option" => modifiers.push_str("<Alt>"),
            "up" => key = Some("Up".into()),
            "down" => key = Some("Down".into()),
            "left" => key = Some("Left".into()),
            "right" => key = Some("Right".into()),
            "return" | "enter" => key = Some("Return".into()),
            "escape" => key = Some("Escape".into()),
            "space" => key = Some("space".into()),
            "tab" => key = Some("Tab".into()),
            "delete" => key = Some("Delete".into()),
            other if other.chars().count() == 1 => key = Some(other.to_owned()),
            _ => return None,
        }
    }
    key.map(|key| format!("{modifiers}{key}"))
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
    // The markdown tab carries a preview paned; the rust one does not.
    let markdown_child = workbench.tab_view.nth_page(1).child();
    if markdown_child.downcast_ref::<gtk::Paned>().is_none() {
        eprintln!("FAIL: markdown tab has no preview paned");
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

        // Formatting: the scripted server prepends "formatted: " as a
        // TextEdit; the action must carry it through the buffer (and
        // therefore the core) — proving the whole edits pipeline.
        if !fire("win.format") {
            eprintln!("FAIL: format action");
            return 1;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            context.iteration(true);
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
            if text.starts_with("formatted: ") {
                break;
            }
            if std::time::Instant::now() > deadline {
                eprintln!("FAIL: formatting edit never landed");
                return 1;
            }
        }
        println!("gtk smoke test passed (with language server)");
    } else {
        println!("gtk smoke test passed (no fake server available)");
    }

    // External changes follow the disk while the buffer is clean. Any
    // formatting edit above left it dirty, so save first; the app's
    // own save must be ignored, so wait out the suppression window.
    {
        if !fire("win.save") {
            eprintln!("FAIL: save before watch test");
            return 1;
        }
        let context = glib::MainContext::default();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(2200);
        while std::time::Instant::now() < until {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let external = "fn main() {}\n// rewritten outside\n";
        if std::fs::write(&path, external).is_err() {
            eprintln!("FAIL: external rewrite");
            return 1;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            context.iteration(true);
            if buffer.text(&buffer.start_iter(), &buffer.end_iter(), true) == external {
                break;
            }
            if std::time::Instant::now() > deadline {
                eprintln!("FAIL: external change never reached the buffer");
                return 1;
            }
        }
    }

    // Spell check: with a dictionary on hand, a typo in the markdown
    // document gets the misspell tag — and only prose does.
    let have_hunspell = std::process::Command::new("hunspell")
        .arg("-v")
        .output()
        .is_ok();
    if have_hunspell {
        let second_key = second.to_string_lossy().into_owned();
        let md_page = workbench
            .all_pages()
            .into_iter()
            .find(|candidate| candidate.path.borrow().as_deref() == Some(second_key.as_str()));
        if let Some(md_page) = md_page {
            shell::Shell::instance()
                .config
                .borrow_mut()
                .set_spell_language(Some("en_US"));
            {
                let buffer = &md_page.buffer;
                let mut end = buffer.end_iter();
                buffer.insert(&mut end, "\nthis sentense is wrogn\n");
            }
            spell::run(&md_page);
            let buffer = &md_page.buffer;
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), true)
                .to_string();
            let target = text.find("sentense").map(|byte| {
                text[..byte].chars().count() as i32
            });
            let tagged = target.is_some_and(|offset| {
                buffer
                    .iter_at_offset(offset)
                    .tags()
                    .iter()
                    .any(|tag| tag.name().as_deref() == Some(spell::TAG))
            });
            if !tagged {
                eprintln!("FAIL: misspelling was not tagged");
                return 1;
            }
            shell::Shell::instance().config.borrow_mut().set_spell_language(None);
        }
    } else {
        eprintln!("note: hunspell not installed; spell smoke skipped");
    }

    // Ctags: with Universal Ctags on hand, the index finds a definition
    // in a plain directory.
    if ctags::available() {
        let project = directory.join("ctagsproj");
        let _ = std::fs::create_dir_all(&project);
        let _ = std::fs::write(
            project.join("lib.py"),
            "def frobnicate():\n    return 1\n",
        );
        match ctags::definition("frobnicate", &project) {
            Some((path, line)) if path.ends_with("lib.py") && line == 0 => {}
            other => {
                eprintln!("FAIL: ctags definition wrong: {other:?}");
                return 1;
            }
        }
    } else {
        eprintln!("note: Universal Ctags not installed; ctags smoke skipped");
    }

    // Save As on an untitled document: the new extension brings its
    // language, colors, and shell registration along (issue #2).
    {
        workbench.open(None, None);
        let untitled = workbench.selected().expect("untitled page selected");
        if untitled.path.borrow().is_some() {
            eprintln!("FAIL: fresh tab unexpectedly has a path");
            return 1;
        }
        {
            let buffer = &untitled.buffer;
            let mut end = buffer.end_iter();
            buffer.insert(&mut end, "fn saved_as() {}\n");
        }
        let target = directory.join("gained.rs");
        if !workbench::save_page_as(&workbench, &untitled, &target) {
            eprintln!("FAIL: save_page_as failed");
            return 1;
        }
        if untitled.state.borrow().document.language_name() != Some("rust") {
            eprintln!("FAIL: save-as did not detect the language");
            return 1;
        }
        if untitled.buffer.iter_at_offset(0).tags().is_empty() {
            eprintln!("FAIL: save-as did not repaint highlighting");
            return 1;
        }
        let key = target.to_string_lossy().into_owned();
        if !shell::Shell::instance().pages.borrow().contains_key(&key) {
            eprintln!("FAIL: save-as did not register shell handles");
            return 1;
        }
    }

    // The session file records the open documents.
    session::save();
    match std::fs::read_to_string(session::session_path()) {
        Ok(session_json) if session_json.contains("smoke.rs") => {}
        _ => {
            eprintln!("FAIL: session file missing the open document");
            return 1;
        }
    }
    let _ = std::fs::remove_dir_all(&directory);
    0
}
