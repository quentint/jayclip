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
    /// Text of the last copy skipped for carrying the password-manager hint.
    pub last_secret: RefCell<Option<String>>,
    /// True from `show` until its interaction ends; blocks overlapping menus.
    pub menu_active: std::cell::Cell<bool>,
}

const APP_ID: &str = "dev.quentin.DiodonNext";

/// GTK warns on every failed menu grab; `menu.rs` retries on purpose while the
/// shortcut keys are held, so that one message is just noise. GLib 2.80 routes
/// these through the structured log writer, so filter there and pass every
/// other message to the default writer unchanged.
fn quiet_known_warnings() {
    glib::log_set_writer_func(|level, fields| {
        let noisy = fields.iter().any(|f| {
            f.key() == "MESSAGE"
                && f.value_str()
                    .is_some_and(|m| m.contains("no trigger event for menu popup"))
        });
        if noisy {
            glib::LogWriterOutput::Handled
        } else {
            glib::log_writer_default(level, fields)
        }
    });
}

fn main() -> glib::ExitCode {
    quiet_known_warnings();
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
                last_secret: RefCell::new(None),
                menu_active: std::cell::Cell::new(false),
            });
            watcher::start(st.clone());
            let _ = state.set(st);
        });
    }

    app.connect_activate(|_| {});

    // Runs in the primary instance for every invocation, local or remote.
    let cmd_state = state.clone();
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
                if let Some(st) = cmd_state.get() {
                    // Not remote means no daemon was running and this process
                    // became the primary instance. Do not linger as a hidden
                    // daemon: exit once the menu interaction is over.
                    let on_finish: Rc<dyn Fn()> = if cmd.is_remote() {
                        Rc::new(|| {})
                    } else {
                        let app = app.clone();
                        Rc::new(move || app.quit())
                    };
                    menu::show(st.clone(), on_finish);
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

    // Quit through the main loop on SIGTERM/SIGINT so we can clean up below.
    for signum in [2, 15] {
        let app = app.clone();
        glib::unix_signal_add_local(signum, move || {
            app.quit();
            glib::ControlFlow::Break
        });
    }

    let code = app.run();
    if let Some(st) = state.get() {
        st.db.checkpoint();
    }
    code
}
