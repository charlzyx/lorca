//! Images as a model takes them inline. A provider refuses a whole request over an image it does
//! not take, and since the image stays in the chat, every later turn fails too, so an image goes
//! to a model only past `inline_type`: a tool's, a script's, and an attachment's alike.

/// The largest image a model takes inline: Anthropic refuses one whose base64 passes 5 MB, which
/// 3.75 MB of image makes.
pub const MAX_INLINE_BYTES: usize = 5 * 1024 * 1024 / 4 * 3;

/// The longest side, in pixels, of an image a model takes inline: Anthropic refuses a longer one.
pub const MAX_INLINE_SIDE: u32 = 8000;

/// The type of an image in base64 that a model can take inline, or why it cannot
/// (`inline_type_of`).
pub fn inline_type(data: &str) -> Result<&'static str, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "its data is not base64".to_string())?;
    inline_type_of(&bytes)
}

/// The type of an image a model can take inline, or why it cannot: a PNG, JPEG, GIF, or WebP
/// image, as its first bytes say rather than how it was labeled, of at most 3.75 MB and 8000
/// pixels on a side.
pub fn inline_type_of(bytes: &[u8]) -> Result<&'static str, String> {
    let mime = match bytes {
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => "image/png",
        // JPEG-LS starts as a JPEG does, and no provider takes it.
        [0xFF, 0xD8, 0xFF, marker, ..] if !matches!(marker, 0xF7 | 0xF8) => "image/jpeg",
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => return Err("it is not a PNG, JPEG, GIF, or WebP image".into()),
    };
    check_len(bytes.len() as u64)?;
    if let Some((width, height)) = size(bytes).filter(|(width, height)| (*width).max(*height) > MAX_INLINE_SIDE) {
        return Err(format!("it is {width}×{height} pixels, over {MAX_INLINE_SIDE} on a side"));
    }
    Ok(mime)
}

/// Whether an image of `len` bytes is small enough to go inline, for a caller that can tell
/// before reading it.
pub fn check_len(len: u64) -> Result<(), String> {
    if len > MAX_INLINE_BYTES as u64 {
        return Err(format!("it is over {} MB", MAX_INLINE_BYTES as f64 / (1024.0 * 1024.0)));
    }
    Ok(())
}

/// Width and height from the header of a PNG, JPEG, GIF, or WebP, without decoding the image.
pub fn size(bytes: &[u8]) -> Option<(u32, u32)> {
    let be16 = |at: usize| u16::from_be_bytes([bytes[at], bytes[at + 1]]) as u32;
    let le16 = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as u32;
    let be32 = |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    if bytes.len() >= 24 && bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some((be32(16), be32(20)));
    }
    if bytes.len() >= 10 && bytes.starts_with(b"GIF8") {
        return Some((le16(6), le16(8)));
    }
    if bytes.len() >= 30 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return match &bytes[12..16] {
            b"VP8X" => Some((1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]), 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]))),
            // A lossy frame's size follows its start code, 14 bits each.
            b"VP8 " if bytes[23..26] == [0x9D, 0x01, 0x2A] => Some((le16(26) & 0x3FFF, le16(28) & 0x3FFF)),
            // A lossless one packs both, less one, into the 28 bits after its signature.
            b"VP8L" if bytes[20] == 0x2F => {
                let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
                Some((1 + (bits & 0x3FFF), 1 + ((bits >> 14) & 0x3FFF)))
            }
            _ => None,
        };
    }
    if bytes.len() > 4 && bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
                return Some((be16(i + 7), be16(i + 5)));
            }
            i += 2 + (be16(i + 2) as usize).max(2);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// A PNG header of the given size: all `size` and `inline_type_of` read.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png
    }

    #[test]
    fn sizes_come_from_headers() {
        assert_eq!(size(&png(640, 480)), Some((640, 480)));
        assert_eq!(size(b"GIF89a\x40\x01\xF0\x00"), Some((320, 240)));
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0xE0, 0x02, 0x80, 0x03]);
        assert_eq!(size(&jpeg), Some((640, 480)));
        let webp = |chunk: &[u8], frame: &[u8]| [b"RIFF\0\0\0\0WEBP".as_slice(), chunk, &[0; 4], frame, &[0; 8]].concat();
        assert_eq!(size(&webp(b"VP8X", &[0, 0, 0, 0, 0x7F, 0x02, 0x00, 0xDF, 0x01, 0x00])), Some((640, 480)));
        assert_eq!(size(&webp(b"VP8 ", &[0, 0, 0, 0x9D, 0x01, 0x2A, 0x80, 0x02, 0xE0, 0x01])), Some((640, 480)));
        // 639 and 479 in 14 bits each: 639 | 479 << 14.
        let bits = (639u32 | (479 << 14)).to_le_bytes();
        assert_eq!(size(&webp(b"VP8L", &[0x2F, bits[0], bits[1], bits[2], bits[3]])), Some((640, 480)));
        assert_eq!(size(b"nope"), None);
    }

    #[test]
    fn only_images_a_model_takes_pass() {
        assert_eq!(inline_type_of(&png(640, 480)), Ok("image/png"));
        assert_eq!(inline_type_of(b"\xFF\xD8\xFF\xE0\x00\x10JFIF\x00\x01"), Ok("image/jpeg"));
        assert_eq!(inline_type_of(b"BM\x36\x00\x0C\x00\x00\x00\x00\x00"), Err("it is not a PNG, JPEG, GIF, or WebP image".into()), "a BMP");
        assert_eq!(inline_type_of(b"\xFF\xD8\xFF\xF7\x00\x0B"), Err("it is not a PNG, JPEG, GIF, or WebP image".into()), "JPEG-LS");
        assert_eq!(inline_type_of(&png(1280, 8001)), Err("it is 1280×8001 pixels, over 8000 on a side".into()), "a long full-page screenshot");
        let mut large = png(640, 480);
        large.resize(MAX_INLINE_BYTES + 1, 0);
        assert_eq!(inline_type_of(&large), Err("it is over 3.75 MB".into()), "its base64 would pass Anthropic's 5 MB");
        large.truncate(MAX_INLINE_BYTES);
        assert_eq!(base64::engine::general_purpose::STANDARD.encode(&large).len(), 5 * 1024 * 1024, "the largest one fills 5 MB of base64 exactly");
        assert_eq!(inline_type(&base64::engine::general_purpose::STANDARD.encode(&large)), Ok("image/png"));
        assert_eq!(inline_type("not base64!"), Err("its data is not base64".into()));
    }
}
