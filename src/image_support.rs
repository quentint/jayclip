use gtk::gdk_pixbuf::{Colorspace, InterpType, Pixbuf};
use gtk::glib::Bytes;
use std::io::Cursor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pixels {
    pub bytes: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub rowstride: i32,
    pub has_alpha: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct EncodedImage {
    pub png: Vec<u8>,
    pub thumbnail: Vec<u8>,
    pub width: i32,
    pub height: i32,
}

pub fn copy_pixels(pixbuf: &Pixbuf) -> Pixels {
    Pixels {
        bytes: pixbuf.read_pixel_bytes().as_ref().to_vec(),
        width: pixbuf.width(),
        height: pixbuf.height(),
        rowstride: pixbuf.rowstride(),
        has_alpha: pixbuf.has_alpha(),
    }
}

pub fn pixbuf_from_pixels(pixels: Pixels) -> Pixbuf {
    Pixbuf::from_bytes(
        &Bytes::from_owned(pixels.bytes),
        Colorspace::Rgb,
        pixels.has_alpha,
        8,
        pixels.width,
        pixels.height,
        pixels.rowstride,
    )
}

pub fn encode_pixels(pixels: Pixels) -> Option<EncodedImage> {
    let pixbuf = pixbuf_from_pixels(pixels);
    let png = pixbuf.save_to_bufferv("png", &[]).ok()?;
    let scale = (48.0 / pixbuf.width() as f64)
        .min(48.0 / pixbuf.height() as f64)
        .min(1.0);
    let width = ((pixbuf.width() as f64 * scale).round() as i32).max(1);
    let height = ((pixbuf.height() as f64 * scale).round() as i32).max(1);
    let thumbnail = pixbuf
        .scale_simple(width, height, InterpType::Bilinear)?
        .save_to_bufferv("png", &[])
        .ok()?;
    Some(EncodedImage {
        png,
        thumbnail,
        width: pixbuf.width(),
        height: pixbuf.height(),
    })
}

pub fn decode_png(bytes: Vec<u8>) -> Option<Pixels> {
    let pixbuf = Pixbuf::from_read(Cursor::new(bytes)).ok()?;
    Some(copy_pixels(&pixbuf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pixbuf() -> Pixbuf {
        let pixbuf = Pixbuf::new(Colorspace::Rgb, true, 8, 96, 48).unwrap();
        pixbuf.fill(0xff3366ff);
        pixbuf
    }

    #[test]
    fn png_round_trip_and_thumbnail_dimensions() {
        let encoded = encode_pixels(copy_pixels(&sample_pixbuf())).unwrap();
        assert_eq!((encoded.width, encoded.height), (96, 48));
        let thumbnail = Pixbuf::from_read(Cursor::new(encoded.thumbnail)).unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (48, 24));
        let decoded = decode_png(encoded.png).unwrap();
        let round_trip = pixbuf_from_pixels(decoded);
        assert_eq!((round_trip.width(), round_trip.height()), (96, 48));
    }
}
