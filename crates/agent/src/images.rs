//! Images as a model takes them inline, after pi's. A provider refuses a whole request over an
//! image it does not take, and since the image stays in the chat, every later turn fails too.
//! So every image Lorca shows a model (a tool's, a script's, an attachment's) goes through
//! `prepare`: read whatever its format, turned upright, scaled down to fit 2000×2000 pixels,
//! and written as a PNG or a JPEG small enough, with a note for the model when it changed.

use std::io::Cursor;

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageError, ImageFormat, ImageReader, Rgb, RgbImage};
use serde::{Deserialize, Serialize};

use crate::types::ContentPart;

/// The longest side an image goes to a model with, as in pi; a larger one is scaled down to fit.
/// Anthropic refuses an image over 2000 pixels on a side in a request with more than 20 images,
/// and scales one past 1568 down itself anyway.
pub const MAX_SIDE: u32 = 2000;

/// The most base64 an image takes, as in pi: room under the 5 MB Anthropic refuses an image over.
pub const MAX_BASE64: usize = 4608 * 1024;

/// The most memory decoding one image may take, the image crate's own default.
const MAX_DECODED_BYTES: u64 = 512 * 1024 * 1024;

/// The JPEG qualities tried in turn when the smaller of the PNG and the first JPEG is too large.
const JPEG_QUALITIES: [u8; 4] = [80, 70, 55, 40];

/// An image ready for a model inline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inline {
    /// The image's bytes in base64.
    pub data: String,
    pub mime_type: String,
    /// For the model, when the image changed on the way: what it was converted from, or its
    /// original size and how to map coordinates back to it, as pi words them.
    pub note: Option<String>,
}

impl Inline {
    /// The image as content parts: its note, when it has one, and then the image.
    pub fn into_parts(self) -> Vec<ContentPart> {
        let mut parts = Vec::with_capacity(2);
        if let Some(note) = self.note {
            parts.push(ContentPart::text(note));
        }
        parts.push(ContentPart::Image { data: self.data, mime_type: self.mime_type });
        parts
    }
}

/// Any image this computer reads, made into one a model takes. A PNG, JPEG, or WebP that is
/// upright, within 2000×2000, and under 4.5 MB of base64 goes as it is. Anything else (a BMP, a
/// GIF, a TIFF, a HEIC on a Mac, a photo its camera turned, a large screenshot) is decoded,
/// turned upright, scaled down to fit, and written as a PNG or a JPEG, whichever is smaller, at
/// lower JPEG qualities and then smaller sizes while it is still too large. The error says why the
/// image cannot go. A large image takes a moment: call this off the async threads.
pub fn prepare(bytes: &[u8]) -> Result<Inline, String> {
    Ok(made(bytes, MAX_SIDE, MAX_BASE64)?.inline(|| base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// [`prepare`] for base64 data, which an image that already fits keeps as it is.
pub fn prepare_base64(data: &str) -> Result<Inline, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "its data is not base64".to_string())?;
    Ok(made(&bytes, MAX_SIDE, MAX_BASE64)?.inline(|| data.to_string()))
}

/// The type of an image file by its first bytes, for a tool that tells images from text: the
/// formats `prepare` reads, and HEIC and AVIF, which it reads on a Mac.
pub fn file_type(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        [b'I', b'I', b'*', 0, ..] | [b'M', b'M', 0, b'*', ..] => Some("image/tiff"),
        _ if is_bmp(bytes) => Some("image/bmp"),
        _ => heif_type(bytes),
    }
}

/// A BMP by its whole file header, its reserved bytes zero and a known size of the header after
/// it, so a text file that starts with "BM" is not one.
fn is_bmp(bytes: &[u8]) -> bool {
    bytes.len() >= 18 && bytes.starts_with(b"BM") && bytes[6..10] == [0; 4] && matches!(u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]), 12 | 40 | 52 | 56 | 64 | 108 | 124)
}

/// HEIC or AVIF, by the brand of the ISO media file the photos of Apple's cameras are.
fn heif_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        return None;
    }
    match &bytes[8..12] {
        b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => Some("image/heic"),
        b"avif" | b"avis" => Some("image/avif"),
        _ => None,
    }
}

