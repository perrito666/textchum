//! The run panel, under the editor: a command's output as it comes, in
//! its colours, with the places it names underlined to open with a
//! click and walked with Next and Previous Output Place. While the
//! command runs, a line can be sent to its input — which is what a
//! command that has gone quiet is often waiting for, and the panel says
//! so. The output is the core's (`textchum_core::output`); this draws it.
//!
//! When the output reports tests, they are listed beside it by suite,
//! each with how it went; choosing one shows the line that reported it,
//! and Rerun Failed runs again only the ones that failed.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use textchum_core::i18n::tr;

use crate::shell::Shell;

/// A place an output line names: (start, end, path, line, column), in
/// characters, the line and column zero-based.
type Named = (usize, usize, String, usize, usize);

/// A test's row in the list.
#[derive(Clone)]
struct TestRow {
    suite: String,
    name: String,
    line: Option<usize>,
}

pub struct RunPanel {
    pub root: gtk::Box,
    title: gtk::Label,
    status: gtk::Label,
    stop: gtk::Button,
    view: gtk::TextView,
    input: gtk::Entry,
    run_id: Cell<Option<u64>>,
    command: RefCell<String>,
    places: RefCell<Vec<Vec<Named>>>,
    /// Where Next Output Place stepped last: an output line and a start.
    stepped: Cell<Option<(usize, usize)>>,
    /// Opens a place: path, zero-based line and column.
    on_open: RefCell<Option<Rc<dyn Fn(&str, i32, usize)>>>,
    /// Runs a command line in a folder: Rerun Failed.
    on_run: RefCell<Option<Rc<dyn Fn(&str, &Path)>>>,
    /// Where the run on show was started, which is where it runs again.
    cwd: RefCell<PathBuf>,
    summary: gtk::Label,
    rerun: gtk::Button,
    paned: gtk::Paned,
    tests_list: gtk::ListBox,
    /// Typing filters the tests (3.3); the entry and the list go together.
    tests_filter: gtk::SearchEntry,
    tests_side: gtk::Box,
    /// The tests as last read, to leave the list alone when unchanged.
    tests_json: RefCell<String>,
    rerun_command: RefCell<Option<String>>,
    /// Each list row's suite and test names and the output line that
    /// reported it; None for a suite's row.
    test_rows: RefCell<Vec<Option<TestRow>>>,
}

