//! The debug panel, under the editor: what a debug session is doing,
//! the stack of the thread that stopped, the variables of the frame
//! chosen in it, and a console with the program's output and a field to
//! evaluate expressions in that frame. The session is the core's
//! (`textchum_dap`); this draws what it says and sends what is asked.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use textchum_core::i18n::{fill, tr};
use textchum_dap::Command;

use crate::shell::Shell;

/// A frame of the stack: its id, name, file and zero-based line.
#[derive(Clone)]
struct Frame {
    id: i64,
    name: String,
    path: Option<String>,
    line: Option<i32>,
}

/// A scope or a variable in the variables list.
#[derive(Clone)]
struct Node {
    name: String,
    value: String,
    kind: Option<String>,
    reference: i64,
    scope: bool,
    /// None until asked for, for a variable that holds others.
    children: Option<Vec<Node>>,
}

pub struct DebugPanel {
    pub root: gtk::Box,
    title: gtk::Label,
    status: gtk::Label,
    steps: Vec<gtk::Button>,
    stop: gtk::Button,
    frames_list: gtk::ListBox,
    variables_list: gtk::ListBox,
    console: gtk::TextView,
    evaluate: gtk::Entry,
    /// Stack | (variables | console): their widths are set when a
    /// session begins, since every side would otherwise ask for all.
    outer: gtk::Paned,
    inner: gtk::Paned,
    session: Cell<Option<u64>>,
    frames: RefCell<Vec<Frame>>,
    scopes: RefCell<Vec<Node>>,
    /// The variables opened, by reference, so a rebuild keeps them open.
    expanded: RefCell<HashSet<i64>>,
    /// Each variables row's path into the tree: scope index, then child
    /// indices.
    variable_rows: RefCell<Vec<Vec<usize>>>,
    chosen: Cell<Option<i64>>,
    stopped: Cell<bool>,
    failed: Cell<bool>,
    /// A frame stopped in or chosen: its file and zero-based line.
    on_frame: RefCell<Option<Rc<dyn Fn(&str, i32)>>>,
    /// Running again, or over: no line is stopped at any more.
    on_resumed: RefCell<Option<Rc<dyn Fn()>>>,
    /// The panel itself, for its rows' clicks to reach it.
    me: RefCell<std::rc::Weak<DebugPanel>>,
}

impl DebugPanel {
    pub fn new() -> Rc<DebugPanel> {
        let title = gtk::Label::new(None);
        title.add_css_class("heading");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let status = gtk::Label::new(None);
        status.add_css_class("dim-label");
        status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        status.set_hexpand(true);
        status.set_xalign(0.0);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.set_margin_start(10);
        header.set_margin_end(10);
        header.set_margin_top(2);
        header.append(&title);
        header.append(&status);
        let mut steps = Vec::new();
        for label in [tr("Continue"), tr("Step Over"), tr("Step Into"), tr("Step Out")] {
            let button = gtk::Button::with_label(&label);
            button.add_css_class("flat");
            header.append(&button);
            steps.push(button);
        }
        let stop = gtk::Button::with_label(&tr("Stop"));
        let close = gtk::Button::with_label(&tr("Close"));
        for button in [&stop, &close] {
            button.add_css_class("flat");
            header.append(button);
        }

        let frames_list = gtk::ListBox::new();
        frames_list.add_css_class("navigation-sidebar");
        frames_list.update_property(&[gtk::accessible::Property::Label(&tr("Call Stack"))]);
        let variables_list = gtk::ListBox::new();
        variables_list.set_selection_mode(gtk::SelectionMode::None);
        variables_list.update_property(&[gtk::accessible::Property::Label(&tr("Variables"))]);
        let console = gtk::TextView::new();
        console.set_editable(false);
        console.set_cursor_visible(false);
        console.set_monospace(true);
        console.set_left_margin(6);
        let evaluate = gtk::Entry::new();
        evaluate.set_placeholder_text(Some(&tr("An expression to evaluate where the program stopped — ⏎")));
        let scrolled = |child: &gtk::Widget| {
            gtk::ScrolledWindow::builder().child(child).vexpand(true).width_request(160).build()
        };
        let console_side = gtk::Box::new(gtk::Orientation::Vertical, 2);
        console_side.append(&scrolled(console.upcast_ref()));
        console_side.append(&evaluate);
        console_side.set_hexpand(true);
        let inner = gtk::Paned::new(gtk::Orientation::Horizontal);
        inner.set_start_child(Some(&scrolled(variables_list.upcast_ref())));
        inner.set_end_child(Some(&console_side));
        inner.set_shrink_start_child(false);
        inner.set_shrink_end_child(false);
        inner.set_hexpand(true);
        let outer = gtk::Paned::new(gtk::Orientation::Horizontal);
        outer.set_start_child(Some(&scrolled(frames_list.upcast_ref())));
        outer.set_end_child(Some(&inner));
        outer.set_resize_start_child(false);
        outer.set_shrink_start_child(false);
        outer.set_shrink_end_child(false);
        outer.set_vexpand(true);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&header);
        root.append(&outer);
        root.set_size_request(-1, 240);
        root.set_visible(false);