/// What `prepare` made of an image: the image as it is, of this type, or new bytes.
enum Made {
    AsIs(&'static str),
    New { bytes: Vec<u8>, mime_type: &'static str, note: Option<String> },
}

impl Made {
    fn inline(self, original: impl FnOnce() -> String) -> Inline {
        match self {
            Made::AsIs(mime_type) => Inline { data: original(), mime_type: mime_type.into(), note: None },
            Made::New { bytes, mime_type, note } => Inline { data: base64::engine::general_purpose::STANDARD.encode(bytes), mime_type: mime_type.into(), note },
        }
    }
}

fn base64_len(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

fn made(bytes: &[u8], max_side: u32, max_base64: usize) -> Result<Made, String> {
    let (image, format, orientation) = read(bytes)?;
    let source = format.map(|format| format.to_mime_type()).or_else(|| heif_type(bytes)).unwrap_or("an image");
    let as_is = matches!(format, Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP));
    let (width, height) = image.dimensions();
    if as_is && orientation == Orientation::NoTransforms && width.max(height) <= max_side && base64_len(bytes.len()) <= max_base64 {
        return Ok(Made::AsIs(source));
    }
    let mut image = image;
    image.apply_orientation(orientation);
    // Eight bits a channel, with or without alpha: all a model needs, and what PNG and JPEG take.
    let image = if image.color().has_alpha() { DynamicImage::ImageRgba8(image.to_rgba8()) } else { DynamicImage::ImageRgb8(image.to_rgb8()) };
    let (width, height) = image.dimensions();
    // A photo comes out smaller as a JPEG every time, and writing a PNG of one is the slow part.
    let photo = format == Some(ImageFormat::Jpeg) || heif_type(bytes).is_some();
    let (mut w, mut h) = fit(width, height, max_side);
    loop {
        let scaled = ((w, h) != (width, height)).then(|| image.resize_exact(w, h, FilterType::Lanczos3));
        if let Some((bytes, mime_type)) = smallest(scaled.as_ref().unwrap_or(&image), max_base64, !photo)? {
            let mut notes = Vec::new();
            if !as_is {
                notes.push(format!("[Image converted from {source} to {mime_type}.]"));
            }
            if (w, h) != (width, height) {
                let scale = f64::from(width) / f64::from(w);
                notes.push(format!("[Image: original {width}x{height}, displayed at {w}x{h}. Multiply coordinates by {scale:.2} to map to original image.]"));
            }
            return Ok(Made::New { bytes, mime_type, note: (!notes.is_empty()).then(|| notes.join("\n")) });
        }
        if (w, h) == (1, 1) {
            return Err("it could not be made small enough".into());
        }
        (w, h) = ((w * 3 / 4).max(1), (h * 3 / 4).max(1));
    }
}

/// The size within `max_side` on both sides, its shape kept.
fn fit(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max_side {
        return (width, height);
    }
    let scale = |side: u32| ((u64::from(side) * u64::from(max_side) + u64::from(longest) / 2) / u64::from(longest)).max(1) as u32;
    (scale(width), scale(height))
}

/// The image as a PNG (when `try_png`) or a JPEG, whichever is smaller, when that fits; else
/// the first lower JPEG quality that does.
fn smallest(image: &DynamicImage, max_base64: usize, try_png: bool) -> Result<Option<(Vec<u8>, &'static str)>, String> {
    let rgb = DynamicImage::ImageRgb8(on_white(image));
    let jpeg = |quality: u8| -> Result<Vec<u8>, String> {
        let mut jpeg = Vec::new();
        rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut jpeg, quality)).map_err(|e| format!("it could not be written as a JPEG: {e}"))?;
        Ok(jpeg)
    };
    let first = jpeg(JPEG_QUALITIES[0])?;
    let (best, mime_type) = if try_png {
        let mut png = Vec::new();
        image.write_with_encoder(PngEncoder::new_with_quality(&mut png, CompressionType::Default, PngFilter::Adaptive)).map_err(|e| format!("it could not be written as a PNG: {e}"))?;
        if png.len() <= first.len() { (png, "image/png") } else { (first, "image/jpeg") }
    } else {
        (first, "image/jpeg")
    };
    if base64_len(best.len()) <= max_base64 {
        return Ok(Some((best, mime_type)));
    }
    for quality in &JPEG_QUALITIES[1..] {
        let smaller = jpeg(*quality)?;
        if base64_len(smaller.len()) <= max_base64 {
            return Ok(Some((smaller, "image/jpeg")));
        }
    }
    Ok(None)
}