impl RunPanel {
    pub fn new() -> Rc<RunPanel> {
        let title = gtk::Label::new(None);
        title.add_css_class("heading");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let status = gtk::Label::new(None);
        status.add_css_class("dim-label");
        status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        status.set_hexpand(true);
        status.set_xalign(0.0);
        let summary = gtk::Label::new(None);
        summary.add_css_class("dim-label");
        summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let rerun = gtk::Button::with_label(&tr("Rerun Failed"));
        rerun.set_visible(false);
        let stop = gtk::Button::with_label(&tr("Stop"));
        let close = gtk::Button::with_label(&tr("Close"));
        for button in [&rerun, &stop, &close] {
            button.add_css_class("flat");
        }
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.set_margin_start(10);
        header.set_margin_end(10);
        header.set_margin_top(2);
        header.append(&title);
        header.append(&status);
        header.append(&summary);
        header.append(&rerun);
        header.append(&stop);
        header.append(&close);

        let view = gtk::TextView::new();
        view.set_editable(false);
        view.set_cursor_visible(false);
        view.set_monospace(true);
        view.set_left_margin(8);
        let table = view.buffer().tag_table();
        table.add(&gtk::TextTag::builder().name("bold").weight(700).build());
        table.add(&gtk::TextTag::builder().name("place").underline(gtk::pango::Underline::Single).build());
        table.add(&gtk::TextTag::builder().name("note").foreground("#8a8a8a").build());
        let scroll = gtk::ScrolledWindow::builder().child(&view).vexpand(true).hexpand(true).build();
        let tests_list = gtk::ListBox::new();
        tests_list.add_css_class("navigation-sidebar");
        // A double-click is Return: either runs the test alone; a single
        // click only chooses it.
        tests_list.set_activate_on_single_click(false);
        tests_list.update_property(&[gtk::accessible::Property::Label(&tr("Tests"))]);
        let tests_scroll = gtk::ScrolledWindow::builder()
            .child(&tests_list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let tests_filter = gtk::SearchEntry::new();
        tests_filter.set_placeholder_text(Some(&tr("Filter tests")));
        let tests_side = gtk::Box::new(gtk::Orientation::Vertical, 2);
        tests_side.append(&tests_filter);
        tests_side.append(&tests_scroll);
        tests_side.set_width_request(160);
        tests_side.set_visible(false);
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&tests_side));
        paned.set_end_child(Some(&scroll));
        paned.set_shrink_start_child(false);
        paned.set_resize_start_child(false);
        paned.set_vexpand(true);
        let input = gtk::Entry::new();
        input.set_placeholder_text(Some(&tr("A line for its input — ⏎ sends it")));
        input.set_margin_start(8);
        input.set_margin_end(8);
        input.set_margin_bottom(4);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&header);
        root.append(&paned);
        root.append(&input);
        root.set_size_request(-1, 220);
        root.set_visible(false);

        let panel = Rc::new(RunPanel {
            root,
            title,
            status,
            stop: stop.clone(),
            view: view.clone(),
            input: input.clone(),
            run_id: Cell::new(None),
            command: RefCell::new(String::new()),
            places: RefCell::new(Vec::new()),
            stepped: Cell::new(None),
            on_open: RefCell::new(None),
            on_run: RefCell::new(None),
            cwd: RefCell::new(PathBuf::new()),
            summary,
            rerun: rerun.clone(),
            paned,
            tests_list: tests_list.clone(),
            tests_filter: tests_filter.clone(),
            tests_side,
            tests_json: RefCell::new(String::new()),
            rerun_command: RefCell::new(None),
            test_rows: RefCell::new(Vec::new()),
        });
        {
            let weak = Rc::downgrade(&panel);
            rerun.connect_clicked(move |_| {
                if let Some(panel) = weak.upgrade() {
                    panel.rerun_failed();
                }
            });
        }
        {
            // A test chosen: the output line that reported it.
            let weak = Rc::downgrade(&panel);
            tests_list.connect_row_selected(move |_, row| {
                let Some(panel) = weak.upgrade() else { return };
                let Some(row) = row else { return };
                let line = panel
                    .test_rows
                    .borrow()
                    .get(row.index() as usize)
                    .and_then(|test| test.as_ref().and_then(|test| test.line));
                if let Some(line) = line {
                    panel.show_output_line(line);
                }
            });
        }
        {
            let weak = Rc::downgrade(&panel);
            tests_filter.connect_search_changed(move |_| {
                if let Some(panel) = weak.upgrade() {
                    if let Some(id) = panel.run_id.get() {
                        panel.refresh_tests(id);
                    }
                }
            });
        }
        {
            let weak = Rc::downgrade(&panel);
            tests_list.connect_row_activated(move |_, _| {
                if let Some(panel) = weak.upgrade() {
                    panel.run_chosen_test();
                }
            });
        }
        {
            let weak = Rc::downgrade(&panel);
            stop.connect_clicked(move |_| {
                if let Some(panel) = weak.upgrade() {
                    panel.stop();
                }
            });
        }
        {
            let weak = Rc::downgrade(&panel);
            close.connect_clicked(move |_| {
                if let Some(panel) = weak.upgrade() {
                    panel.hide();
                }
            });
        }
        {
            let weak = Rc::downgrade(&panel);
            input.connect_activate(move |entry| {
                let Some(panel) = weak.upgrade() else { return };
                let Some(id) = panel.run_id.get() else { return };
                let line = entry.text().to_string();
                if !line.is_empty() && Shell::instance().runs.borrow().write(id, &line) {
                    entry.set_text("");
                    panel.status.set_text(&tr("running"));
                }
            });
        }
        {
            // A place clicked in the output opens.
            let weak = Rc::downgrade(&panel);
            let click = gtk::GestureClick::new();
            click.connect_released(move |_, _, x, y| {
                let Some(panel) = weak.upgrade() else { return };
                let (bx, by) = panel.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
                let Some(at) = panel.view.iter_at_location(bx, by) else { return };
                let (line, offset) = (at.line() as usize, at.line_offset() as usize);
                let place = panel
                    .places
                    .borrow()
                    .get(line)
                    .and_then(|named| named.iter().find(|place| place.0 <= offset && offset < place.1).cloned());
                if let Some(place) = place {
                    panel.stepped.set(Some((line, place.0)));
                    panel.open(&place);
                }
            });
            view.add_controller(click);
        }
        panel
    }

    pub fn set_on_open(&self, open: Rc<dyn Fn(&str, i32, usize)>) {
        *self.on_open.borrow_mut() = Some(open);
    }

    pub fn set_on_run(&self, run: Rc<dyn Fn(&str, &Path)>) {
        *self.on_run.borrow_mut() = Some(run);
    }

    /// Where the run on show was started.
    pub fn cwd(&self) -> PathBuf {
        self.cwd.borrow().clone()
    }

    pub fn run_id(&self) -> Option<u64> {
        self.run_id.get()
    }

    pub fn command(&self) -> String {
        self.command.borrow().clone()
    }

    pub fn is_shown(&self) -> bool {
        self.root.is_visible()
    }

    /// Shows a run that just started in `cwd`, in place of the one
    /// before, which is let go of.
    pub fn begin(&self, id: u64, command: &str, cwd: &Path) {
        if let Some(previous) = self.run_id.get().filter(|previous| *previous != id) {
            Shell::instance().runs.borrow_mut().forget(previous);
        }
        self.run_id.set(Some(id));
        *self.command.borrow_mut() = command.to_owned();
        *self.cwd.borrow_mut() = cwd.to_path_buf();
        self.title.set_text(command);
        self.tests_json.borrow_mut().clear();
        *self.rerun_command.borrow_mut() = None;
        self.test_rows.borrow_mut().clear();
        self.tests_list.remove_all();
        self.tests_side.set_visible(false);
        self.tests_filter.set_text("");
        self.summary.set_text("");
        self.rerun.set_visible(false);
        self.status.set_text(&tr("running"));
        self.view.buffer().set_text("");
        self.places.borrow_mut().clear();
        self.stepped.set(None);
        self.stop.set_sensitive(true);
        self.input.set_visible(true);
        self.input.set_text("");
        self.root.set_visible(true);
    }

    /// What the core said about the run on show.
    pub fn handle(&self, news: &str) {
        let Some(id) = self.run_id.get() else { return };
        if news == "quiet" {
            self.status.set_text(&tr("no output for a while — it may be waiting for input"));
            return;
        }
        let read = Shell::instance().runs.borrow().read(id);
        let Some(read) = read else { return };
        let Ok(update) = serde_json::from_str::<serde_json::Value>(&read) else { return };
        self.apply(&update);
        self.refresh_tests(id);
        if update["running"] == false {
            self.status.set_text(&tr("finished"));
            self.stop.set_sensitive(false);
            self.input.set_visible(false);
        } else if news == "output" {
            self.status.set_text(&tr("running"));
        }
    }

    fn apply(&self, update: &serde_json::Value) {
        let buffer = self.view.buffer();
        let lines = update["lines"].as_array().cloned().unwrap_or_default();
        let first = if update["reset"] == true { 0 } else { update["first"].as_u64().unwrap_or(0) as usize };
        // Was the end in view? Then it stays in view.
        let at_end = self
            .view
            .vadjustment()
            .is_none_or(|adjustment| adjustment.value() + adjustment.page_size() >= adjustment.upper() - 4.0);
        let line_count = buffer.line_count() as usize;
        let mut start = if first < line_count {
            buffer.iter_at_line(first as i32).unwrap_or_else(|| buffer.end_iter())
        } else {
            buffer.end_iter()
        };
        let mut end = buffer.end_iter();
        buffer.delete(&mut start, &mut end);
        let mut places = self.places.borrow_mut();
        places.truncate(first);
        for (index, line) in lines.iter().enumerate() {
            let mut at = buffer.end_iter();
            if index > 0 || (first > 0 && first >= line_count) {
                buffer.insert(&mut at, "\n");
            }
            let line_start = buffer.end_iter().offset();
            let text = line["text"].as_str().unwrap_or_default();
            buffer.insert(&mut buffer.end_iter(), text);
            let range = |from: u64, to: u64| {
                (
                    buffer.iter_at_offset(line_start + from as i32),
                    buffer.iter_at_offset(line_start + to as i32),
                )
            };
            for span in line["spans"].as_array().into_iter().flatten() {
                let (Some(from), Some(to)) = (span[0].as_u64(), span[1].as_u64()) else { continue };
                let (from_iter, to_iter) = range(from, to);
                if let Some(colour) = span[2].as_str() {
                    let name = format!("fg{colour}");
                    if buffer.tag_table().lookup(&name).is_none() {
                        buffer.tag_table().add(&gtk::TextTag::builder().name(&name).foreground(colour).build());
                    }
                    buffer.apply_tag_by_name(&name, &from_iter, &to_iter);
                }
                if span[3] == true {
                    buffer.apply_tag_by_name("bold", &from_iter, &to_iter);
                }
            }
            let mut named = Vec::new();
            for place in line["places"].as_array().into_iter().flatten() {
                let (Some(from), Some(to), Some(path), Some(row), Some(column)) =
                    (place[0].as_u64(), place[1].as_u64(), place[2].as_str(), place[3].as_u64(), place[4].as_u64())
                else {
                    continue;
                };
                let (from_iter, to_iter) = range(from, to);
                buffer.apply_tag_by_name("place", &from_iter, &to_iter);
                named.push((from as usize, to as usize, path.to_owned(), row as usize, column as usize));
            }
            places.push(named);
        }
        drop(places);
        if at_end {
            let view = self.view.clone();
            // Once the new lines are laid out, so the end is where it is.
            glib::idle_add_local_once(move || {
                if let Some(adjustment) = view.vadjustment() {
                    adjustment.set_value(adjustment.upper() - adjustment.page_size());
                }
            });
        }
    }

    /// Lists the tests the output reported, when they changed.
    fn refresh_tests(&self, id: u64) {
        let query = self.tests_filter.text().to_string();
        let Some(json) = Shell::instance().runs.borrow().tests(id, &query) else { return };
        if *self.tests_json.borrow() == json {
            return;
        }
        let Ok(tests) = serde_json::from_str::<serde_json::Value>(&json) else { return };
        *self.tests_json.borrow_mut() = json;
        let suites = tests["suites"].as_array().cloned().unwrap_or_default();
        let reported = ["passed", "failed", "skipped", "running"].iter().any(|count| tests[*count].as_u64().unwrap_or(0) > 0);
        self.tests_list.remove_all();
        let mut rows = Vec::new();
        for suite in &suites {
            let tests = suite["tests"].as_array().cloned().unwrap_or_default();
            let outcomes: Vec<&str> = tests.iter().filter_map(|test| test["outcome"].as_str()).collect();
            let suite_name = suite["name"].as_str().unwrap_or_default();
            let label = if suite_name.is_empty() { tr("package not reported yet") } else { suite_name.to_owned() };
            self.tests_list.append(&test_row(&label, None, &outcomes, true));
            rows.push(None);
            for test in &tests {
                let name = test["name"].as_str().unwrap_or_default();
                let outcome = test["outcome"].as_str().unwrap_or_default();
                self.tests_list.append(&test_row(name, test["duration"].as_str(), &[outcome], false));
                rows.push(Some(TestRow {
                    suite: suite_name.to_owned(),
                    name: name.to_owned(),
                    line: test["line"].as_u64().map(|line| line as usize),
                }));
            }
        }
        *self.test_rows.borrow_mut() = rows;
        if reported && !self.tests_side.is_visible() {
            self.tests_side.set_visible(true);
            let width = self.root.width();
            self.paned.set_position(if width > 0 { (width * 3 / 10).clamp(200, 320) } else { 240 });
        }
        self.summary.set_text(tests["summary"].as_str().unwrap_or_default());
        let rerun = tests["rerun"].as_str().map(str::to_owned);
        self.rerun.set_visible(rerun.is_some());
        *self.rerun_command.borrow_mut() = rerun;
    }

    /// Selects output line `line` and scrolls it into view.
    pub fn show_output_line(&self, line: usize) {
        let buffer = self.view.buffer();
        let Some(start) = buffer.iter_at_line(line as i32) else { return };
        let mut end = start;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        buffer.select_range(&start, &end);
        self.view.scroll_to_iter(&mut start.clone(), 0.1, false, 0.0, 0.0);
    }

    /// Rerun Failed: only the tests that failed, from where the run was
    /// started. False when there is nothing to run again.
    pub fn rerun_failed(&self) -> bool {
        let command = self.rerun_command.borrow().clone();
        let run = self.on_run.borrow().clone();
        match (command, run) {
            (Some(command), Some(run)) => {
                let cwd = self.cwd();
                run(&command, &cwd);
                true
            }
            _ => false,
        }
    }

    /// Return or a double-click on a test: that test alone, from where
    /// the run was started. False when no test is chosen or the run's
    /// command cannot be narrowed to it, which the status says.
    pub fn run_chosen_test(&self) -> bool {
        let Some(id) = self.run_id.get() else { return false };
        let Some(index) = self.tests_list.selected_row().map(|row| row.index() as usize) else { return false };
        let Some(test) = self.test_rows.borrow().get(index).cloned().flatten() else { return false };
        let Some(command) = Shell::instance().runs.borrow().rerun_test(id, &test.suite, &test.name) else {
            self.status.set_text(&tr("This run's command cannot be narrowed to one test."));
            return false;
        };
        let run = self.on_run.borrow().clone();
        match run {
            Some(run) => {
                let cwd = self.cwd();
                run(&command, &cwd);
                true
            }
            None => false,
        }
    }

    fn open(&self, place: &Named) {
        let open = self.on_open.borrow().clone();
        if let Some(open) = open {
            open(&place.2, place.3 as i32, place.4);
        }
    }

    /// Next Output Place and Previous: the place after or before the last
    /// one gone to, round the end, marked in the output and opened.
    /// False when the output names none.
    pub fn step(&self, forward: bool) -> bool {
        let Some(id) = self.run_id.get() else { return false };
        let found = Shell::instance().runs.borrow().neighbouring_place(id, self.stepped.get(), forward);
        let Some((line, place)) = found else { return false };
        self.stepped.set(Some((line, place.start)));
        let buffer = self.view.buffer();
        if let Some(start) = buffer.iter_at_line(line as i32) {
            let mut from = start;
            from.forward_chars(place.start as i32);
            let mut to = start;
            to.forward_chars(place.end as i32);
            buffer.select_range(&from, &to);
            self.view.scroll_to_iter(&mut from.clone(), 0.1, false, 0.0, 0.0);
        }
        self.open(&(place.start, place.end, place.path.to_string_lossy().into_owned(), place.line, place.column));
        true
    }

    pub fn stop(&self) {
        if let Some(id) = self.run_id.get() {
            Shell::instance().runs.borrow().stop(id);
        }
    }

    /// Puts the panel away; a run still going keeps going.
    pub fn hide(&self) {
        self.root.set_visible(false);
    }

    pub fn show(&self) {
        if self.run_id.get().is_some() {
            self.root.set_visible(true);
        }
    }

    /// What the panel shows, for the smoke test.
    pub(crate) fn shown_text(&self) -> String {
        let buffer = self.view.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string()
    }

    pub(crate) fn status_text(&self) -> String {
        self.status.text().to_string()
    }

    pub(crate) fn shows_tests(&self) -> bool {
        self.tests_side.is_visible()
    }

    /// Types `query` into the tests' filter, for the smoke test.
    pub(crate) fn filter_tests(&self, query: &str) {
        self.tests_filter.set_text(query);
        if let Some(id) = self.run_id.get() {
            self.refresh_tests(id);
        }
    }

    /// The tests listed, as the filter leaves them, for the smoke test.
    pub(crate) fn listed_test_names(&self) -> Vec<String> {
        self.test_rows.borrow().iter().flatten().map(|test| test.name.clone()).collect()
    }

    pub(crate) fn summary_text(&self) -> String {
        self.summary.text().to_string()
    }

    /// Chooses the test called `name` in the list, as a click would.
    pub(crate) fn choose_test(&self, name: &str) {
        let index = self
            .test_rows
            .borrow()
            .iter()
            .position(|row| row.as_ref().is_some_and(|test| test.name == name));
        if let Some(row) = index.and_then(|index| self.tests_list.row_at_index(index as i32)) {
            self.tests_list.select_row(Some(&row));
        }
    }

    pub(crate) fn selected_output_text(&self) -> String {
        let buffer = self.view.buffer();
        buffer.selection_bounds().map(|(from, to)| buffer.text(&from, &to, false).to_string()).unwrap_or_default()
    }
}

