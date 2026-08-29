//! Textchum's Linux shell: GTK4 + libadwaita over the same Rust core the
//! macOS app uses — linked as a crate rather than through the C ABI,
//! since both sides are Rust. The architecture is identical: the core
//! owns every document, the shell owns presentation, and every edit
//! funnels through one choke point.

mod ctags;
mod lsp_edits;
mod keyboard;
mod page;
mod paths;
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

/// Set when the WebKit sandbox had to be switched off to start at all,
/// so the first window can say so once.
static SANDBOX_NOTE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Keeps the editor startable where WebKit's sandbox cannot run.
///
/// The Markdown preview is a WebKitWebView, and WebKit sandboxes its
/// processes with bubblewrap. Ubuntu 24.04 ships
/// `kernel.apparmor_restrict_unprivileged_userns=1`, which denies the
/// user namespace bubblewrap needs to a program with no AppArmor
/// profile granting it — so `bwrap` fails, and WebKit treats that as
/// fatal rather than degrading. The editor aborts before drawing
/// anything, over a preview pane the user may never open.
///
/// The probe runs `bwrap` because nothing else predicts it: an ordinary
/// process on the same machine creates a user namespace happily, and
/// only bubblewrap's own attempt to write the uid map is refused. So we
/// ask the exact binary the exact question.
///
/// Switching the sandbox off is a real reduction, and it is the lesser
/// one: the alternative is an editor that does not start. It applies
/// only where the sandbox was already impossible, and it says so. The
/// better answer for a packaged build is an AppArmor profile granting
/// `userns`, which keeps the sandbox — but that needs root, and
/// `make install-linux` deliberately installs into the user's own
/// `~/.local`.
fn accommodate_webkit_sandbox() {
    // Already decided by whoever launched us — theirs to own.
    if std::env::var_os("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS").is_some() {
        return;
    }
    let probe = std::process::Command::new("bwrap")
        .args(["--unshare-user", "--ro-bind", "/", "/", "/bin/true"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match probe {
        // bwrap works: leave the sandbox alone.
        Ok(status) if status.success() => {}
        // bwrap is there and refused. This is the case that aborts.
        Ok(_) => {
            std::env::set_var("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS", "1");
            let _ = SANDBOX_NOTE.set(
                "This system does not allow the sandbox the Markdown preview's \
                 web view normally runs in (Ubuntu 24.04 and later restrict \
                 unprivileged user namespaces), so it has been switched off for \
                 this session — otherwise Textchum could not start at all. The \
                 preview only renders your own documents.",
            );
        }
        // No bwrap to ask. WebKit finds its own sandbox helper or does
        // without one; either way this is not the failure we know how to
        // diagnose, so nothing is changed behind the user's back.
        Err(_) => {}
    }
}

/// Tells the user, once, that the sandbox had to go.
///
/// Called from both entry points: launching with a file goes through
/// `open` and launching without one through `activate`, and opening a
/// file is the common case — a message wired into only one of them is a
/// message most people never see.
fn announce_sandbox_note(workbench: &std::rc::Rc<Workbench>) {
    use std::cell::Cell;
    thread_local! {
        static SAID: Cell<bool> = const { Cell::new(false) };
    }
    let Some(note) = SANDBOX_NOTE.get() else { return };
    if SAID.with(|said| said.replace(true)) {
        return;
    }
    // After a beat: AdwToastOverlay drops a toast handed to it before
    // the window it lives in is on screen.
    let workbench = std::rc::Rc::clone(workbench);
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
        workbench.explain(note)
    });
}

