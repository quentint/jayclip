mod entry;
mod input;
mod popup;

use crate::State;
use crate::clipboard_out;
use crate::image_support;
use crate::{paste, settings, watcher};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const GRACE: std::time::Duration = std::time::Duration::from_millis(500);

fn pin_submenu(state: &Rc<State>, entries: &[crate::db::Entry]) -> gtk::MenuItem {
    let parent = gtk::MenuItem::with_mnemonic("_Pin / unpin");
    let sub = gtk::Menu::new();
    for entry in entries {
        let item = gtk::MenuItem::new();
        item.add(&entry::pin_item(entry));
        let (state, id) = (state.clone(), entry.id);
        item.connect_activate(move |_| {
            if let Err(e) = state.db.toggle_pin(id) {
                eprintln!("jayclip: db error: {e}");
            }
        });
        sub.append(&item);
    }
    parent.set_submenu(Some(&sub));
    parent
}

pub fn show(state: Rc<State>, on_finish: Rc<dyn Fn()>) {
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

    let helper = gtk::Window::new(gtk::WindowType::Toplevel);
    helper.set_decorated(false);
    helper.set_skip_taskbar_hint(true);
    helper.set_skip_pager_hint(true);
    helper.set_default_size(1, 1);
    helper.set_type_hint(gtk::gdk::WindowTypeHint::PopupMenu);
    helper.move_(x, y);
    helper.show_all();

    let menu = gtk::Menu::new();
    let entries = state.db.recent_display().unwrap_or_default();
    let target = Rc::new(RefCell::new(target));
    let pasting = Rc::new(Cell::new(false));
    let on_hide = {
        let pasting = pasting.clone();
        let on_finish = on_finish.clone();
        Rc::new(move || {
            if !pasting.get() {
                on_finish();
            }
        }) as Rc<dyn Fn()>
    };

    if entries.is_empty() {
        let item = gtk::MenuItem::with_label("(empty)");
        item.set_sensitive(false);
        menu.append(&item);
    }

    let mut separated = false;
    let mut items = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 && !entry.pinned && entries[index - 1].pinned && !separated {
            menu.append(&gtk::SeparatorMenuItem::new());
            separated = true;
        }
        let item = gtk::MenuItem::new();
        item.add(&entry::history_item(entry, index, &item));
        connect_paste_item(
            &menu,
            &item,
            entry.id,
            state.clone(),
            target.clone(),
            pasting.clone(),
            on_finish.clone(),
        );
        menu.append(&item);
        items.push(item);
    }

    menu.connect_key_press_event(move |menu, event| {
        if input::shift_enter(event.keyval(), event.state())
            && let Some(item) = menu.selected_item()
            && let Ok(item) = item.downcast::<gtk::MenuItem>()
        {
            menu.popdown();
            item.activate();
            return glib::Propagation::Stop;
        }
        match input::keypad_index(event.keyval())
            .or_else(|| input::digit_index(event.keyval()))
            .and_then(|index| items.get(index))
        {
            Some(item) => {
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
    menu.append(&settings::submenu(state.clone()));
    menu.show_all();
    popup::popup(menu, helper, on_hide, on_finish);
}

fn connect_paste_item(
    menu: &gtk::Menu,
    item: &gtk::MenuItem,
    entry_id: i64,
    state: Rc<State>,
    target: Rc<RefCell<Option<paste::Target>>>,
    pasting: Rc<Cell<bool>>,
    on_finish: Rc<dyn Fn()>,
) {
    let menu = menu.clone();
    item.connect_activate(move |_| {
        let formats = match state.db.get_formats(entry_id) {
            Ok(Some(formats)) => formats,
            Ok(None) => {
                eprintln!("jayclip: entry not found: {entry_id}");
                return;
            }
            Err(e) => {
                eprintln!("jayclip: db error: {e}");
                return;
            }
        };
        if let Err(e) = state.db.touch(entry_id) {
            eprintln!("jayclip: db error: {e}");
        }
        let cb = watcher::clipboard();
        let plain_paste = input::current_event_requests_plain_paste();
        if !plain_paste {
            menu.popdown();
        }
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
                        state
                            .ignore_until
                            .set(Some(std::time::Instant::now() + crate::SELF_WRITE_WINDOW));
                        let pixbuf = image_support::pixbuf_from_pixels(pixels);
                        watcher::clipboard().set_image(&pixbuf);
                        paste::send_paste(target).await;
                    }
                    Ok(None) => eprintln!("jayclip: could not decode stored image"),
                    Err(e) => eprintln!("jayclip: image decode task failed: {e:?}"),
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
}