/// A row of the test list: a suite's, in bold with its worst outcome, or
/// a test's, indented under it.
fn test_row(name: &str, duration: Option<&str>, outcomes: &[&str], suite: bool) -> gtk::ListBoxRow {
    let (icon, class) = if outcomes.contains(&"failed") {
        ("process-stop-symbolic", Some("error"))
    } else if outcomes.contains(&"unfinished") {
        ("dialog-warning-symbolic", Some("warning"))
    } else if outcomes.contains(&"running") {
        ("content-loading-symbolic", None)
    } else if !outcomes.is_empty() && outcomes.iter().all(|outcome| *outcome == "skipped") {
        ("list-remove-symbolic", None)
    } else {
        ("object-select-symbolic", Some("success"))
    };
    let image = gtk::Image::from_icon_name(icon);
    if let Some(class) = class {
        image.add_css_class(class);
    }
    if let Some(outcome) = outcomes.first().filter(|_| !suite) {
        image.update_property(&[gtk::accessible::Property::Label(outcome)]);
    }
    let label = gtk::Label::new(Some(name));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_hexpand(true);
    label.set_tooltip_text(Some(name));
    if suite {
        label.add_css_class("heading");
    }
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    line.set_margin_start(if suite { 4 } else { 20 });
    line.append(&image);
    line.append(&label);
    if let Some(duration) = duration {
        let time = gtk::Label::new(Some(duration));
        time.add_css_class("dim-label");
        line.append(&time);
    }
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&line));
    row.set_selectable(!suite);
    row.set_activatable(!suite);
    row
}
