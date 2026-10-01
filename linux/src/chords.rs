//! The chord window: commands by one key each.
//!
//! Two modifier keys held together open it over the bottom of the
//! window; it lists what can be done, a key beside each, in groups
//! short enough to read. The next key runs a command or opens a group,
//! Escape closes it. The menu is the core's, so the keys are the ones
//! the macOS build shows.
//!
//! It is an overlay, not a window of its own: the editor keeps the
//! keyboard focus throughout, so a command it runs acts on the document
//! the user was in.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use textchum_core::chords::{self, Entry, Target};
use textchum_core::i18n::tr;

use crate::shell::Shell;

/// What a key typed into the window did.
#[derive(Debug, PartialEq)]
pub enum Pressed {
    /// Nothing has that key; the window stays as it was.
    Refused,
    /// A group opened.
    Opened,
    /// The window closed, and this action is to run.
    Ran(&'static str),
}

pub struct ChordWindow {
    frame: gtk::Box,
    heading: gtk::Label,
    grid: gtk::Grid,
    /// The keys of the groups opened to get to the list on show.
    path: RefCell<Vec<char>>,
    /// Goes up with every key pressed or released. An opening that was
    /// set off by the pair being held goes ahead only if this is still
    /// what it was then: anything in between means the pair was part of
    /// a shortcut, or let go.
    presses: Cell<u64>,
}

/// One entry as its line: the key, then its name, a group marked with
/// a plus.
fn line(entry: &Entry) -> String {
    let mark = if matches!(entry.target, Target::Group(_)) { "+" } else { "" };
    format!("{}  {mark}{}", entry.key, entry.label)
}

impl ChordWindow {
    pub fn show(&self) {
        self.path.borrow_mut().clear();
        self.render();
        self.frame.set_visible(true);
    }

    pub fn close(&self) {
        self.frame.set_visible(false);
    }

    pub fn is_shown(&self) -> bool {
        self.frame.get_visible()
    }

    /// The lines on show, for the smoke test.
    pub fn lines(&self) -> Vec<String> {
        let path = self.path.borrow().clone();
        chords::list_at(&path).0.iter().map(line).collect()
    }

    /// Takes a key typed while the window is up. A key that is nothing
    /// leaves the window up: a slip should not cost the place in the
    /// menu.
    pub fn press(&self, key: char) -> Pressed {
        let path = self.path.borrow().clone();
        let (entries, _) = chords::list_at(&path);
        let Some(entry) = entries.into_iter().find(|entry| entry.key == key) else {
            return Pressed::Refused;
        };
        match entry.target {
            Target::Group(_) => {
                self.path.borrow_mut().push(key);
                self.render();
                Pressed::Opened
            }
            Target::Action(name) => {
                self.close();
                Pressed::Ran(name)
            }
        }
    }