/// The image without transparency, over white, as a JPEG has none: the encoder's black would
/// hide dark content.
fn on_white(image: &DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let rgba = image.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let over = |channel: u8| ((u32::from(channel) * u32::from(a) + 255 * (255 - u32::from(a)) + 127) / 255) as u8;
        Rgb([over(r), over(g), over(b)])
    })
}

/// The image's pixels, its format (none when macOS converted it), and how its file says to turn
/// it. An image the decoders here cannot read goes through `sips` on a Mac, which reads HEIC and
/// every other format macOS knows.
fn read(bytes: &[u8]) -> Result<(DynamicImage, Option<ImageFormat>, Orientation), String> {
    match decode(bytes) {
        Ok(decoded) => Ok(decoded),
        Err(error) => {
            #[cfg(target_os = "macos")]
            {
                if let Some(png) = sips_png(bytes) {
                    return decode(&png).map(|(image, _, orientation)| (image, None, orientation));
                }
            }
            Err(match heif_type(bytes) {
                Some(mime) if cfg!(target_os = "macos") => format!("it is an {mime} image macOS could not convert"),
                Some(mime) => format!("it is an {mime} image, which Lorca converts only on a Mac"),
                None => error,
            })
        }
    }
}

fn decode(bytes: &[u8]) -> Result<(DynamicImage, Option<ImageFormat>, Orientation), String> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let Some(format) = reader.format() else { return Err("it is not an image Lorca can read".into()) };
    let unreadable = |error: ImageError| match error {
        ImageError::Unsupported(_) => format!("Lorca cannot read {} images", format.to_mime_type()),
        error => format!("it could not be read: {}", error.to_string().trim()),
    };
    let mut decoder = reader.into_decoder().map_err(unreadable)?;
    if decoder.total_bytes() > MAX_DECODED_BYTES {
        let (width, height) = decoder.dimensions();
        return Err(format!("it is {width}×{height} pixels, too large to read"));
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let image = DynamicImage::from_decoder(decoder).map_err(unreadable)?;
    Ok((image, Some(format), orientation))
}

