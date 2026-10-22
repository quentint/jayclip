use crate::State;
use crate::db::{Entry, MAX_ENTRIES};
use crate::image_support;
use crate::{clipboard_out, paste, watcher};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

const LABEL_CHARS: usize = 60;

fn plain_paste_for_event(
    event_type: gtk::gdk::EventType,
    key: Option<gtk::gdk::keys::Key>,
    state: gtk::gdk::ModifierType,
) -> bool {
    if !state.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
        return false;
    }
    match event_type {
        gtk::gdk::EventType::KeyPress => {
            use gtk::gdk::keys::constants as k;
            matches!(key, Some(k::Return | k::KP_Enter | k::ISO_Enter))
        }
        gtk::gdk::EventType::ButtonRelease => true,
        _ => false,
    }
}

fn shift_enter(key: gtk::gdk::keys::Key, state: gtk::gdk::ModifierType) -> bool {
    use gtk::gdk::keys::constants as k;
    state.contains(gtk::gdk::ModifierType::SHIFT_MASK)
        && matches!(key, k::Return | k::KP_Enter | k::ISO_Enter)
}

fn current_event_requests_plain_paste() -> bool {
    let Some(event) = gtk::current_event() else {
        return false;
    };
    if let Some(key_event) = event.downcast_ref::<gtk::gdk::EventKey>() {
        return shift_enter(key_event.keyval(), key_event.state());
    }
    plain_paste_for_event(
        event.event_type(),
        None,
        event.state().unwrap_or_else(gtk::gdk::ModifierType::empty),
    )
}

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
    if line.chars().count() > LABEL_CHARS
        || content.lines().filter(|l| !l.trim().is_empty()).count() > 1
    {
        out.push('…');
    }
    out.replace('_', "__")
}

/// Keypad digit 1-9 to a zero-based entry index.
fn digit_index(key: gtk::gdk::keys::Key) -> Option<usize> {
    use gtk::gdk::keys::constants as k;
    [
        k::_1,
        k::_2,
        k::_3,
        k::_4,
        k::_5,
        k::_6,
        k::_7,
        k::_8,
        k::_9,
    ]
    .iter()
    .position(|digit| *digit == key)
}

fn keypad_index(key: gtk::gdk::keys::Key) -> Option<usize> {
    use gtk::gdk::keys::constants as k;
    [
        k::KP_1,
        k::KP_2,
        k::KP_3,
        k::KP_4,
        k::KP_5,
        k::KP_6,
        k::KP_7,
        k::KP_8,
        k::KP_9,
    ]
    .iter()
    .position(|c| *c == key)
}

fn pixbuf_from_png(bytes: &[u8]) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(bytes.to_vec())).ok()
}

fn entry_text(entry: &Entry) -> String {
    if entry.is_image {
        match (entry.width, entry.height) {
            (Some(width), Some(height)) => format!("Image {width}×{height}"),
            _ => "Image".to_string(),
        }
    } else {
        label(&entry.content)
    }
}

fn entry_markers(entry: &Entry) -> String {
    let mut markers = String::new();
    if entry.pinned {
        markers.push('📌');
    }
    if entry.has_rich {
        markers.push('🤖');
    }
    markers
}

fn history_menu_item(entry: &Entry, index: usize) -> gtk::MenuItem {
    let item = gtk::MenuItem::new();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);

    let mnemonic = gtk::Label::new(None);
    mnemonic.set_use_underline(true);
    mnemonic.set_width_chars(3);
    mnemonic.set_xalign(0.0);
    if index < 9 {
        mnemonic.set_text_with_mnemonic(&format!("_{}", index + 1));
        mnemonic.set_mnemonic_widget(Some(&item));
        item.add_mnemonic_label(&mnemonic);
    }
    row.pack_start(&mnemonic, false, false, 0);

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if entry.is_image
        && let Some(bytes) = entry.thumb.as_deref()
        && let Some(pixbuf) = pixbuf_from_png(bytes)
    {
        content.pack_start(&gtk::Image::from_pixbuf(Some(&pixbuf)), false, false, 0);
    }
    let text = gtk::Label::new(Some(&entry_text(entry)));
    text.set_xalign(0.0);
    text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.set_max_width_chars(LABEL_CHARS as i32);
    content.pack_start(&text, true, true, 0);
    row.pack_start(&content, true, true, 0);

    let markers = gtk::Label::new(Some(&entry_markers(entry)));
    markers.set_xalign(1.0);
    markers.set_halign(gtk::Align::End);
    row.pack_start(&markers, false, false, 0);

    item.add(&row);
    item
}

