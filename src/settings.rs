use crate::State;
use gtk::prelude::*;
use std::rc::Rc;

pub const PASSWORD_WARNING: &str = "Do not store passwords uses an X11 clipboard hint. It skips copies only when the source advertises x-kde-passwordManagerHint. Some password managers do not provide this hint, and desktop clipboard services can strip it when re-offering clipboard data. In those cases, copied passwords may still be stored. Keep your password manager's clipboard-clearing timer enabled.";

pub fn needs_password_warning(previous: bool, next: bool) -> bool {
    !previous && next
}

pub fn submenu(state: Rc<State>) -> gtk::MenuItem {
    let parent = gtk::MenuItem::with_mnemonic("_Settings");
    let menu = gtk::Menu::new();
    menu.append(&password_toggle(state.clone()));
    menu.append(&history_size_submenu(state.clone()));
    menu.append(&recording_toggle(state.clone()));
    menu.append(&gtk::SeparatorMenuItem::new());
    let clear = gtk::MenuItem::with_mnemonic("_Clear history");
    let state = state.clone();
    clear.connect_activate(move |_| {
        if let Err(e) = state.db.clear() {
            eprintln!("diodon-next: db error: {e}");
        }
    });
    menu.append(&clear);
    parent.set_submenu(Some(&menu));
    parent
}

fn password_toggle(state: Rc<State>) -> gtk::CheckMenuItem {
    let item = gtk::CheckMenuItem::with_label("Do not store passwords");
    item.set_active(state.do_not_store_passwords.get());
    let suppress_toggle = Rc::new(std::cell::Cell::new(false));
    let suppress_for_handler = suppress_toggle.clone();
    item.connect_toggled(move |item| {
        if suppress_for_handler.replace(false) {
            return;
        }
        let enabled = item.is_active();
        if needs_password_warning(state.do_not_store_passwords.get(), enabled) {
            set_toggle_active(item, false, &suppress_toggle);
            let dialog = gtk::MessageDialog::new(
                None::<&gtk::Window>,
                gtk::DialogFlags::MODAL,
                gtk::MessageType::Warning,
                gtk::ButtonsType::OkCancel,
                PASSWORD_WARNING,
            );
            dialog.show_all();
            let response = dialog.run();
            dialog.close();
            if response == gtk::ResponseType::Ok {
                save_password_setting(&state, true, item, &suppress_toggle);
            }
            set_toggle_active(item, state.do_not_store_passwords.get(), &suppress_toggle);
        } else {
            save_password_setting(&state, enabled, item, &suppress_toggle);
        }
    });
    item
}

fn history_size_submenu(state: Rc<State>) -> gtk::MenuItem {
    let parent = gtk::MenuItem::with_label("History size");
    let menu = gtk::Menu::new();
    let current = state
        .db
        .display_entries()
        .unwrap_or(crate::db::DEFAULT_DISPLAY_ENTRIES);
    let mut group: Option<gtk::RadioMenuItem> = None;
    for count in crate::db::DISPLAY_ENTRY_OPTIONS {
        let item = if let Some(group) = group.as_ref() {
            gtk::RadioMenuItem::with_label_from_widget(group, Some(&count.to_string()))
        } else {
            let item = gtk::RadioMenuItem::new();
            item.set_label(&count.to_string());
            group = Some(item.clone());
            item
        };
        item.set_active(count == current);
        let state = state.clone();
        item.connect_activate(move |item| {
            if item.is_active()
                && let Err(e) = state.db.set_display_entries(count)
            {
                eprintln!("diodon-next: db error setting history size: {e}");
            }
        });
        menu.append(&item);
    }
    parent.set_submenu(Some(&menu));
    parent
}

fn recording_toggle(state: Rc<State>) -> gtk::CheckMenuItem {
    let item = gtk::CheckMenuItem::with_label("Pause recording");
    item.set_active(state.recording_paused.get());
    let suppress_toggle = Rc::new(std::cell::Cell::new(false));
    let suppress_for_handler = suppress_toggle.clone();
    item.connect_toggled(move |item| {
        if suppress_for_handler.replace(false) {
            return;
        }
        let paused = item.is_active();
        match state.db.set_recording_paused(paused) {
            Ok(()) => {
                state.recording_paused.set(paused);
                state
                    .recording_generation
                    .set(state.recording_generation.get().wrapping_add(1));
            }
            Err(e) => {
                eprintln!("diodon-next: could not save recording state: {e}");
                set_toggle_active(item, state.recording_paused.get(), &suppress_toggle);
            }
        }
    });
    item
}

fn set_toggle_active(
    item: &gtk::CheckMenuItem,
    active: bool,
    suppress_toggle: &std::cell::Cell<bool>,
) {
    if item.is_active() != active {
        suppress_toggle.set(true);
        item.set_active(active);
    }
}

fn save_password_setting(
    state: &State,
    enabled: bool,
    item: &gtk::CheckMenuItem,
    suppress_toggle: &std::cell::Cell<bool>,
) {
    match state.db.set_do_not_store_passwords(enabled) {
        Ok(()) => state.do_not_store_passwords.set(enabled),
        Err(e) => {
            eprintln!("balloon: could not save settings: {e}");
            set_toggle_active(item, state.do_not_store_passwords.get(), suppress_toggle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_is_only_requested_when_enabling() {
        assert!(!needs_password_warning(true, true));
        assert!(!needs_password_warning(true, false));
        assert!(needs_password_warning(false, true));
        assert!(!needs_password_warning(false, false));
        assert!(PASSWORD_WARNING.contains("x-kde-passwordManagerHint"));
        assert!(PASSWORD_WARNING.contains("clipboard-clearing timer"));
    }
}
