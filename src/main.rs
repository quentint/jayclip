mod db;
mod menu;
mod paste;
mod watcher;

use gtk::gio::{self, prelude::*};
use gtk::glib;
use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

pub struct State {
    pub db: db::Db,
    /// Text we put on the clipboard ourselves, so the watcher can ignore it.
    pub last_set: RefCell<Option<String>>,
}

const APP_ID: &str = "dev.quentin.DiodonNext";

fn main() -> glib::ExitCode {
    let app = gtk::Application::new(
        Some(APP_ID),
        gio::ApplicationFlags::HANDLES_COMMAND_LINE,
    );

    // Keep the process alive without any window.
    let state: Rc<OnceCell<Rc<State>>> = Rc::new(OnceCell::new());
    let guard = RefCell::new(None);
    {
        let state = state.clone();
        app.connect_startup(move |app| {
            *guard.borrow_mut() = Some(app.hold());
            let db = match db::Db::open() {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("diodon-next: cannot open database: {e}");
                    std::process::exit(1);
                }
            };
            let st = Rc::new(State {
                db,
                last_set: RefCell::new(None),
            });
            watcher::start(st.clone());
            let _ = state.set(st);
        });
    }

    app.connect_activate(|_| {});

    // Runs in the primary instance for every invocation, local or remote.
    app.connect_command_line(move |app, cmd| {
        let args: Vec<String> = cmd
            .arguments()
            .iter()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        match args.first().map(String::as_str) {
            Some("daemon") => {}
            Some("show") => {
                if let Some(st) = state.get() {
                    menu::show(st.clone());
                }
            }
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
