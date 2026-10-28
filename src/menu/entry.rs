use crate::db::Entry;
use gtk::prelude::*;

const LABEL_CHARS: usize = 60;

pub fn label(content: &str) -> String {
    let line = content
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = line.chars().take(LABEL_CHARS).collect();
    if line.chars().count() > LABEL_CHARS
        || content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            > 1
    {
        out.push('…');
    }
    out
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
    if entry.has_rich {
        markers.push_str("🖌️");
    }
    if entry.has_rich && entry.pinned {
        markers.push(' ');
    }
    if entry.pinned {
        markers.push('📌');
    }
    markers
}

fn content_widget(entry: &Entry) -> gtk::Box {
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
    content
}

fn marker_widget(entry: &Entry) -> gtk::Label {
    let markers = gtk::Label::new(Some(&entry_markers(entry)));
    markers.set_xalign(1.0);
    markers.set_halign(gtk::Align::End);
    markers
}

pub fn history_item(entry: &Entry, index: usize, item: &gtk::MenuItem) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let mnemonic = gtk::Label::new(None);
    mnemonic.set_use_underline(true);
    mnemonic.set_width_chars(3);
    mnemonic.set_xalign(0.0);
    if index < 9 {
        mnemonic.set_text_with_mnemonic(&format!("_{}", index + 1));
        mnemonic.set_mnemonic_widget(Some(item));
        item.add_mnemonic_label(&mnemonic);
    }
    row.pack_start(&mnemonic, false, false, 0);
    row.pack_start(&content_widget(entry), true, true, 0);
    row.pack_start(&marker_widget(entry), false, false, 0);
    row.upcast()
}

pub fn pin_item(entry: &Entry) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.pack_start(&content_widget(entry), true, true, 0);
    row.pack_start(&marker_widget(entry), false, false, 0);
    row.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_text_and_preserves_underscores() {
        assert_eq!(label("  foo   bar\tbaz "), "foo bar baz");
        assert_eq!(label("snake_case"), "snake_case");
    }

    #[test]
    fn marks_multiline_and_truncates() {
        assert_eq!(label("first\nsecond"), "first…");
        assert_eq!(label("\n\n  hi"), "hi");
        let long = "x".repeat(100);
        assert_eq!(label(&long).chars().count(), LABEL_CHARS + 1);
    }
}
