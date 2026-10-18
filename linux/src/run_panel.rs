//! The run panel, under the editor: a command's output as it comes, in
//! its colours, with the places it names underlined to open with a
//! click and walked with Next and Previous Output Place. While the
//! command runs, a line can be sent to its input — which is what a
//! command that has gone quiet is often waiting for, and the panel says
//! so. The output is the core's (`textchum_core::output`); this draws it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use textchum_core::i18n::tr;

use crate::shell::Shell;

/// A place an output line names: (start, end, path, line, column), in
/// characters, the line and column zero-based.
type Named = (usize, usize, String, usize, usize);

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
        let stop = gtk::Button::with_label(&tr("Stop"));
        let close = gtk::Button::with_label(&tr("Close"));
        for button in [&stop, &close] {
            button.add_css_class("flat");
        }
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.set_margin_start(10);
        header.set_margin_end(10);
        header.set_margin_top(2);
        header.append(&title);
        header.append(&status);
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
        let scroll = gtk::ScrolledWindow::builder().child(&view).vexpand(true).build();
        let input = gtk::Entry::new();
        input.set_placeholder_text(Some(&tr("A line for its input — ⏎ sends it")));
        input.set_margin_start(8);
        input.set_margin_end(8);
        input.set_margin_bottom(4);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&header);
        root.append(&scroll);
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
        });
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

    pub fn run_id(&self) -> Option<u64> {
        self.run_id.get()
    }

    pub fn command(&self) -> String {
        self.command.borrow().clone()
    }

    pub fn is_shown(&self) -> bool {
        self.root.is_visible()
    }

    /// Shows a run that just started, in place of the one before, which
    /// is let go of.
    pub fn begin(&self, id: u64, command: &str) {
        if let Some(previous) = self.run_id.get().filter(|previous| *previous != id) {
            Shell::instance().runs.borrow_mut().forget(previous);
        }
        self.run_id.set(Some(id));
        *self.command.borrow_mut() = command.to_owned();
        self.title.set_text(command);
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
}