    fn render(&self) {
        let path = self.path.borrow().clone();
        let (entries, trail) = chords::list_at(&path);
        let heading = if trail.is_empty() { tr("Commands") } else { trail.join(" \u{25b8} ") };
        self.heading.set_text(&heading);
        while let Some(child) = self.grid.first_child() {
            self.grid.remove(&child);
        }
        // Down the columns, so a list reads top to bottom.
        let rows = entries.len().div_ceil(3).max(1);
        for (index, entry) in entries.iter().enumerate() {
            let mark = if matches!(entry.target, Target::Group(_)) { "+" } else { "" };
            let label = gtk::Label::new(None);
            label.set_markup(&format!(
                "<b><span foreground=\"#f8e45c\">{}</span></b>  {mark}{}",
                glib::markup_escape_text(&entry.key.to_string()),
                glib::markup_escape_text(&entry.label),
            ));
            label.set_xalign(0.0);
            self.grid.attach(&label, (index / rows) as i32, (index % rows) as i32, 1, 1);
        }
    }
}

const HELD: gdk::ModifierType = gdk::ModifierType::CONTROL_MASK
    .union(gdk::ModifierType::ALT_MASK)
    .union(gdk::ModifierType::SHIFT_MASK)
    .union(gdk::ModifierType::SUPER_MASK);

/// The modifiers of a pair as the configuration spells it, or `None`
/// for the empty setting that leaves the window off.
pub fn modifiers(pair: &str) -> Option<gdk::ModifierType> {
    if pair.is_empty() {
        return None;
    }
    let mut held = gdk::ModifierType::empty();
    for part in pair.split('+') {
        held |= match part {
            "ctrl" => gdk::ModifierType::CONTROL_MASK,
            "alt" => gdk::ModifierType::ALT_MASK,
            "shift" => gdk::ModifierType::SHIFT_MASK,
            "cmd" => gdk::ModifierType::SUPER_MASK,
            _ => return None,
        };
    }
    Some(held)
}

/// A pair as Preferences names it: `ctrl+alt` is Ctrl+Alt.
pub fn pair_title(pair: &str) -> String {
    pair.split('+')
        .map(|part| match part {
            "ctrl" => "Ctrl",
            "alt" => "Alt",
            "shift" => "Shift",
            "cmd" => "Super",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// The modifier a key is, if it is one.
fn modifier_of(key: gdk::Key) -> Option<gdk::ModifierType> {
    match key {
        gdk::Key::Control_L | gdk::Key::Control_R => Some(gdk::ModifierType::CONTROL_MASK),
        // With Shift down first, the Alt keys arrive as Meta.
        gdk::Key::Alt_L | gdk::Key::Alt_R | gdk::Key::Meta_L | gdk::Key::Meta_R => {
            Some(gdk::ModifierType::ALT_MASK)
        }
        gdk::Key::Shift_L | gdk::Key::Shift_R => Some(gdk::ModifierType::SHIFT_MASK),
        gdk::Key::Super_L | gdk::Key::Super_R => Some(gdk::ModifierType::SUPER_MASK),
        _ => None,
    }
}

/// The character a key types with nothing held. The pair may still be
/// down when the next key comes, and with Shift in it `[` would arrive
/// as `{`.
fn plain_key(controller: &gtk::EventControllerKey, key: gdk::Key, keycode: u32) -> Option<char> {
    let group = controller.group() as i32;
    let unshifted = gdk::Display::default()
        .and_then(|display| display.map_keycode(keycode))
        .and_then(|keys| {
            keys.iter()
                .find(|(at, _)| at.level() == 0 && at.group() == group)
                .or_else(|| keys.iter().find(|(at, _)| at.level() == 0))
                .map(|(_, key)| *key)
        })
        .unwrap_or(key);
    unshifted.to_unicode().map(|typed| typed.to_lowercase().next().unwrap_or(typed))
}

fn run(window: &adw::ApplicationWindow, name: &str) {
    if let Some(action) = crate::keyboard::gtk_action(name) {
        let _ = gtk::prelude::WidgetExt::activate_action(window, action, None);
    }
}

/// Builds the window over `overlay` and has `window` watch its keys for
/// the pair that opens it.
///
/// The window opens only when the pair is held on its own for a
/// moment. A shortcut that uses the same modifiers presses its key well
/// inside that moment, so it is neither delayed nor taken.
pub fn install(window: &adw::ApplicationWindow, overlay: &gtk::Overlay) -> Rc<ChordWindow> {
    let heading = gtk::Label::new(None);
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    let grid = gtk::Grid::new();
    grid.set_column_spacing(28);
    grid.set_row_spacing(2);
    let footer = gtk::Label::new(Some(&tr("esc closes")));
    footer.set_xalign(0.0);
    footer.add_css_class("dim-label");

    let frame = gtk::Box::new(gtk::Orientation::Vertical, 10);
    frame.add_css_class("osd");
    frame.add_css_class("chord-window");
    frame.set_halign(gtk::Align::Center);
    frame.set_valign(gtk::Align::End);
    frame.set_margin_bottom(48);
    // It is only looked at: a click goes to what is under it, and the
    // keyboard focus stays in the editor.
    frame.set_can_target(false);
    frame.set_can_focus(false);
    frame.set_visible(false);
    frame.append(&heading);
    frame.append(&grid);
    frame.append(&footer);
    overlay.add_overlay(&frame);

    let provider = gtk::CssProvider::new();
    provider.load_from_string(".chord-window { padding: 14px 18px; border-radius: 10px; }");
    gtk::style_context_add_provider_for_display(
        &gtk::prelude::WidgetExt::display(window),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let chord = Rc::new(ChordWindow {
        frame,
        heading,
        grid,
        path: RefCell::new(Vec::new()),
        presses: Cell::new(0),
    });

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let chord = Rc::clone(&chord);
        // Weak: the window owns the controller, which owns this.
        let window = window.downgrade();
        keys.connect_key_pressed(move |controller, key, keycode, state| {
            if chord.is_shown() {
                if modifier_of(key).is_some() {
                    return glib::Propagation::Proceed;
                }
                if key == gdk::Key::Escape {
                    chord.close();
                    return glib::Propagation::Stop;
                }
                let Some(window) = window.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                match plain_key(controller, key, keycode).map(|typed| chord.press(typed)) {
                    Some(Pressed::Ran(name)) => run(&window, name),
                    Some(Pressed::Refused) => window.error_bell(),
                    Some(Pressed::Opened) | None => {}
                }
                // Taken either way: while the window is up, keys are its.
                return glib::Propagation::Stop;
            }
            let held = state & HELD;
            let Some(pressed) = modifier_of(key) else {
                chord.presses.set(chord.presses.get() + 1);
                return glib::Propagation::Proceed;
            };
            if held.contains(pressed) {
                // A modifier repeating while held changes nothing.
                return glib::Propagation::Proceed;
            }
            chord.presses.set(chord.presses.get() + 1);
            let wanted = modifiers(&Shell::instance().config.borrow().chord_modifiers());
            if wanted == Some(held | pressed) {
                let armed = chord.presses.get();
                let chord = Rc::clone(&chord);
                glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                    if chord.presses.get() == armed {
                        chord.show();
                    }
                });
            }
            glib::Propagation::Proceed
        });
    }
    {
        let chord = Rc::clone(&chord);
        keys.connect_key_released(move |_, _, _, _| {
            chord.presses.set(chord.presses.get() + 1);
        });
    }
    window.add_controller(keys);

    // A click anywhere is the user going back to work.
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let chord = Rc::clone(&chord);
        click.connect_pressed(move |_, _, _, _| chord.close());
    }
    window.add_controller(click);

    // A window that lost the keyboard hears no release: without this a
    // pair held while switching away would open the window unseen.
    {
        let chord = Rc::clone(&chord);
        window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                chord.presses.set(chord.presses.get() + 1);
                chord.close();
            }
        });
    }

    // Screenshot-driven verification: the keys to press once it is up.
    if let Some(typed) = std::env::var_os("TEXTCHUM_DEBUG_CHORD") {
        let chord = Rc::clone(&chord);
        glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
            chord.show();
            for key in typed.to_string_lossy().chars() {
                chord.press(key);
            }
        });
    }

    chord
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_is_its_two_modifiers_and_nothing_is_off() {
        assert_eq!(modifiers(""), None);
        assert_eq!(modifiers("ctrl+bogus"), None);
        assert_eq!(
            modifiers("ctrl+alt"),
            Some(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK)
        );
        for pair in chords::MODIFIER_PAIRS {
            let held = modifiers(pair).expect("a pair the core offers");
            assert_eq!(held.bits().count_ones(), 2, "{pair} is not two keys here");
            assert!(!pair_title(pair).contains("cmd"), "{pair} is shown as it is spelled");
        }
        assert_eq!(pair_title("alt+cmd"), "Alt+Super");
    }
}