        let panel = Rc::new(DebugPanel {
            root,
            title,
            status,
            steps: steps.clone(),
            stop: stop.clone(),
            frames_list: frames_list.clone(),
            variables_list: variables_list.clone(),
            console,
            evaluate: evaluate.clone(),
            outer: outer.clone(),
            inner: inner.clone(),
            session: Cell::new(None),
            frames: RefCell::new(Vec::new()),
            scopes: RefCell::new(Vec::new()),
            expanded: RefCell::new(HashSet::new()),
            variable_rows: RefCell::new(Vec::new()),
            chosen: Cell::new(None),
            stopped: Cell::new(false),
            failed: Cell::new(false),
            on_frame: RefCell::new(None),
            on_resumed: RefCell::new(None),
            me: RefCell::new(std::rc::Weak::new()),
        });
        *panel.me.borrow_mut() = Rc::downgrade(&panel);
        for (button, command) in steps.iter().zip(["continue", "next", "stepIn", "stepOut"]) {
            let weak = Rc::downgrade(&panel);
            button.connect_clicked(move |_| {
                if let Some(panel) = weak.upgrade() {
                    panel.perform(command);
                }
            });
        }
        {
            let weak = Rc::downgrade(&panel);
            stop.connect_clicked(move |_| {
                if let Some(panel) = weak.upgrade() {
                    panel.perform("stop");
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
            frames_list.connect_row_selected(move |_, row| {
                let (Some(panel), Some(row)) = (weak.upgrade(), row) else { return };
                let index = row.index().max(0) as usize;
                let id = panel.frames.borrow().get(index).map(|frame| frame.id);
                if id.is_some() && id != panel.chosen.get() {
                    panel.choose(index);
                }
            });
        }
        {
            let weak = Rc::downgrade(&panel);
            evaluate.connect_activate(move |entry| {
                let Some(panel) = weak.upgrade() else { return };
                let expression = entry.text().trim().to_owned();
                if panel.evaluate_expression(&expression) {
                    entry.set_text("");
                }
            });
        }
        panel.set_steppable(false);
        panel
    }

    pub fn set_on_frame(&self, open: Rc<dyn Fn(&str, i32)>) {
        *self.on_frame.borrow_mut() = Some(open);
    }

    pub fn set_on_resumed(&self, resumed: Rc<dyn Fn()>) {
        *self.on_resumed.borrow_mut() = Some(resumed);
    }

    pub fn session_id(&self) -> Option<u64> {
        self.session.get()
    }

    pub fn is_shown(&self) -> bool {
        self.root.is_visible()
    }

    fn set_steppable(&self, steppable: bool) {
        self.stopped.set(steppable);
        for button in &self.steps {
            button.set_sensitive(steppable);
        }
        self.evaluate.set_sensitive(steppable);
    }

    /// Shows a session that just started, in place of the one before,
    /// which is ended and let go of.
    pub fn begin(&self, id: u64, title: &str) {
        if let Some(previous) = self.session.get().filter(|previous| *previous != id) {
            Shell::instance().debuggers.borrow_mut().forget(previous);
        }
        self.session.set(Some(id));
        self.failed.set(false);
        self.title.set_text(title);
        self.status.set_text(&tr("starting"));
        self.frames.borrow_mut().clear();
        self.scopes.borrow_mut().clear();
        self.expanded.borrow_mut().clear();
        self.chosen.set(None);
        self.frames_list.remove_all();
        self.rebuild_variables();
        self.console.buffer().set_text("");
        self.stop.set_sensitive(true);
        self.set_steppable(false);
        self.root.set_visible(true);
        // A quarter for the stack, the rest halved between the variables
        // and the console.
        let width = self.root.width().max(self.root.parent().map(|parent| parent.width()).unwrap_or(0));
        let width = if width > 0 { width } else { 1000 };
        self.outer.set_position(width / 4);
        self.inner.set_position(width * 3 / 8);
    }

    /// What the session said.
    pub fn handle(&self, kind: &str, json: &str) {
        let body: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
        match kind {
            "output" => self.append(body["text"].as_str().unwrap_or_default()),
            "running" | "continued" => {
                self.status.set_text(&tr("running"));
                self.frames.borrow_mut().clear();
                self.frames_list.remove_all();
                self.scopes.borrow_mut().clear();
                self.rebuild_variables();
                self.set_steppable(false);
                self.resumed();
            }
            "breakpoints" => {
                let file = body["path"].as_str().unwrap_or_default().rsplit('/').next().unwrap_or_default().to_owned();
                for point in body["breakpoints"].as_array().into_iter().flatten() {
                    if point["verified"] == false {
                        let line = point["line"].as_u64().map(|line| (line + 1).to_string()).unwrap_or_else(|| "?".into());
                        let why = point["message"].as_str().map(str::to_owned).unwrap_or_else(|| tr("the debugger did not say why"));
                        self.append(&format!("{}\n", fill(&tr("The breakpoint at {}:{} is not set: {}"), &[&file, &line, &why])));
                    }
                }
            }
            "stopped" => {
                let frames: Vec<Frame> = body["frames"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|frame| Frame {
                        id: frame["id"].as_i64().unwrap_or(0),
                        name: frame["name"].as_str().unwrap_or("?").to_owned(),
                        path: frame["path"].as_str().map(str::to_owned),
                        line: frame["line"].as_i64().map(|line| line as i32),
                    })
                    .collect();
                let reason = body["description"].as_str().filter(|text| !text.is_empty()).or(body["reason"].as_str()).unwrap_or_default();
                self.status.set_text(&if reason.is_empty() { tr("stopped") } else { fill(&tr("stopped: {}"), &[reason]) });
                self.set_steppable(true);
                self.frames_list.remove_all();
                for frame in &frames {
                    let place = frame.path.as_deref().map(|path| {
                        let file = path.rsplit('/').next().unwrap_or(path);
                        match frame.line {
                            Some(line) => format!("{file}:{}", line + 1),
                            None => file.to_owned(),
                        }
                    });
                    let text = match place {
                        Some(place) => format!("{}  {place}", frame.name),
                        None => frame.name.clone(),
                    };
                    self.frames_list.append(&row_label(&text, frame.path.is_none(), 0));
                }
                *self.frames.borrow_mut() = frames;
                self.chosen.set(None);
                if let Some(first) = self.frames_list.row_at_index(0) {
                    // Choosing it asks for its variables and shows its line.
                    self.frames_list.select_row(Some(&first));
                }
            }
            "variables" => {
                if body["frame"].as_i64() != self.chosen.get() {
                    return;
                }
                let scopes: Vec<Node> = body["scopes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|scope| Node {
                        name: scope["name"].as_str().unwrap_or_default().to_owned(),
                        value: String::new(),
                        kind: None,
                        reference: 0,
                        scope: true,
                        children: Some(nodes(&scope["variables"])),
                    })
                    .collect();
                *self.scopes.borrow_mut() = scopes;
                self.rebuild_variables();
            }
            "children" => {
                let Some(reference) = body["reference"].as_i64() else { return };
                let children = nodes(&body["variables"]);
                let mut scopes = self.scopes.borrow_mut();
                if let Some(node) = find(&mut scopes, reference) {
                    node.children = Some(children);
                }
                drop(scopes);
                self.rebuild_variables();
            }
            "evaluated" => {
                let result = body["result"].as_str().unwrap_or_default();
                self.append(&format!("› {}\n{result}\n", body["expression"].as_str().unwrap_or_default()));
            }
            "exited" => {
                let code = body["exitCode"].as_i64().unwrap_or(0).to_string();
                self.append(&format!("{}\n", fill(&tr("— exited with {}"), &[&code])));
            }
            "waiting" | "failed" => {
                if kind == "failed" {
                    self.failed.set(true);
                }
                let message = body["message"].as_str().unwrap_or_default();
                self.status.set_text(message);
                self.append(&format!("{message}\n"));
            }
            "ended" => {
                if !self.failed.get() {
                    self.status.set_text(&tr("ended"));
                }
                self.stop.set_sensitive(false);
                self.set_steppable(false);
                self.resumed();
            }
            _ => {}
        }
    }

    fn resumed(&self) {
        let resumed = self.on_resumed.borrow().clone();
        if let Some(resumed) = resumed {
            resumed();
        }
    }

    fn append(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let buffer = self.console.buffer();
        buffer.insert(&mut buffer.end_iter(), text);
        let mut end = buffer.end_iter();
        self.console.scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
    }

    /// A frame of the stack chosen: its variables asked for, its line
    /// shown.
    fn choose(&self, index: usize) {
        let Some(frame) = self.frames.borrow().get(index).cloned() else { return };
        let Some(session) = self.session.get() else { return };
        self.chosen.set(Some(frame.id));
        self.scopes.borrow_mut().clear();
        self.rebuild_variables();
        Shell::instance().debuggers.borrow().send(session, Command::Variables { frame: frame.id });
        if let (Some(path), Some(line)) = (frame.path.as_deref(), frame.line) {
            let open = self.on_frame.borrow().clone();
            if let Some(open) = open {
                open(path, line);
            }
        }
    }

    /// Lists the scopes and the variables, the opened ones with what
    /// they hold.
    fn rebuild_variables(&self) {
        self.variables_list.remove_all();
        let mut rows = Vec::new();
        let scopes = self.scopes.borrow();
        let expanded = self.expanded.borrow();
        for (index, scope) in scopes.iter().enumerate() {
            self.variables_list.append(&row_label(&scope.name, true, 0));
            rows.push(vec![index]);
            self.add_children(scope.children.as_deref().unwrap_or_default(), &vec![index], 1, &expanded, &mut rows);
        }
        drop(scopes);
        drop(expanded);
        *self.variable_rows.borrow_mut() = rows;
    }

    fn add_children(&self, nodes: &[Node], at: &[usize], depth: i32, expanded: &HashSet<i64>, rows: &mut Vec<Vec<usize>>) {
        for (index, node) in nodes.iter().enumerate() {
            let mut path = at.to_vec();
            path.push(index);
            let open = node.reference > 0 && expanded.contains(&node.reference);
            let marker = if node.reference > 0 { if open { "▾ " } else { "▸ " } } else { "  " };
            let kind = node.kind.as_deref().map(|kind| format!("  {kind}")).unwrap_or_default();
            let row = row_label(&format!("{marker}{} = {}{kind}", node.name, node.value), false, depth);
            if node.reference > 0 {
                let click = gtk::GestureClick::new();
                let reference = node.reference;
                let weak = self.me.borrow().clone();
                click.connect_released(move |_, _, _, _| {
                    if let Some(panel) = weak.upgrade() {
                        panel.toggle(reference);
                    }
                });
                row.add_controller(click);
            }
            self.variables_list.append(&row);
            rows.push(path.clone());
            if open {
                if let Some(children) = &node.children {
                    self.add_children(children, &path, depth + 1, expanded, rows);
                }
            }
        }
    }

    /// Opens or closes a variable that holds others; opened for the
    /// first time, it asks what it holds.
    pub(crate) fn toggle(&self, reference: i64) {
        let opened = {
            let mut expanded = self.expanded.borrow_mut();
            if !expanded.remove(&reference) {
                expanded.insert(reference);
                true
            } else {
                false
            }
        };
        let unknown = {
            let mut scopes = self.scopes.borrow_mut();
            find(&mut scopes, reference).is_some_and(|node| node.children.is_none())
        };
        if opened && unknown {
            if let Some(session) = self.session.get() {
                Shell::instance().debuggers.borrow().send(session, Command::Children { reference });
            }
        }
        self.rebuild_variables();
    }

    /// Continue, Step Over, Step Into, Step Out, Stop: false when there
    /// is no session, or the program is not stopped for a step.
    pub fn perform(&self, command: &str) -> bool {
        let Some(session) = self.session.get() else { return false };
        if command != "stop" && !self.stopped.get() {
            return false;
        }
        let Some(command) = Command::from_json(&serde_json::json!({"command": command}).to_string()) else {
            return false;
        };
        Shell::instance().debuggers.borrow().send(session, command)
    }

    /// Evaluates an expression in the chosen frame.
    pub(crate) fn evaluate_expression(&self, expression: &str) -> bool {
        let (Some(session), Some(frame)) = (self.session.get(), self.chosen.get()) else { return false };
        if expression.is_empty() {
            return false;
        }
        Shell::instance().debuggers.borrow().send(session, Command::Evaluate { frame, expression: expression.to_owned() })
    }

    /// Puts the panel away; a session still going keeps going.
    pub fn hide(&self) {
        self.root.set_visible(false);
    }

    pub fn show(&self) {
        if self.session.get().is_some() {
            self.root.set_visible(true);
        }
    }

    /// What the smoke test reads back.
    pub(crate) fn status_text(&self) -> String {
        self.status.text().to_string()
    }

    pub(crate) fn frame_names(&self) -> Vec<String> {
        self.frames.borrow().iter().map(|frame| frame.name.clone()).collect()
    }

    pub(crate) fn console_text(&self) -> String {
        let buffer = self.console.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string()
    }

    pub(crate) fn variable_texts(&self) -> Vec<String> {
        let mut texts = Vec::new();
        let mut child = self.variables_list.first_child();
        while let Some(widget) = child {
            if let Some(label) = widget.downcast_ref::<gtk::ListBoxRow>().and_then(|row| row.child()).and_then(|child| child.downcast::<gtk::Label>().ok()) {
                texts.push(label.text().trim_start_matches(['▾', '▸', ' ']).to_owned());
            }
            child = widget.next_sibling();
        }
        texts
    }

    /// The reference of the variable called `name`, for the smoke test to
    /// open it.
    pub(crate) fn reference_of(&self, name: &str) -> Option<i64> {
        fn search(nodes: &[Node], name: &str) -> Option<i64> {
            nodes.iter().find_map(|node| {
                if !node.scope && node.name == name && node.reference > 0 {
                    Some(node.reference)
                } else {
                    search(node.children.as_deref().unwrap_or_default(), name)
                }
            })
        }
        search(&self.scopes.borrow(), name)
    }
}

fn nodes(value: &serde_json::Value) -> Vec<Node> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|variable| Node {
            name: variable["name"].as_str().unwrap_or_default().to_owned(),
            value: variable["value"].as_str().unwrap_or_default().to_owned(),
            kind: variable["type"].as_str().filter(|kind| !kind.is_empty()).map(str::to_owned),
            reference: variable["reference"].as_i64().unwrap_or(0),
            scope: false,
            children: None,
        })
        .collect()
}

fn find(nodes: &mut [Node], reference: i64) -> Option<&mut Node> {
    for node in nodes.iter_mut() {
        if !node.scope && node.reference == reference {
            return Some(node);
        }
        if let Some(children) = node.children.as_mut() {
            if let Some(found) = find(children, reference) {
                return Some(found);
            }
        }
    }
    None
}

/// A row of one of the lists: monospaced, indented by `depth`, dimmed
/// for a heading or a frame with no file.
fn row_label(text: &str, dim: bool, depth: i32) -> gtk::ListBoxRow {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_tooltip_text(Some(text));
    label.set_margin_start(6 + depth * 14);
    label.add_css_class("monospace");
    if dim {
        label.add_css_class("dim-label");
    }
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&label));
    row
}
