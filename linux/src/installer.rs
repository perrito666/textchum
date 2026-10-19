//! Installing what a document needs and does not have: the language
//! servers, debuggers and save formatters found missing, each with the
//! ways this machine can install it (`textchum_core::install`). The
//! editor does not tell the person to run a command it can run itself
//! (6.4): the install runs in the run panel, and when it succeeds the
//! servers that were missing are started without being asked.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use serde_json::Value;
use textchum_core::i18n::{fill, tr};

use crate::shell::Shell;
use crate::workbench::{Workbench, PickerRow};

thread_local! {
    /// Installs running in a run panel, by run id: the program each
    /// installs.
    static PENDING: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new());
    /// What to do again once a program is installed: the debugging that
    /// could not start without it.
    static RETRY: RefCell<HashMap<String, Box<dyn Fn()>>> = RefCell::new(HashMap::new());
}

/// The document whose needs are asked about: the selected page's file,
/// or its project.
fn subject(workbench: &Workbench) -> Option<PathBuf> {
    let page = workbench.selected()?;
    let path = page.path().borrow().clone().or_else(|| page.document.project_root.borrow().clone())?;
    Some(PathBuf::from(path))
}

/// What is missing for the selected page, with how each can be
/// installed.
fn offers(workbench: &Workbench) -> Vec<Value> {
    let Some(path) = subject(workbench) else { return Vec::new() };
    serde_json::from_str::<Value>(&textchum_core::install::offers_json(&path))
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

/// Shows, in the status bar, what the selected page needs and does not
/// have. Asked when the page in front changes and when something is
/// found missing or installed, not at every keystroke: it looks on disk.
pub fn refresh(workbench: &Workbench) {
    let programs: Vec<String> = offers(workbench)
        .iter()
        .filter_map(|offer| offer["program"].as_str().map(str::to_owned))
        .collect();
    workbench.show_missing(&programs);
}

/// Textchum ▸ Install Missing Tools…, and the status bar's Install
/// button: what is missing for the page in front and the ways to install
/// each, the one that suits the project first; Return installs the first.
pub fn offer(workbench: &Rc<Workbench>) {
    let offers = offers(workbench);
    if offers.is_empty() {
        workbench.hint(&tr("Nothing this document needs is missing."));
        refresh(workbench);
        return;
    }
    let mut rows = Vec::new();
    let mut choices: Vec<(String, Value)> = Vec::new();
    for offer in &offers {
        let program = offer["program"].as_str().unwrap_or_default().to_owned();
        let heading = format!("{program} — {}", offer["purpose"].as_str().unwrap_or_default());
        let methods = offer["methods"].as_array().cloned().unwrap_or_default();
        if methods.is_empty() {
            // Nothing to run: what it comes with, under its heading, as
            // a row that says so and does nothing.
            let notes: Vec<String> = offer["notes"]
                .as_array()
                .map(|notes| notes.iter().filter_map(|note| note.as_str().map(str::to_owned)).collect())
                .filter(|notes: &Vec<String>| !notes.is_empty())
                .unwrap_or_else(|| vec![tr("nothing on this machine can install it")]);
            rows.push(PickerRow::new(notes.join("; "), Some(heading)));
            choices.push((program, Value::Null));
            continue;
        }
        for (index, method) in methods.into_iter().enumerate() {
            // The command on a line of its own: it is long, and it is
            // what will run.
            let label = format!(
                "{}\n    {}",
                method["title"].as_str().unwrap_or_default(),
                method["command"].as_str().unwrap_or_default()
            );
            rows.push(PickerRow::new(label, (index == 0).then(|| heading.clone())));
            choices.push((program.clone(), method));
        }
    }
    crate::workbench::present_picker(workbench, &tr("Install Missing Tools"), rows, move |workbench, index| {
        let Some((program, method)) = choices.get(index) else { return };
        if method.is_null() {
            return;
        }
        install(workbench, program, method);
    });
}

/// Runs `method` in the run panel, or, for what needs an administrator,
/// hands the command over.
fn install(workbench: &Rc<Workbench>, program: &str, method: &Value) {
    let command = method["command"].as_str().unwrap_or_default();
    if method["runs"] == false {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(command);
        }
        Shell::instance().notify(&fill(
            &tr("Copied “{}”: it needs an administrator's password, which the run panel cannot ask for. Paste it into a terminal."),
            &[command],
        ));
        return;
    }
    run_install(workbench, program, command, Path::new(method["directory"].as_str().unwrap_or("/")));
}

/// Runs `command`, which installs `program`, in `directory`, in the run
/// panel; [`finished`] hears how it ended.
pub fn run_install(workbench: &Rc<Workbench>, program: &str, command: &str, directory: &Path) {
    crate::workbench::run_in_directory(workbench, command, directory);
    if let Some(id) = workbench.run_panel.run_id() {
        PENDING.with(|pending| pending.borrow_mut().insert(id, program.to_owned()));
    }
}

/// A run ended in `workbench`'s panel: if it was an install, says how it
/// went, and on success starts what was missing.
pub fn finished(workbench: &Rc<Workbench>, id: u64) {
    let Some(program) = PENDING.with(|pending| pending.borrow_mut().remove(&id)) else { return };
    let shell = Shell::instance();
    if workbench.run_panel.exit_code() == Some(0) {
        textchum_core::install::forget(&program);
        shell.notify(&fill(&tr("{} is installed"), &[&program]));
        start_what_was_missing();
        if let Some(retry) = RETRY.with(|retry| retry.borrow_mut().remove(&program)) {
            retry();
        }
    } else {
        shell.notify(&fill(&tr("Installing {} failed; the run panel has what the installer said."), &[&program]));
    }
    Workbench::for_each(|workbench| refresh(workbench));
}

/// Lets the pool try the servers that were missing, and announces every
/// open document again: a server that now starts gets it, one that
/// already had it is not told twice. A formatter is picked up at the next
/// save.
fn start_what_was_missing() {
    let shell = Shell::instance();
    shell.reconfigure_pool();
    Workbench::for_each(|workbench| {
        for page in workbench.all_pages() {
            let path = page.path().borrow().clone();
            let language = page.state.borrow().document.language_name().map(str::to_owned);
            if let (Some(path), Some(language)) = (path, language) {
                let text = page.state.borrow().document.text();
                shell.pool.borrow_mut().did_open(Path::new(&path), &language, &text);
            }
        }
    });
}

/// Debugging could not start: when it is because the debugger is not
/// installed, the ways to install it are offered at once — it was asked
/// for just now — and `retry` runs once one of them worked.
pub fn after_debug_failure(workbench: &Rc<Workbench>, retry: Box<dyn Fn()>) {
    let debugger = tr("debugger");
    let Some(program) = offers(workbench)
        .into_iter()
        .find(|offer| offer["purpose"].as_str() == Some(debugger.as_str()))
        .and_then(|offer| offer["program"].as_str().map(str::to_owned))
    else {
        return;
    };
    RETRY.with(|pending| pending.borrow_mut().insert(program, retry));
    refresh(workbench);
    offer(workbench);
}
