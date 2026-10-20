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

/// Keypad digit 1-9 to a zero-based entry index.
fn keypad_index(key: gtk::gdk::keys::Key) -> Option<usize> {
    use gtk::gdk::keys::constants as k;
    [
        k::KP_1, k::KP_2, k::KP_3, k::KP_4, k::KP_5, k::KP_6, k::KP_7, k::KP_8, k::KP_9,
    ]
    .iter()
    .position(|c| *c == key)
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

/// Time the clipboard stays owned after the paste keystroke, so the target
/// app can fetch the text before a one-shot process exits.
const GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// Show the history menu. `on_finish` runs once the interaction is over: the
/// menu was dismissed, or the chosen entry has been pasted.
pub fn show(state: Rc<State>, on_finish: Rc<dyn Fn()>) {
    // A held or repeated shortcut can call us again while a menu is already
    // up (or still retrying its grab). Ignore it instead of stacking menus.
    if state.menu_active.replace(true) {
        return;
    }
    let on_finish: Rc<dyn Fn()> = {
        let state = state.clone();
        Rc::new(move || {
            state.menu_active.set(false);
            on_finish();
        })
    };

    // Capture the focused window before our own UI steals focus.
    let target = paste::active_target();

    let Some(display) = gtk::gdk::Display::default() else {
        on_finish();
        return;
    };
    let Some(pointer) = display.default_seat().and_then(|s| s.pointer()) else {
        on_finish();
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
    // Set when an item starts a paste, which then owns calling `on_finish`.
    let pasting = Rc::new(std::cell::Cell::new(false));

    if entries.is_empty() {
        let item = gtk::MenuItem::with_label("(empty)");
        item.set_sensitive(false);
        menu.append(&item);
    }
    let mut separated = false;
    // Entry items in display order, for keypad shortcuts.
    let mut items: Vec<gtk::MenuItem> = Vec::new();
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
        let pasting = pasting.clone();
        let on_finish = on_finish.clone();
        item.connect_activate(move |_| {
            *state.last_set.borrow_mut() = Some(content.clone());
            if let Err(e) = state.db.upsert(&content) {
                eprintln!("diodon-next: db error: {e}");
            }
            let cb = watcher::clipboard();
            cb.set_text(&content);
            pasting.set(true);
            let target = target.borrow_mut().take();
            let on_finish = on_finish.clone();
            glib::spawn_future_local(async move {
                paste::send_paste(target).await;
                glib::timeout_future(GRACE).await;
                on_finish();
            });
        });
        menu.append(&item);
        items.push(item);
    }

    // Keypad 1-9 select entries by position. The top-row digits keep working
    // through the mnemonics above (Shift+digit on AZERTY).
    menu.connect_key_press_event(move |menu, event| {
        match keypad_index(event.keyval()).and_then(|i| items.get(i)) {
            Some(item) => {
                // Activating directly bypasses GTK's own close-on-select path.
                menu.popdown();
                item.activate();
                glib::Propagation::Stop
            }
            None => glib::Propagation::Proceed,
        }
    });

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

    // Set once the menu is really on screen. Until then a hide is just a failed
    // popup attempt and must not end the interaction.
    let shown = Rc::new(std::cell::Cell::new(false));
    {
        let shown = shown.clone();
        menu.connect_map(move |_| shown.set(true));
    }
    {
        let helper = helper.clone();
        let shown = shown.clone();
        let pasting = pasting.clone();
        let on_finish = on_finish.clone();
        menu.connect_hide(move |_| {
            if !shown.get() {
                return;
            }
            // Defer so a pending "activate" is delivered first.
            let (h, pasting, on_finish) = (helper.clone(), pasting.clone(), on_finish.clone());
            glib::idle_add_local_once(move || {
                h.close();
                if !pasting.get() {
                    on_finish();
                }
            });
        });
    }

    popup_when_grabbable(menu, helper, shown, on_finish, 0);
}

/// The global shortcut runs us while its keys are still held, and the window
/// manager's key grab makes GTK's menu grab fail. Retry until the keys are
/// released; give up (and clean up) after a couple of seconds.
fn popup_when_grabbable(
    menu: gtk::Menu,
    helper: gtk::Window,
    shown: Rc<std::cell::Cell<bool>>,
    on_finish: Rc<dyn Fn()>,
    attempt: u32,
) {
    const RETRY: std::time::Duration = std::time::Duration::from_millis(25);
    const MAX_ATTEMPTS: u32 = 80;

    menu.popup_at_widget(
        &helper,
        gtk::gdk::Gravity::NorthWest,
        gtk::gdk::Gravity::NorthWest,
        None,
    );
    glib::timeout_add_local_once(RETRY, move || {
        if shown.get() && menu.is_visible() {
            return;
        }
        menu.popdown();
        if attempt + 1 >= MAX_ATTEMPTS {
            eprintln!("diodon-next: could not grab the keyboard for the menu");
            helper.close();
            on_finish();
            return;
        }
        popup_when_grabbable(menu, helper, shown, on_finish, attempt + 1);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypad_digits_map_to_positions() {
        use gtk::gdk::keys::constants as k;
        assert_eq!(keypad_index(k::KP_1), Some(0));
        assert_eq!(keypad_index(k::KP_9), Some(8));
        assert_eq!(keypad_index(k::KP_0), None);
        assert_eq!(keypad_index(k::_2), None);
        assert_eq!(keypad_index(k::KP_End), None);
    }

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
