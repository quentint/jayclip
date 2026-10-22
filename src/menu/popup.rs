use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::Duration;

pub fn popup(menu: gtk::Menu, helper: gtk::Window, on_hide: Rc<dyn Fn()>, on_finish: Rc<dyn Fn()>) {
    let shown = Rc::new(std::cell::Cell::new(false));
    {
        let shown = shown.clone();
        menu.connect_map(move |_| shown.set(true));
    }
    {
        let helper = helper.clone();
        let shown = shown.clone();
        let on_hide = on_hide.clone();
        menu.connect_hide(move |_| {
            if shown.get() {
                let (helper, on_hide) = (helper.clone(), on_hide.clone());
                glib::idle_add_local_once(move || {
                    helper.close();
                    on_hide();
                });
            }
        });
    }
    popup_when_grabbable(menu, helper, shown, on_finish, 0);
}

fn popup_when_grabbable(
    menu: gtk::Menu,
    helper: gtk::Window,
    shown: Rc<std::cell::Cell<bool>>,
    on_finish: Rc<dyn Fn()>,
    attempt: u32,
) {
    const RETRY: Duration = Duration::from_millis(25);
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