fn main() -> gtk::glib::ExitCode {
    // Before anything creates a web view, and before GTK starts: this
    // works by setting an environment variable WebKit reads once.
    accommodate_webkit_sandbox();
    let smoke_test = std::env::args().any(|argument| argument == "--smoke-test");
    let fresh = std::env::args().any(|argument| argument == "--fresh");
    // --wait (the GIT_EDITOR contract): a private instance that stays
    // in the foreground until its windows close, instead of handing
    // the file to the running one and returning immediately. Waiting
    // implies not restoring the whole session into git's editor.
    let wait = std::env::args().any(|argument| argument == "--wait");
    let fresh = fresh || wait;
    // A run with a profile of its own has to be its own process:
    // handing the files to an instance already running would open them
    // in that instance's profile, and the flag would have done nothing.
    let own_profile = paths::has_data_dir();

    let mut flags = gio::ApplicationFlags::HANDLES_OPEN;
    // The smoke test is non-unique for the same reason --wait is, and
    // for one more: handed to an instance that is already running, it
    // would exit 0 having checked nothing — a green that means the
    // opposite of what it says.
    if wait || smoke_test || own_profile {
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
        announce_sandbox_note(&workbench);
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
        announce_sandbox_note(&workbench);
    });

    install_default_accels(&app);

    // Key overrides read the configuration, which touches GTK-backed
    // state — so they wait for startup, after GTK initializes. The
    // config watcher starts here too: external edits to config.json
    // apply while running, through the same pipeline a Preferences
    // change uses.
    app.connect_startup(|app| {
        // Quitting is the application's business, not a window's: it
        // closes every window, and connect_shutdown saves the session
        // on the way out.
        let quit = gio::SimpleAction::new("quit", None);
        {
            let app = app.clone();
            quit.connect_activate(move |_, _| app.quit());
        }
        app.add_action(&quit);
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
    let mut arguments: Vec<String> = Vec::new();
    let mut rest = std::env::args();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--smoke-test" | "--fresh" | "--wait" => {}
            // Its value goes with it, or GApplication tries to open a
            // file named after the directory.
            "--data-dir" => {
                let _ = rest.next();
            }
            _ => arguments.push(argument),
        }
    }
    app.run_with_args(&arguments)
}

/// Applies the configuration's `keys` section — the same action names
/// and `modifiers+key` specs the macOS shell understands, mapped onto
/// this shell's actions (`cmd` means the primary modifier, so it lands
/// on Ctrl here).
/// Every action with a shortcut, and the one it comes with.
///
/// A keyboard profile names the shortcuts it moves and nothing else, so
/// leaving one — or dropping an override — has to give the original
/// back, and this is where the originals live. `app.quit` is among them:
/// Ctrl+Q is what a GNOME application quits with, and without it the
/// only way out is the window's close button.
static DEFAULT_ACCELS: &[(&str, &str)] = &[
    ("win.new-tab", "<Ctrl>t"),
    ("win.new", "<Ctrl>n"),
    ("win.find", "<Ctrl>f"),
    ("win.find-in-project", "<Ctrl><Shift>f"),
    ("win.quick-open", "<Ctrl>p"),
    ("win.definition", "F12"),
    ("win.sidebar", "F9"),
    ("win.preview", "<Ctrl><Alt>p"),
    ("win.open", "<Ctrl>o"),
    ("win.save", "<Ctrl>s"),
    ("win.save-as", "<Ctrl><Shift>s"),
    ("win.undo", "<Ctrl>z"),
    ("win.redo", "<Ctrl><Shift>z"),
    ("win.preferences", "<Ctrl>comma"),
    ("win.close-tab", "<Ctrl>w"),
    ("win.fold", "<Ctrl>bracketleft"),
    ("win.fold-all", "<Ctrl><Alt>bracketleft"),
    ("win.unfold-all", "<Ctrl>bracketright"),
    ("win.split", "<Ctrl>backslash"),
    ("win.unsplit", "<Ctrl><Shift>backslash"),
    ("win.reopen-tab", "<Ctrl><Shift>t"),
    ("window.close", "<Ctrl><Shift>w"),
    ("app.quit", "<Ctrl>q"),
    ("win.back", "<Alt>Left"),
    ("win.forward", "<Alt>Right"),
    ("win.references", "<Shift>F12"),
    ("win.code-actions", "<Ctrl>period"),
    ("win.rename", "F2"),
    ("win.format", "<Ctrl><Shift>i"),
    ("win.outline", "<Ctrl><Shift>o"),
    ("win.revert", "<Ctrl><Alt>r"),
    ("win.redraw", "<Ctrl><Alt>l"),
    ("win.hover", "<Ctrl><Alt>h"),
    ("win.preprocess", "<Ctrl><Alt>f"),
    ("win.palette", "<Ctrl><Shift>p"),
    ("win.new-format-picker", "<Ctrl><Shift>n"),
    ("win.paths", "<Ctrl><Alt>t"),
    ("win.file-properties", "<Ctrl>i"),
    ("win.goto-line", "<Ctrl>l"),
    ("win.blame", "<Ctrl><Alt>b"),
    ("win.diagnostic", "<Ctrl><Alt>e"),
    ("win.diagnostic-list", "<Ctrl><Shift>e"),
    ("win.block-start", "<Ctrl><Alt>Up"),
    ("win.block-end", "<Ctrl><Alt>Down"),
];

