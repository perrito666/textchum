//! Textchum's Linux shell: GTK4 + libadwaita over the same Rust core the
//! macOS app uses — linked as a crate rather than through the C ABI,
//! since both sides are Rust. The architecture is identical: the core
//! owns every document, the shell owns presentation, and every edit
//! funnels through one choke point.
//!
//! The editor view is GtkSourceView for its gutter (line numbers come
//! free), with its own syntax engine switched off: coloring is the
//! core's tree-sitter spans painted as text tags from the shared theme
//! style table, exactly like the macOS rendering attributes.

mod editor;
mod shell;

use adw::prelude::*;
use gtk::gio;

const APP_ID: &str = "io.github.perrito666.textchum";

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
            let code = editor::run_smoke_test(app);
            std::process::exit(code);
        }
        editor::EditorWindow::new(app, None).present();
    });
    app.connect_open(|app, files, _hint| {
        for file in files {
            if let Some(path) = file.path() {
                editor::EditorWindow::new(app, Some(path)).present();
            }
        }
    });

    app.set_accels_for_action("win.new", &["<Ctrl>n"]);
    app.set_accels_for_action("win.find", &["<Ctrl>f"]);
    app.set_accels_for_action("win.quick-open", &["<Ctrl>p"]);
    app.set_accels_for_action("win.definition", &["F12"]);
    app.set_accels_for_action("win.open", &["<Ctrl>o"]);
    app.set_accels_for_action("win.save", &["<Ctrl>s"]);
    app.set_accels_for_action("win.save-as", &["<Ctrl><Shift>s"]);
    app.set_accels_for_action("win.undo", &["<Ctrl>z"]);
    app.set_accels_for_action("win.redo", &["<Ctrl><Shift>z"]);
    app.set_accels_for_action("window.close", &["<Ctrl>w"]);

    // GApplication consumes argv; strip our own flag so it does not try
    // to open a file called --smoke-test.
    let arguments: Vec<String> = std::env::args()
        .filter(|argument| argument != "--smoke-test")
        .collect();
    app.run_with_args(&arguments)
}
