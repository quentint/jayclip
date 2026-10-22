use crate::State;
use crate::db::Formats;
use crate::image_support::{self, EncodedImage};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";
const HTML_TARGET: &str = "text/html";
const RTF_TARGETS: [&str; 2] = ["text/rtf", "application/rtf"];

pub fn clipboard() -> gtk::Clipboard {
    gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD)
}

fn is_own_write_ignored(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.is_some_and(|deadline| now < deadline)
}

fn capture_is_current(state: &State, write_generation: u64, recording_generation: u64) -> bool {
    !state.recording_paused.get()
        && state.recording_generation.get() == recording_generation
        && state.clipboard_write_generation.get() == write_generation
        && !is_own_write_ignored(state.ignore_until.get(), Instant::now())
}

pub fn start(state: Rc<State>) {
    clipboard().connect_local("owner-change", false, move |args| {
        let cb: gtk::Clipboard = args[0].get().expect("clipboard arg");
        let state = state.clone();
        if state.recording_paused.get()
            || is_own_write_ignored(state.ignore_until.get(), Instant::now())
        {
            return None;
        }
        let write_generation = state.clipboard_write_generation.get();
        let recording_generation = state.recording_generation.get();
        if state.do_not_store_passwords.get()
            && cb.wait_is_target_available(&gtk::gdk::Atom::intern(PASSWORD_HINT))
        {
            *state.last_secret.borrow_mut() = cb.wait_for_text().map(|t| t.to_string());
            return None;
        }

        let mut targets = Vec::new();
        if cb.wait_is_target_available(&gtk::gdk::Atom::intern(HTML_TARGET)) {
            targets.push(HTML_TARGET.to_string());
        }
        if let Some(target) = RTF_TARGETS
            .iter()
            .find(|target| cb.wait_is_target_available(&gtk::gdk::Atom::intern(target)))
        {
            targets.push((*target).to_string());
        }

        cb.request_text(move |cb, text| {
            if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
                let formats = Rc::new(RefCell::new(Formats {
                    text: Some(text.to_string()),
                    ..Formats::default()
                }));
                request_rich_formats(
                    cb.clone(),
                    targets,
                    0,
                    formats,
                    state,
                    write_generation,
                    recording_generation,
                );
                return;
            }
            if !targets.is_empty() {
                let formats = Rc::new(RefCell::new(Formats::default()));
                request_rich_formats(
                    cb.clone(),
                    targets,
                    0,
                    formats,
                    state,
                    write_generation,
                    recording_generation,
                );
                return;
            }
            request_png_image(cb.clone(), state, write_generation, recording_generation);
        });
        None
    });
}

fn request_rich_formats(
    cb: gtk::Clipboard,
    targets: Vec<String>,
    index: usize,
    formats: Rc<RefCell<Formats>>,
    state: Rc<State>,
    write_generation: u64,
    recording_generation: u64,
) {
    let Some(target) = targets.get(index).cloned() else {
        let captured = formats.borrow();
        if captured.text.is_none() && captured.html.is_none() && captured.rtf.is_none() {
            drop(captured);
            request_png_image(cb, state, write_generation, recording_generation);
        } else {
            drop(captured);
            finish_record(&state, formats, write_generation, recording_generation);
        }
        return;
    };
    let is_html = target == HTML_TARGET;
    cb.request_contents(&gtk::gdk::Atom::intern(&target), move |cb, data| {
        let bytes = data.data();
        if !bytes.is_empty() {
            let mut formats = formats.borrow_mut();
            if is_html {
                formats.html = Some(bytes);
            } else {
                formats.rtf = Some(bytes);
            }
        }
        request_rich_formats(
            cb.clone(),
            targets,
            index + 1,
            formats,
            state,
            write_generation,
            recording_generation,
        );
    });
}

fn request_png_image(
    clipboard: gtk::Clipboard,
    state: Rc<State>,
    write_generation: u64,
    recording_generation: u64,
) {
    clipboard.request_image(move |_, pixbuf| {
        let Some(pixbuf) = pixbuf else {
            return;
        };
        capture_image(
            image_support::copy_pixels(pixbuf),
            state,
            write_generation,
            recording_generation,
        );
    });
}

fn capture_image(
    pixels: image_support::Pixels,
    state: Rc<State>,
    write_generation: u64,
    recording_generation: u64,
) {
    glib::MainContext::default().spawn_local(async move {
        let Ok(Some(encoded)) =
            gtk::gio::spawn_blocking(move || image_support::encode_pixels(pixels)).await
        else {
            return;
        };
        if !capture_is_current(&state, write_generation, recording_generation) {
            return;
        }
        let formats = formats_from_image(encoded);
        if let Err(e) = state.db.upsert_formats(&formats) {
            eprintln!("jayclip: db error: {e}");
        }
    });
}

fn formats_from_image(encoded: EncodedImage) -> Formats {
    Formats {
        image: Some(encoded.png),
        thumb: Some(encoded.thumbnail),
        width: Some(encoded.width),
        height: Some(encoded.height),
        ..Formats::default()
    }
}

fn finish_record(
    state: &State,
    formats: Rc<RefCell<Formats>>,
    write_generation: u64,
    recording_generation: u64,
) {
    if !capture_is_current(state, write_generation, recording_generation) {
        return;
    }
    let formats = formats.borrow().clone();
    if formats
        .text
        .as_deref()
        .is_some_and(|text| text.trim().is_empty())
    {
        return;
    }
    if formats.text.is_none() && formats.html.is_none() && formats.rtf.is_none() {
        return;
    }
    if formats
        .text
        .as_deref()
        .is_some_and(|text| state.last_secret.borrow().as_deref() == Some(text))
    {
        return;
    }
    state.last_secret.borrow_mut().take();
    if let Err(e) = state.db.upsert_formats(&formats) {
        eprintln!("jayclip: db error: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn should_record(captured_generation: u64, current_generation: u64) -> bool {
        captured_generation == current_generation
    }

    #[test]
    fn paused_or_invalidated_capture_is_not_current() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let state = State {
            db,
            do_not_store_passwords: std::cell::Cell::new(true),
            recording_paused: std::cell::Cell::new(false),
            recording_generation: std::cell::Cell::new(2),
            ignore_until: std::cell::Cell::new(None),
            clipboard_write_generation: std::cell::Cell::new(3),
            last_secret: std::cell::RefCell::new(None),
            menu_active: std::cell::Cell::new(false),
        };
        assert!(capture_is_current(&state, 3, 2));
        state.recording_paused.set(true);
        assert!(!capture_is_current(&state, 3, 2));
        state.recording_paused.set(false);
        state.recording_generation.set(3);
        assert!(!capture_is_current(&state, 3, 2));
        assert!(!capture_is_current(&state, 4, 3));
    }

    #[test]
    fn own_clipboard_write_invalidates_inflight_capture() {
        assert!(should_record(4, 4));
        assert!(!should_record(4, 5));
    }

    #[test]
    fn own_write_window_ignores_only_events_before_deadline() {
        let now = Instant::now();
        let deadline = now + crate::SELF_WRITE_WINDOW;
        assert!(is_own_write_ignored(Some(deadline), now));
        assert!(!is_own_write_ignored(Some(deadline), deadline));
        assert!(!is_own_write_ignored(None, now));
    }
}
