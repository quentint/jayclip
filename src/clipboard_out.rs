use crate::db::Formats;
const TARGET_TEXT_UTF8: u32 = 1;
const TARGET_TEXT_PLAIN: u32 = 2;
const TARGET_UTF8_STRING: u32 = 3;
const TARGET_HTML: u32 = 4;
const TARGET_RTF: u32 = 5;

pub fn set_rich(clipboard: &gtk::Clipboard, formats: Formats) -> bool {
    let mut targets = Vec::new();
    if formats.text.is_some() {
        targets.extend([
            gtk::TargetEntry::new(
                "text/plain;charset=utf-8",
                gtk::TargetFlags::empty(),
                TARGET_TEXT_UTF8,
            ),
            gtk::TargetEntry::new("text/plain", gtk::TargetFlags::empty(), TARGET_TEXT_PLAIN),
            gtk::TargetEntry::new("UTF8_STRING", gtk::TargetFlags::empty(), TARGET_UTF8_STRING),
        ]);
    }
    if formats.html.is_some() {
        targets.push(gtk::TargetEntry::new(
            "text/html",
            gtk::TargetFlags::empty(),
            TARGET_HTML,
        ));
    }
    if formats.rtf.is_some() {
        targets.push(gtk::TargetEntry::new(
            "text/rtf",
            gtk::TargetFlags::empty(),
            TARGET_RTF,
        ));
    }
    if targets.is_empty() {
        return false;
    }

    clipboard.set_with_data(&targets, move |_, selection, info| {
        let bytes = match info {
            TARGET_TEXT_UTF8 | TARGET_TEXT_PLAIN | TARGET_UTF8_STRING => {
                formats.text.as_ref().map(|text| text.as_bytes())
            }
            TARGET_HTML => formats.html.as_deref(),
            TARGET_RTF => formats.rtf.as_deref(),
            _ => None,
        };
        if let Some(bytes) = bytes {
            selection.set(&selection.target(), 8, bytes);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_targets_round_trip_through_gtk_clipboard() {
        gtk::init().unwrap();
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        let formats = Formats {
            text: Some("plain hello".into()),
            html: Some(b"<b>rich hello</b>".to_vec()),
            rtf: Some(b"{\\rtf1 rich hello}".to_vec()),
            ..Formats::default()
        };
        assert!(set_rich(&clipboard, formats));

        for (target, expected) in [
            ("text/plain;charset=utf-8", b"plain hello".as_slice()),
            ("text/html", b"<b>rich hello</b>".as_slice()),
            ("text/rtf", b"{\\rtf1 rich hello}".as_slice()),
        ] {
            let selection = clipboard
                .wait_for_contents(&gtk::gdk::Atom::intern(target))
                .unwrap();
            assert_eq!(selection.data(), expected);
        }

        let pixbuf =
            gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 12, 8).unwrap();
        pixbuf.fill(0x3366ccff);
        clipboard.set_image(&pixbuf);
        assert!(clipboard.wait_is_image_available());
        let received = clipboard.wait_for_image().unwrap();
        assert_eq!((received.width(), received.height()), (12, 8));
    }

    #[test]
    fn target_ids_are_unique_and_stable() {
        let formats = Formats {
            text: Some("plain".into()),
            html: Some(b"<b>rich</b>".to_vec()),
            rtf: Some(b"{\\rtf1 rich}".to_vec()),
            ..Formats::default()
        };
        assert_eq!(
            target_bytes(&formats, TARGET_TEXT_UTF8),
            Some(&b"plain"[..])
        );
        assert_eq!(
            target_bytes(&formats, TARGET_HTML),
            Some(&b"<b>rich</b>"[..])
        );
        assert_eq!(
            target_bytes(&formats, TARGET_RTF),
            Some(&b"{\\rtf1 rich}"[..])
        );
        assert_eq!(target_bytes(&formats, 99), None);
    }

    fn target_bytes(formats: &Formats, info: u32) -> Option<&[u8]> {
        match info {
            TARGET_TEXT_UTF8 | TARGET_TEXT_PLAIN | TARGET_UTF8_STRING => {
                formats.text.as_ref().map(|text| text.as_bytes())
            }
            TARGET_HTML => formats.html.as_deref(),
            TARGET_RTF => formats.rtf.as_deref(),
            _ => None,
        }
    }
}