/// The shortcut an action comes with, for a screen that has to say
/// what it would be without a profile.
pub fn default_accel(action: &str) -> Option<&'static str> {
    DEFAULT_ACCELS
        .iter()
        .find(|(name, _)| *name == action)
        .map(|(_, accel)| *accel)
}

/// Puts every action back on the shortcut it comes with.
fn install_default_accels(app: &adw::Application) {
    for (action, accel) in DEFAULT_ACCELS {
        app.set_accels_for_action(action, &[accel]);
    }
}

pub fn apply_key_overrides(app: &adw::Application) {
    // Back to what everything comes with first: a profile that stops
    // naming an action, or an override that is removed, has to give the
    // original shortcut back, and nothing else says what it was.
    install_default_accels(app);
    let shell = shell::Shell::instance();
    let bindings = {
        let config = shell.config.borrow();
        textchum_core::keys::effective(
            &config.keys_profile(),
            &config.key_profiles_json(),
            &config.keys_json(),
        )
    };
    for (name, spec) in &bindings {
        let Some(action) = crate::keyboard::gtk_action(name) else {
            eprintln!("textchum: keys: no such action on this platform: {name}");
            continue;
        };
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
            // Function keys: profiles from other editors lean on them
            // (F12 for a definition, F2 for a rename).
            other
                if other.starts_with('f')
                    && other.len() > 1
                    && other[1..].parse::<u8>().is_ok_and(|n| (1..=20).contains(&n)) =>
            {
                key = Some(other.to_uppercase())
            }
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
    let buffer = handles.document.buffer.clone();

    // Type through the buffer; the signals must carry it into the core.
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, "// typed on linux\n");
    let expected = "fn main() {}\n// typed on linux\n";
    if buffer.text(&buffer.start_iter(), &buffer.end_iter(), true) != expected {
        eprintln!("FAIL: unexpected buffer text");
        return 1;
    }
    // Importing a theme from another editor: the file goes in, a theme
    // of ours comes out in the themes directory, and the core can wear
    // it.
    {
        use textchum_core::theme_import::{import_into, Source};
        let source_dir = directory.join("import-source");
        let themes_dir = directory.join("import-themes");
        let _ = std::fs::create_dir_all(&source_dir);
        let _ = std::fs::write(
            source_dir.join("night.json"),
            r##"{
              // A theme as VS Code writes them, comments and all.
              "name": "Smoke Night",
              "type": "dark",
              "tokenColors": [
                {"scope": "comment", "settings": {"foreground": "#5A6472"}},
                {"scope": "keyword", "settings": {"foreground": "#C678DD"}},
              ]
            }"##,
        );
        let outcome = import_into(&source_dir, Source::VsCode, &themes_dir);
        if !outcome.errors.is_empty() || outcome.written != vec!["Smoke Night".to_string()] {
            eprintln!("FAIL: theme import: {:?} {:?}", outcome.written, outcome.errors);
            return 1;
        }
        let Ok(json) = std::fs::read_to_string(themes_dir.join("Smoke Night.json")) else {
            eprintln!("FAIL: the imported theme was not written where it is looked for");
            return 1;
        };
        let Ok(theme) = textchum_core::theme::Theme::from_json(&json) else {
            eprintln!("FAIL: an imported theme must be one the core can wear");
            return 1;
        };
        textchum_core::theme::set_active(theme);
        // A keyword's colour reaches the kinds of keyword the source
        // never named separately.
        let Some(id) = textchum_core::theme::resolve("conditional") else {
            eprintln!("FAIL: no style for conditional");
            return 1;
        };
        if textchum_core::theme::styles()[id as usize].dark != 0xC678_DDFF {
            eprintln!("FAIL: imported colours did not reach every capture");
            return 1;
        }
        if let Some(default) = textchum_core::theme::Theme::builtin("Textchum") {
            textchum_core::theme::set_active(default);
        }
        println!("theme import ok (VS Code JSON with comments, inherited captures, wearable)");
    }

    // A file-icon pack: the tree asks the core for an image per file,
    // and the answers follow VS Code's order — whole name, then the
    // longest extension, then the language.
    {
        let pack_dir = directory.join("pack");
        let _ = std::fs::create_dir_all(pack_dir.join("icons"));
        for name in ["rust.svg", "docker.svg", "default.svg"] {
            let _ = std::fs::write(
                pack_dir.join("icons").join(name),
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">
                   <rect width="16" height="16" fill="#4488CC"/></svg>"##,
            );
        }
        let _ = std::fs::write(
            pack_dir.join("icons.json"),
            r##"{
              "iconDefinitions": {
                "_rust": {"iconPath": "./icons/rust.svg"},
                "_docker": {"iconPath": "./icons/docker.svg"},
                "_default": {"iconPath": "./icons/default.svg"}
              },
              "fileExtensions": {"rs": "_rust"},
              "fileNames": {"dockerfile": "_docker"},
              "languageIds": {"rust": "_rust"},
              "file": "_default"
            }"##,
        );
        use textchum_core::icons;
        if let Err(error) = icons::set_active_from(&pack_dir.join("icons.json")) {
            eprintln!("FAIL: icon pack did not load: {error}");
            return 1;
        }
        let named = |name: &str| {
            icons::icon_for(name, None, false)
                .and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        };
        if named("main.rs").as_deref() != Some("rust.svg")
            || named("Dockerfile").as_deref() != Some("docker.svg")
            || named("notes.xyz").as_deref() != Some("default.svg")
        {
            eprintln!("FAIL: icon pack lookups");
            return 1;
        }
        icons::clear_active();
        if icons::is_active() || icons::icon_for("main.rs", None, false).is_some() {
            eprintln!("FAIL: clearing the pack must return the tree to system icons");
            return 1;
        }
        println!("icon pack ok (loaded, looked up by name and extension, cleared)");
    }

    // Go to Line reads what people actually type and paste, and
    // resolves it against the document, clamped to what is there.
    {
        use textchum_core::goto;
        let shapes = [
            ("412", 412, 1),
            ("412:8", 412, 8),
            ("src/main.rs:412:8", 412, 8),
            (r"C:\src\main.rs:412:8", 412, 8),
            ("main.rs, line 412", 412, 1),
            ("utf8.rs:12", 12, 1),
        ];
        for (text, line, column) in shapes {
            match goto::parse(text) {
                Some(target) if target.line == line && target.column == column => {}
                other => {
                    eprintln!("FAIL: go-to-line parsing {text:?}: {other:?}");
                    return 1;
                }
            }
        }
        if goto::parse("nowhere").is_some() {
            eprintln!("FAIL: text naming no line must name no line");
            return 1;
        }
        let mut line_doc = textchum_core::Document::new();
        let _ = line_doc.replace_utf16(0, 0, "one\ntwo\nthree");
        if line_doc.len_lines() != 3
            || line_doc.offset_for_line(2, 1) != 4
            || line_doc.offset_for_line(2, 3) != 6
            || line_doc.offset_for_line(2, 99) != 7
            || line_doc.offset_for_line(9999, 1) != 8
        {
            eprintln!("FAIL: go-to-line offsets");
            return 1;
        }
        println!("go to line ok (compiler shapes, drive letters, clamping)");
    }

    // Find References splits its answer: what calls this, then what
    // checks it. Telling them apart is a convention, so the rules are
    // held here — including the ones that must not fire.
    {
        use textchum_core::references::is_test_path;
        let tests = [
            "/p/tests/helpers.rs",
            "/p/spec/models/user_spec.rb",
            "/p/src/__tests__/Button.tsx",
            "/p/src/parser_test.go",
            "/p/src/test_parser.py",
            "/p/src/Button.test.ts",
            "/p/src/ParserTest.java",
            "/p/src/AppTests.swift",
        ];
        let not_tests = [
            "/p/src/main.rs",
            "/p/src/latest.rs",
            "/p/src/protest.go",
            "/p/src/manifest.json",
            "/p/testing-library/index.js",
        ];
        if !tests.iter().all(|path| is_test_path(path))
            || not_tests.iter().any(|path| is_test_path(path))
        {
            eprintln!("FAIL: test-path classification");
            return 1;
        }
        println!("reference split ok (conventions matched, near-misses left alone)");

    // The context menu carries the editor's own commands, each about
    // the character that was clicked rather than about the caret.
    {
        use gtk::prelude::*;
        let offset = 42;
        let menu = crate::page::context_menu(None, false, Some("/p/main.py"), offset);
        let mut labels = Vec::new();
        let mut targets = Vec::new();
        for section in 0..menu.n_items() {
            let Some(items) = menu.item_link(section, gtk::gio::MENU_LINK_SECTION) else {
                continue;
            };
            for at in 0..items.n_items() {
                if let Some(label) = items
                    .item_attribute_value(at, "label", None)
                    .and_then(|value| value.str().map(str::to_owned))
                {
                    labels.push(label);
                }
                if let Some(action) = items
                    .item_attribute_value(at, "target", None)
                    .and_then(|value| value.get::<(String, i32)>())
                {
                    targets.push(action);
                }
            }
        }
        let wanted = ["Jump to Definition", "Blame Line…", "File Properties…"];
        if !wanted.iter().all(|label| labels.iter().any(|had| had == label)) {
            eprintln!("FAIL: context menu is missing items: {labels:?}");
            return 1;
        }
        if targets.is_empty() || targets.iter().any(|(_, at)| *at != offset) {
            eprintln!("FAIL: context commands do not carry the clicked character");
            return 1;
        }
        println!("context menu ok (editor commands, clicked position)");
    }
    }

    // The gutter's git marks: a committed file, edited three ways.
    {
        use textchum_core::changes::{changes_for, ChangeKind};
        let repo = directory.join("gutter");
        let _ = std::fs::create_dir_all(&repo);
        let git = |arguments: &[&str]| {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(arguments)
                .output();
        };
        let tracked = repo.join("thing.txt");
        let _ = std::fs::write(&tracked, "one\ntwo\nthree\nfour\n");
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@e.invalid"]);
        git(&["config", "user.name", "T"]);
        git(&["add", "thing.txt"]);
        git(&["commit", "-qm", "first"]);

        if !changes_for(&tracked, "one\ntwo\nthree\nfour\n").is_empty() {
            eprintln!("FAIL: an unchanged file should carry no marks");
            return 1;
        }
        // "two" edited, "three" deleted, "five" added at the end. The
        // removed mark lands on the line "three" sat above, since a
        // deleted line occupies no place of its own.
        let described: Vec<String> = changes_for(&tracked, "one\nTWO\nfour\nfive\n")
            .into_iter()
            .map(|mark| format!("{}:{}", mark.line, mark.kind.name()))
            .collect();
        if described != ["1:modified", "2:removed", "3:added"] {
            eprintln!("FAIL: gutter marks: {described:?}");
            return 1;
        }
        let _ = ChangeKind::Added;

        // A file with no committed version is not an error, and not
        // marked.
        let untracked = repo.join("never-committed.txt");
        let _ = std::fs::write(&untracked, "hello\n");
        if !changes_for(&untracked, "hello\nworld\n").is_empty() {
            eprintln!("FAIL: an untracked file should carry no marks");
            return 1;
        }
        println!("git gutter ok (marks what changed, silent without a baseline)");
    }

    // Blame: what git knows about one line, asked with the buffer's
    // text so an unsaved edit cannot shift the answer.
    {
        use textchum_core::blame::blame_line;
        let repo = directory.join("blame");
        let _ = std::fs::create_dir_all(&repo);
        let git = |arguments: &[&str]| {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(arguments)
                .output();
        };
        let path = repo.join("thing.txt");
        let _ = std::fs::write(&path, "first\nsecond\n");
        git(&["init", "-q"]);
        git(&["config", "user.email", "ada@example.invalid"]);
        git(&["config", "user.name", "Ada Lovelace"]);
        git(&["add", "thing.txt"]);
        git(&["commit", "-qm", "Add two lines\n\nAnd a reason for them."]);

        // Two lines typed above "second" and not saved: on disk line 4
        // does not exist, in the buffer it is the committed line.
        let buffer = "first\ntyped one\ntyped two\nsecond\n";
        match blame_line(&path, 4, buffer) {
            Ok(blame)
                if !blame.uncommitted
                    && blame.author == "Ada Lovelace"
                    && blame.summary == "Add two lines"
                    && blame.body == "And a reason for them."
                    && blame.commit.len() == 40
                    && blame.renamed_from.is_empty() => {}
            other => {
                eprintln!("FAIL: blame of a committed line: {other:?}");
                return 1;
            }
        }
        match blame_line(&path, 2, buffer) {
            Ok(blame) if blame.uncommitted && blame.commit.is_empty() => {}
            other => {
                eprintln!("FAIL: a typed line should say it is not committed: {other:?}");
                return 1;
            }
        }
        // The caret past the end is answered about the last line there
        // is, and says which line that was.
        match blame_line(&path, 99, buffer) {
            Ok(blame) if blame.line == 4 => {}
            other => {
                eprintln!("FAIL: blame past the end: {other:?}");
                return 1;
            }
        }
        // A file outside a repository says so rather than answering.
        let loose = directory.join("loose.txt");
        let _ = std::fs::write(&loose, "hello\n");
        if blame_line(&loose, 1, "hello\n").is_ok() {
            eprintln!("FAIL: a file outside a repository should be refused, not answered");
            return 1;
        }
        println!("blame ok (buffer-aware, uncommitted lines, past the end, outside a repo)");
    }

    // Backspace in a line's leading spaces takes a whole indent, and
    // one character anywhere else; Tab in the indentation lines up with
    // the block above. It is the position that decides.
    {
        use textchum_core::indent::{aligned_indent, backspace_width};
        if backspace_width("    ", 4) != 4
            || backspace_width("      ", 4) != 2
            || backspace_width("    let x", 4) != 1
            || backspace_width("\t\t", 4) != 1
            || backspace_width("", 4) != 0
        {
            eprintln!("FAIL: backspace indent widths");
            return 1;
        }
        if aligned_indent(Some("        deep()"), "", 4, false) != "        "
            || aligned_indent(Some("    thing()"), "    ", 4, false) != "        "
            || aligned_indent(Some("    thing()"), "  ", 4, false) != "    "
            || aligned_indent(None, "", 4, false) != "    "
            || aligned_indent(Some("\t\tdeep()"), "", 4, true) != "\t\t"
        {
            eprintln!("FAIL: aligned indentation");
            return 1;
        }
        println!("indentation ok (backspace by level, tab aligns with the block above)");
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

    // Snippets: the core expands the body, the buffer inserts what comes
    // back through the ordinary typing path, and the session it starts
    // walks its stops and mirrors the linked ones.
    let Some(snippet_page) = workbench.selected() else {
        eprintln!("FAIL: no selected page for the snippet check");
        return 1;
    };
    let origin = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .encode_utf16()
        .count();
    let origin_chars = buffer.end_iter().offset();
    let expanded = snippet_page
        .state
        .borrow_mut()
        .document
        .expand_snippet(origin, "let ${1:name} = ${1:name}.frob(${2:arg});$0");
    if expanded != "let name = name.frob(arg);" {
        eprintln!("FAIL: snippet expansion: {expanded}");
        return 1;
    }
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, &expanded);
    let first = snippet_page.state.borrow_mut().document.begin_snippet(origin);
    match first {
        Some(region) if (region.start, region.end) == (origin + 4, origin + 8) => {}
        other => {
            eprintln!("FAIL: snippet did not start on its first placeholder: {other:?}");
            return 1;
        }
    }

    // Type over the first stop; its twin follows.
    let placeholder_start = buffer.iter_at_offset(
        page::char_offset(
            &buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
            origin + 4,
        ),
    );
    let mut from = placeholder_start;
    let mut to = buffer.iter_at_offset(from.offset() + 4);
    buffer.delete(&mut from, &mut to);
    let mut at = from;
    buffer.insert(&mut at, "value");
    // Mirroring waits for an idle turn, so let the loop reach it.
    let context = glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
    let after = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    if !after.ends_with("let value = value.frob(arg);") {
        eprintln!("FAIL: linked stop did not mirror: {after}");
        return 1;
    }

    // Tab to the last stop, then off the end, which gives the keys back.
    if !page::move_through_snippet(&snippet_page, true) {
        eprintln!("FAIL: tab did not move to the next stop");
        return 1;
    }
    let (selection_start, selection_end) = buffer
        .selection_bounds()
        .map(|(from, to)| (from.offset(), to.offset()))
        .unwrap_or((-1, -1));
    if selection_end - selection_start != 3 {
        eprintln!("FAIL: the second stop's placeholder is not selected");
        return 1;
    }
    if !page::move_through_snippet(&snippet_page, true)
        || snippet_page.state.borrow().document.snippet_active()
    {
        eprintln!("FAIL: the exit stop did not end the session");
        return 1;
    }
    println!("snippets ok (expansion, tabstops, linked stops, exit)");

    // Leave the file as the rest of the checks expect it, through the
    // buffer so the core follows.
    let mut from = buffer.iter_at_offset(origin_chars);
    let mut to = buffer.end_iter();
    buffer.delete(&mut from, &mut to);
    if buffer.text(&buffer.start_iter(), &buffer.end_iter(), true) != expected {
        eprintln!("FAIL: the snippet did not come back out cleanly");
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
                        .map(|(label, _, is_snippet)| {
                            if is_snippet {
                                format!("{label} (snippet)")
                            } else {
                                label
                            }
                        })
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
        // The server's snippet item must arrive marked as one, or it
        // gets inserted with its placeholders showing.
        if !labels.iter().any(|label| label == "fake_snippet (snippet)") {
            eprintln!("FAIL: the snippet item was not recognized: {labels:?}");
            return 1;
        }
        if !hover.borrow().as_deref().unwrap_or("").contains("fake hover") {
            eprintln!("FAIL: hover text missing");
            return 1;
        }

        // The shell holds documents; the UI holds views of them. What
        // is checked here is that the table behaves like a registry: a
        // path opens once however many times it is asked for, and a
        // rename follows the document rather than making a second one.
        {
            let shell = shell::Shell::instance();
            let opened = shell.document_count();
            let page = workbench.selected().expect("a selected page");
            let again = shell.open_document(
                &page.buffer,
                &page.state,
                page.path().borrow().as_deref(),
            );
            if shell.document_count() != opened {
                eprintln!("FAIL: opening the same path twice made a second document");
                return 1;
            }
            let Some(path) = page.path().borrow().clone() else {
                eprintln!("FAIL: the page has no path to look up");
                return 1;
            };
            match shell.document_for_path(&path) {
                Some(found) if found.id == again.id => {}
                _ => {
                    eprintln!("FAIL: the path does not name its document");
                    return 1;
                }
            }
            // A rename moves the index, not the document.
            shell.rename_document(again.id, Some(&path), "/tmp/renamed-by-the-smoke-test");
            if shell.document_for_path(&path).is_some() {
                eprintln!("FAIL: the old path still names the document");
                return 1;
            }
            match shell.document_for_path("/tmp/renamed-by-the-smoke-test") {
                Some(found) if found.id == again.id => {}
                _ => {
                    eprintln!("FAIL: the new path does not name the document");
                    return 1;
                }
            }
            shell.rename_document(again.id, Some("/tmp/renamed-by-the-smoke-test"), &path);
            // A view reads its file through the document, so what one
            // view learns, every view of that file knows.
            let folded_before = page.document.folded.borrow().len();
            crate::page::fold_all(&page);
            if page.document.folded.borrow().len() <= folded_before {
                eprintln!("FAIL: folding did not reach the document");
                return 1;
            }
            crate::page::unfold_all(&page);
            let path_now = page.document.path.borrow().clone();
            if path_now != page.path().borrow().clone() {
                eprintln!("FAIL: the view and its document disagree about the path");
                return 1;
            }
            println!("documents ok (one per path, renamed without a second)");
        }

        // Folding hides the lines after the one that opens a block.
        // What is asserted here is the fold state; that GtkTextView
        // honours an invisible tag is a rendering fact, checked by
        // looking at it.
        {
            buffer.set_text("fn folded() {\n    let a = 1;\n    let b = 2;\n}\n");
            context.iteration(false);
            let page = workbench.selected().expect("a selected page");
            if crate::page::has_folds(&page) {
                eprintln!("FAIL: a fresh document is already folded");
                return 1;
            }
            if !crate::page::fold_all(&page) {
                eprintln!("FAIL: nothing folded in a document with a block");
                return 1;
            }
            if !crate::page::has_folds(&page) {
                eprintln!("FAIL: folding left no folds behind");
                return 1;
            }
            if !crate::page::unfold_all(&page) || crate::page::has_folds(&page) {
                eprintln!("FAIL: unfolding did not clear the folds");
                return 1;
            }
            println!("folding ok (folds taken and given back)");
        }

        // Code actions: the scripted server offers its quick fix only
        // when the client hands back the diagnostic as published, `data`
        // and all — which is the whole reason the pool keeps them.
        let actions: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        {
            let shell = shell::Shell::instance();
            let id = shell.pool.borrow_mut().code_action(&path, 0, 2);
            let sink = Rc::clone(&actions);
            shell.expect_response(id, move |json| {
                *sink.borrow_mut() = Some(json.to_owned());
            });
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            context.iteration(true);
            if actions.borrow().is_some() {
                break;
            }
            if std::time::Instant::now() > deadline {
                eprintln!("FAIL: the code action response did not arrive");
                return 1;
            }
        }
        let offered = textchum_core::code_action::actions(
            actions.borrow().as_deref().unwrap_or("[]"),
        );
        if !offered.iter().any(|action| action.title == "Quote the first word") {
            let titles: Vec<&str> =
                offered.iter().map(|action| action.title.as_str()).collect();
            eprintln!("FAIL: the quick fix was not offered: {titles:?}");
            return 1;
        }
        if !offered
            .iter()
            .any(|action| matches!(action.outcome(), textchum_core::code_action::Outcome::Resolve(_)))
        {
            eprintln!("FAIL: the action with no edit did not ask to be resolved");
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
            .find(|candidate| candidate.path().borrow().as_deref() == Some(second_key.as_str()));
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
        if untitled.path().borrow().is_some() {
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
