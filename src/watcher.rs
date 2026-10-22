use crate::State;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::Instant;

const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";

pub fn clipboard() -> gtk::Clipboard {
    gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD)
}

fn is_own_write_ignored(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.is_some_and(|deadline| now < deadline)
}

pub fn start(state: Rc<State>) {
    // The gtk-rs bindings do not wrap "owner-change", so connect by name.
    clipboard().connect_local("owner-change", false, move |args| {
        let cb: gtk::Clipboard = args[0].get().expect("clipboard arg");
        let state = state.clone();
        if is_own_write_ignored(state.ignore_until.get(), Instant::now()) {
            return None;
        }
        // Skip password-manager copies, which advertise a marker target. Remember
        // the text too: if the source exits, the session's clipboard service
        // re-offers the same content without the marker, and we must not store it.
        if cb.wait_is_target_available(&gtk::gdk::Atom::intern(PASSWORD_HINT)) {
            *state.last_secret.borrow_mut() = cb.wait_for_text().map(|t| t.to_string());
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
    if text.trim().is_empty() {
        return;
    }
    // Same text as a skipped password-manager copy (re-offered after its owner exited).
    if state.last_secret.borrow().as_deref() == Some(text) {
        return;
    }
    state.last_secret.borrow_mut().take();
    if let Err(e) = state.db.upsert_text(text) {
        eprintln!("diodon-next: db error: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_write_window_ignores_only_events_before_deadline() {
        let now = Instant::now();
        let deadline = now + crate::SELF_WRITE_WINDOW;
        assert!(is_own_write_ignored(Some(deadline), now));
        assert!(!is_own_write_ignored(Some(deadline), deadline));
        assert!(!is_own_write_ignored(None, now));
    }
}