fn entry_label(entry: &Entry) -> String {
    let text = entry_text(entry);
    let markers = entry_markers(entry);
    if markers.is_empty() {
        text
    } else {
        format!("{markers} {text}")
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
        let item = history_menu_item(entry, i);
        let entry_id = entry.id;
        let state = state.clone();
        let target = target.clone();
        let pasting = pasting.clone();
        let on_finish = on_finish.clone();
        item.connect_activate(move |_| {
            let formats = match state.db.get_formats(entry_id) {
                Ok(Some(formats)) => formats,
                Ok(None) => {
                    eprintln!("diodon-next: entry not found: {entry_id}");
                    return;
                }
                Err(e) => {
                    eprintln!("diodon-next: db error: {e}");
                    return;
                }
            };
            if let Err(e) = state.db.touch(entry_id) {
                eprintln!("diodon-next: db error: {e}");
            }
            let cb = watcher::clipboard();
            let plain_paste = current_event_requests_plain_paste();
            if let Some(image) = formats.image.clone() {
                let target = target.borrow_mut().take();
                let on_finish = on_finish.clone();
                let state = state.clone();
                pasting.set(true);
                glib::spawn_future_local(async move {
                    let pixels =
                        gtk::gio::spawn_blocking(move || image_support::decode_png(image)).await;
                    match pixels {
                        Ok(Some(pixels)) => {
                            let pixbuf = image_support::pixbuf_from_pixels(pixels);
                            state
                                .ignore_until
                                .set(Some(std::time::Instant::now() + crate::SELF_WRITE_WINDOW));
                            watcher::clipboard().set_image(&pixbuf);
                            paste::send_paste(target).await;
                        }
                        Ok(None) => eprintln!("diodon-next: could not decode stored image"),
                        Err(e) => eprintln!("diodon-next: image decode task failed: {e:?}"),
                    }
                    glib::timeout_future(GRACE).await;
                    on_finish();
                });
                return;
            }
            if plain_paste {
                if let Some(text) = formats.text.as_deref() {
                    state
                        .clipboard_write_generation
                        .set(state.clipboard_write_generation.get().wrapping_add(1));
                    state
                        .ignore_until
                        .set(Some(std::time::Instant::now() + crate::SELF_WRITE_WINDOW));
                    cb.set_text(text);
                } else {
                    clipboard_out::set_rich(&cb, formats.clone());
                }
            } else if !clipboard_out::set_rich(&cb, formats.clone())
                && let Some(text) = formats.text.as_deref()
            {
                cb.set_text(text);
            }
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
        let modifiers = event.state();
        if shift_enter(event.keyval(), modifiers)
            && let Some(item) = menu.selected_item()
            && let Ok(item) = item.downcast::<gtk::MenuItem>()
        {
            menu.popdown();
            item.activate();
            return glib::Propagation::Stop;
        }
        match keypad_index(event.keyval())
            .or_else(|| digit_index(event.keyval()))
            .and_then(|i| items.get(i))
        {
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
    fn plain_paste_is_only_shift_enter_or_shift_click() {
        use gtk::gdk::keys::constants as k;
        use gtk::gdk::{EventType, ModifierType};
        let shift = ModifierType::SHIFT_MASK;

        assert!(plain_paste_for_event(
            EventType::KeyPress,
            Some(k::Return),
            shift
        ));
        assert!(plain_paste_for_event(
            EventType::KeyPress,
            Some(k::KP_Enter),
            shift
        ));
        assert!(plain_paste_for_event(
            EventType::KeyPress,
            Some(k::ISO_Enter),
            shift
        ));
        assert!(plain_paste_for_event(EventType::ButtonRelease, None, shift));

        assert!(shift_enter(k::Return, shift));
        assert!(shift_enter(k::KP_Enter, shift));
        assert!(shift_enter(k::ISO_Enter, shift));
        assert!(!shift_enter(k::Return, ModifierType::empty()));
        assert!(!shift_enter(k::_2, shift));

        assert!(!plain_paste_for_event(
            EventType::KeyPress,
            Some(k::Return),
            ModifierType::empty()
        ));
        assert!(!plain_paste_for_event(
            EventType::KeyPress,
            Some(k::_2),
            shift
        ));
        assert!(!plain_paste_for_event(
            EventType::KeyPress,
            Some(k::KP_2),
            shift
        ));
        assert!(!plain_paste_for_event(
            EventType::KeyRelease,
            Some(k::Return),
            shift
        ));
        assert!(!plain_paste_for_event(
            EventType::ButtonRelease,
            None,
            ModifierType::empty()
        ));
    }

    #[test]
    fn keypad_digits_map_to_positions() {
        use gtk::gdk::keys::constants as k;
        assert_eq!(keypad_index(k::KP_1), Some(0));
        assert_eq!(keypad_index(k::KP_9), Some(8));
        assert_eq!(keypad_index(k::KP_0), None);
        assert_eq!(keypad_index(k::_2), None);
        assert_eq!(digit_index(k::_1), Some(0));
        assert_eq!(digit_index(k::_9), Some(8));
        assert_eq!(digit_index(k::KP_2), None);
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
