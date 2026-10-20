use crate::State;
use crate::db::MAX_BYTES;
use gtk::prelude::*;
use std::rc::Rc;

const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";

pub fn clipboard() -> gtk::Clipboard {
    gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD)
}

pub fn start(state: Rc<State>) {
    // The gtk-rs bindings do not wrap "owner-change", so connect by name.
    clipboard().connect_local("owner-change", false, move |args| {
        let cb: gtk::Clipboard = args[0].get().expect("clipboard arg");
        let state = state.clone();
        // Skip password-manager copies, which advertise a marker target.
        if cb.wait_is_target_available(&gtk::gdk::Atom::intern(PASSWORD_HINT)) {
            return None;
        }
        cb.request_text(move |_, text| {
            if let Some(text) = text {
                record(&state, text);
            }
        });
        None
    });
}

fn record(state: &State, text: &str) {
    if text.trim().is_empty() || text.len() > MAX_BYTES {
        return;
    }
    // Ignore the change we caused ourselves while pasting.
    if state.last_set.borrow().as_deref() == Some(text) {
        return;
    }
    if let Err(e) = state.db.upsert(text) {
        eprintln!("diodon-next: db error: {e}");
    }
}
