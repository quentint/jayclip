use crate::State;
use crate::db::{Entry, MAX_ENTRIES};
use crate::{paste, watcher};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

const LABEL_CHARS: usize = 60;

/// One-line, truncated label. `_` is doubled so GTK does not read it as a mnemonic.
pub fn label(content: &str) -> String {
    let line = content
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = line.chars().take(LABEL_CHARS).collect();
    if line.chars().count() > LABEL_CHARS || content.lines().filter(|l| !l.trim().is_empty()).count() > 1 {
        out.push('…');
    }
    out.replace('_', "__")
}

const PIN_MARK: &str = "★ ";

fn entry_label(entry: &Entry) -> String {
    let text = label(&entry.content);
    if entry.pinned {
        format!("{PIN_MARK}{text}")
    } else {
        text
    }
}

/// "Pin / unpin" submenu: every entry is listed, pinned ones are marked;
/// choosing one toggles its pin and does not paste anything.
fn pin_submenu(state: &Rc<State>, entries: &[Entry]) -> gtk::MenuItem {
    let parent = gtk::MenuItem::with_mnemonic("_Pin / unpin");
    let sub = gtk::Menu::new();
    for entry in entries {
        let item = gtk::MenuItem::with_label(&entry_label(entry).replace("__", "_"));
        let (state, id) = (state.clone(), entry.id);
        item.connect_activate(move |_| {
            if let Err(e) = state.db.toggle_pin(id) {
                eprintln!("diodon-next: db error: {e}");
            }
        });
        sub.append(&item);
    }
    parent.set_submenu(Some(&sub));
    parent
}

pub fn show(state: Rc<State>) {
    // Capture the focused window before our own UI steals focus.
    let target = paste::active_target();

    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let Some(pointer) = display.default_seat().and_then(|s| s.pointer()) else {
        return;
    };
    let (_, x, y) = pointer.position();

    // Tiny helper window at the pointer: the menu anchors to it and gets a keyboard grab.
    let helper = gtk::Window::new(gtk::WindowType::Toplevel);
    helper.set_decorated(false);
    helper.set_skip_taskbar_hint(true);
    helper.set_skip_pager_hint(true);
    helper.set_default_size(1, 1);
    helper.set_type_hint(gtk::gdk::WindowTypeHint::PopupMenu);
    helper.move_(x, y);
    helper.show_all();

    let menu = gtk::Menu::new();
    let entries = state.db.recent(MAX_ENTRIES).unwrap_or_default();

    // The target is consumed by whichever item is activated.
    let target = Rc::new(std::cell::RefCell::new(target));

    if entries.is_empty() {
        let item = gtk::MenuItem::with_label("(empty)");
        item.set_sensitive(false);
        menu.append(&item);
    }
    let mut separated = false;
    for (i, entry) in entries.iter().enumerate() {
        // Separate the pinned block from the rest.
        if i > 0 && !entry.pinned && entries[i - 1].pinned && !separated {
            menu.append(&gtk::SeparatorMenuItem::new());
            separated = true;
        }
        let text = entry_label(entry);
        let item = if i < 9 {
            gtk::MenuItem::with_mnemonic(&format!("_{} {}", i + 1, text))
        } else {
            gtk::MenuItem::with_label(&text.replace("__", "_"))
        };
        let content = entry.content.clone();
        let state = state.clone();
        let target = target.clone();
        item.connect_activate(move |_| {
            *state.last_set.borrow_mut() = Some(content.clone());
            if let Err(e) = state.db.upsert(&content) {
                eprintln!("diodon-next: db error: {e}");
            }
            let cb = watcher::clipboard();
            cb.set_text(&content);
            paste::send_paste(target.borrow_mut().take());
        });
        menu.append(&item);
    }

    if !entries.is_empty() {
        menu.append(&gtk::SeparatorMenuItem::new());
        menu.append(&pin_submenu(&state, &entries));
    }
    let clear = gtk::MenuItem::with_mnemonic("_Clear history");
    {
        let state = state.clone();
        clear.connect_activate(move |_| {
            if let Err(e) = state.db.clear() {
                eprintln!("diodon-next: db error: {e}");
            }
        });
    }
    menu.append(&clear);
    menu.show_all();

    let helper_clone = helper.clone();
    menu.connect_hide(move |_| {
        // Defer so a pending "activate" is delivered first.
        let h = helper_clone.clone();
        glib::idle_add_local_once(move || h.close());
    });

    menu.popup_at_widget(
        &helper,
        gtk::gdk::Gravity::NorthWest,
        gtk::gdk::Gravity::NorthWest,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_and_escapes() {
        assert_eq!(label("  foo   bar\tbaz "), "foo bar baz");
        assert_eq!(label("snake_case"), "snake__case");
    }

    #[test]
    fn marks_multiline_and_truncates() {
        assert_eq!(label("first\nsecond"), "first…");
        assert_eq!(label("\n\n  hi"), "hi");
        let long = "x".repeat(100);
        assert_eq!(label(&long).chars().count(), LABEL_CHARS + 1);
    }
}
