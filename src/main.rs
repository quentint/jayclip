use gtk::gio::{self, prelude::*};
use gtk::glib;
use gtk::prelude::*;

const APP_ID: &str = "dev.quentin.DiodonNext";

fn main() -> glib::ExitCode {
    let app = gtk::Application::new(
        Some(APP_ID),
        gio::ApplicationFlags::HANDLES_COMMAND_LINE,
    );

    // Keep the process alive without any window.
    let guard = std::cell::RefCell::new(None);
    app.connect_startup(move |app| {
        *guard.borrow_mut() = Some(app.hold());
    });

    app.connect_activate(|_| {});

    // Runs in the primary instance for every invocation, local or remote.
    app.connect_command_line(|app, cmd| {
        let args: Vec<String> = cmd
            .arguments()
            .iter()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        match args.first().map(String::as_str) {
            Some("daemon") => {}
            Some("show") => show_menu(app),
            Some("quit") => app.quit(),
            _ => {
                eprintln!("usage: diodon-next <daemon|show|quit>");
                return 2;
            }
        }
        0
    });

    app.run()
}

fn show_menu(_app: &gtk::Application) {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let Some(pointer) = display.default_seat().and_then(|s| s.pointer()) else {
        return;
    };
    let (_, x, y) = pointer.position();

    // Tiny undecorated helper window at the pointer, used as the menu anchor.
    let helper = gtk::Window::new(gtk::WindowType::Toplevel);
    helper.set_decorated(false);
    helper.set_skip_taskbar_hint(true);
    helper.set_skip_pager_hint(true);
    helper.set_default_size(1, 1);
    helper.set_type_hint(gtk::gdk::WindowTypeHint::PopupMenu);
    helper.move_(x, y);
    helper.show_all();

    let menu = gtk::Menu::new();
    for i in 1..=5 {
        let item = gtk::MenuItem::with_mnemonic(&format!("_{i} dummy entry {i}"));
        item.connect_activate(move |_| println!("selected {i}"));
        menu.append(&item);
    }
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