/// The image as a PNG by `sips`, which every Mac has, for a format only macOS reads.
#[cfg(target_os = "macos")]
fn sips_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("lorca-image-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).ok()?;
    let (input, output) = (dir.join("image"), dir.join("image.png"));
    let converted = std::fs::write(&input, bytes).ok().and_then(|()| {
        let status = std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "png"])
            .arg(&input)
            .arg("--out")
            .arg(&output)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;
        status.success().then(|| std::fs::read(&output).ok()).flatten()
    });
    let _ = std::fs::remove_dir_all(&dir);
    converted
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba, RgbaImage};

    fn encoded(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        image.write_to(&mut Cursor::new(&mut bytes), format).unwrap();
        bytes
    }

    /// A small picture with a little detail, so its encodings differ in size.
    fn picture(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(width, height, |x, y| Rgb([(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x + y) % 256) as u8])))
    }

    fn decoded(inline: &Inline) -> DynamicImage {
        image::load_from_memory(&base64::engine::general_purpose::STANDARD.decode(&inline.data).unwrap()).unwrap()
    }

    #[test]
    fn an_image_that_fits_goes_as_it_is() {
        let png = encoded(&picture(64, 48), ImageFormat::Png);
        let inline = prepare(&png).unwrap();
        assert_eq!((inline.mime_type.as_str(), inline.note.as_deref()), ("image/png", None));
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&inline.data).unwrap(), png, "the same bytes");
        let data = base64::engine::general_purpose::STANDARD.encode(&png);
        assert_eq!(prepare_base64(&data).unwrap().data, data);
    }

    #[test]
    fn other_formats_are_converted() {
        let bmp = encoded(&picture(64, 48), ImageFormat::Bmp);
        assert_eq!(file_type(&bmp), Some("image/bmp"));
        let inline = prepare(&bmp).unwrap();
        assert_eq!(inline.note.as_deref(), Some(format!("[Image converted from image/bmp to {}.]", inline.mime_type).as_str()));
        assert_eq!(decoded(&inline).dimensions(), (64, 48));
        let gif = encoded(&picture(32, 32), ImageFormat::Gif);
        assert!(prepare(&gif).unwrap().note.unwrap().starts_with("[Image converted from image/gif to "), "a GIF, which not every provider takes");
    }

    #[test]
    fn a_large_image_is_scaled_down_with_a_note() {
        let png = encoded(&picture(2600, 100), ImageFormat::Png);
        let inline = prepare(&png).unwrap();
        assert_eq!(decoded(&inline).dimensions(), (2000, 77));
        assert_eq!(inline.note.as_deref(), Some("[Image: original 2600x100, displayed at 2000x77. Multiply coordinates by 1.30 to map to original image.]"));
        let tall = encoded(&picture(10, 9000), ImageFormat::Png);
        assert_eq!(decoded(&prepare(&tall).unwrap()).dimensions(), (2, 2000), "a long page keeps its shape");
    }

    #[test]
    fn an_image_too_large_in_bytes_gets_smaller() {
        // Noise compresses badly: past the PNG and the first JPEG come lower qualities, then
        // smaller sizes.
        let mut seed = 7u32;
        let noise = DynamicImage::ImageRgb8(ImageBuffer::from_fn(300, 300, |_, _| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            Rgb([(seed >> 16) as u8, (seed >> 8) as u8, seed as u8])
        }));
        let png = encoded(&noise, ImageFormat::Png);
        let jpeg = |quality: u8| {
            let mut jpeg = Vec::new();
            noise.write_with_encoder(JpegEncoder::new_with_quality(&mut jpeg, quality)).unwrap();
            base64_len(jpeg.len())
        };
        let between = (jpeg(40) + jpeg(80)) / 2;
        let Made::New { bytes, mime_type, note } = made(&png, MAX_SIDE, between).unwrap() else { panic!("too large to go as it is") };
        assert_eq!((mime_type, note), ("image/jpeg", None), "a lower quality at the same size");
        assert!(base64_len(bytes.len()) <= between);
        let Made::New { bytes, note, .. } = made(&png, MAX_SIDE, jpeg(40) / 2).unwrap() else { panic!("too large to go as it is") };
        assert!(base64_len(bytes.len()) <= jpeg(40) / 2);
        assert!(note.unwrap().starts_with("[Image: original 300x300, displayed at "), "smaller once the qualities ran out");
    }

    #[test]
    fn transparency_turns_white_in_a_jpeg() {
        let clear: RgbaImage = ImageBuffer::from_pixel(4, 4, Rgba([0, 0, 0, 0]));
        assert_eq!(on_white(&DynamicImage::ImageRgba8(clear)).get_pixel(0, 0), &Rgb([255, 255, 255]));
    }

    #[test]
    fn what_cannot_be_read_says_why() {
        assert_eq!(prepare_base64("not base64!"), Err("its data is not base64".into()));
        let heic = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic";
        assert_eq!(file_type(heic), Some("image/heic"));
        // A Mac hands what the decoders here cannot read to `sips`, which reads more.
        if cfg!(target_os = "macos") {
            return;
        }
        assert_eq!(prepare(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), Err("it is not an image Lorca can read".into()));
        let png = encoded(&picture(64, 48), ImageFormat::Png);
        assert!(prepare(&png[..60]).unwrap_err().starts_with("it could not be read: "), "a cut-off PNG");
        assert_eq!(prepare(heic), Err("it is an image/heic image, which Lorca converts only on a Mac".into()));
    }

    #[test]
    fn files_are_told_apart_by_their_bytes() {
        assert_eq!(file_type(b"\xFF\xD8\xFF\xE0\x00\x10JFIF"), Some("image/jpeg"));
        assert_eq!(file_type(b"II*\0\x08\0\0\0"), Some("image/tiff"));
        assert_eq!(file_type(b"\0\0\0\x1cftypavif\0\0\0\0"), Some("image/avif"));
        assert_eq!(file_type(b"BMW and Audi\nVolvo"), None, "a text file that starts with BM");
    }
}
