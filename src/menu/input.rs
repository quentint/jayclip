pub fn plain_paste_for_event(
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

pub fn shift_enter(key: gtk::gdk::keys::Key, state: gtk::gdk::ModifierType) -> bool {
    use gtk::gdk::keys::constants as k;
    state.contains(gtk::gdk::ModifierType::SHIFT_MASK)
        && matches!(key, k::Return | k::KP_Enter | k::ISO_Enter)
}

pub fn current_event_requests_plain_paste() -> bool {
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

pub fn digit_index(key: gtk::gdk::keys::Key) -> Option<usize> {
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

pub fn keypad_index(key: gtk::gdk::keys::Key) -> Option<usize> {
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
    .position(|candidate| *candidate == key)
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
    fn digit_shortcuts_map_to_display_positions() {
        use gtk::gdk::keys::constants as k;
        assert_eq!(digit_index(k::_1), Some(0));
        assert_eq!(digit_index(k::_9), Some(8));
        assert_eq!(digit_index(k::KP_2), None);
        assert_eq!(keypad_index(k::KP_1), Some(0));
        assert_eq!(keypad_index(k::KP_9), Some(8));
        assert_eq!(keypad_index(k::KP_0), None);
    }
}
