//! Raster format validation shared by the native reader adapters.
use gpui_kit::ImageFormat;

pub(crate) const MAX_PIXELS: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Inspect headers and animation containers before any pixel allocation. Both
/// native readers charge full canvas pixels for every retained animation frame.
pub(crate) fn pixels(mime: &str, bytes: &[u8]) -> Option<u64> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let format = raster(mime, bytes)?;
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    let canvas = u64::from(w).checked_mul(u64::from(h))?;
    if canvas == 0 || canvas > MAX_PIXELS {
        return None;
    }
    let frames = match format {
        ImageFormat::Gif => gif_frames(bytes)?,
        ImageFormat::Png => png_frames(bytes)?,
        ImageFormat::Webp => webp_frames(bytes)?,
        _ => 1,
    };
    let pixels = canvas.checked_mul(frames)?;
    (pixels <= MAX_PIXELS).then_some(pixels)
}

fn sub_blocks(bytes: &[u8], mut offset: usize) -> Option<usize> {
    loop {
        let size = *bytes.get(offset)? as usize;
        offset += 1;
        if size == 0 {
            return Some(offset);
        }
        offset = offset.checked_add(size)?;
        if offset > bytes.len() {
            return None;
        }
    }
}
fn gif_frames(bytes: &[u8]) -> Option<u64> {
    let packed = *bytes.get(10)?;
    let mut offset = 13
        + if packed & 0x80 != 0 {
            3 * (1usize << ((packed & 7) + 1))
        } else {
            0
        };
    let mut frames = 0;
    loop {
        match *bytes.get(offset)? {
            0x3b => return (frames > 0).then_some(frames),
            0x21 => {
                offset = sub_blocks(bytes, offset.checked_add(2)?)?;
            }
            0x2c => {
                let packed = *bytes.get(offset + 9)?;
                offset += 10;
                if packed & 0x80 != 0 {
                    offset += 3 * (1usize << ((packed & 7) + 1));
                }
                offset = sub_blocks(bytes, offset.checked_add(1)?)?;
                frames += 1;
            }
            _ => return None,
        }
    }
}
fn png_frames(bytes: &[u8]) -> Option<u64> {
    let mut offset = 8usize;
    let mut declared = 1u64;
    let mut frames = 0u64;
    while offset < bytes.len() {
        let size = u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize;
        let kind = bytes.get(offset + 4..offset + 8)?;
        let end = offset.checked_add(12)?.checked_add(size)?;
        if end > bytes.len() {
            return None;
        }
        if kind == b"acTL" {
            if size != 8 {
                return None;
            }
            declared =
                u32::from_be_bytes(bytes.get(offset + 8..offset + 12)?.try_into().ok()?) as u64;
            if declared == 0 {
                return None;
            }
        }
        if kind == b"fcTL" {
            frames += 1;
        }
        if kind == b"IEND" {
            return Some(declared.max(frames));
        }
        offset = end;
    }
    None
}
fn webp_frames(bytes: &[u8]) -> Option<u64> {
    let mut offset = 12usize;
    let mut frames = 0u64;
    while offset < bytes.len() {
        let kind = bytes.get(offset..offset + 4)?;
        let size = u32::from_le_bytes(bytes.get(offset + 4..offset + 8)?.try_into().ok()?) as usize;
        offset = offset
            .checked_add(8)?
            .checked_add(size)?
            .checked_add(size % 2)?;
        if offset > bytes.len() {
            return None;
        }
        if kind == b"ANMF" {
            frames += 1;
        }
    }
    Some(frames.max(1))
}

pub(crate) fn raster(mime: &str, bytes: &[u8]) -> Option<ImageFormat> {
    let sniffed = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else if bytes.starts_with(b"BM") {
        Some(ImageFormat::Bmp)
    } else {
        None
    };
    let declared = ImageFormat::from_mime_type(mime.split(';').next().unwrap_or("").trim());
    match (sniffed, declared) {
        (Some(format), None) => Some(format),
        (Some(format), Some(declared)) if format == declared => Some(format),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;
    pub fn png(w: u32, h: u32, color: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut png = encoder.write_header().unwrap();
            let mut writer = png.stream_writer().unwrap();
            let row: Vec<u8> = [color, 128, 255, 255]
                .into_iter()
                .cycle()
                .take(w as usize * 4)
                .collect();
            for _ in 0..h {
                writer.write_all(&row).unwrap();
            }
            writer.finish().unwrap();
        }
        bytes
    }
    fn gif(w: u16, h: u16, frames: usize) -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend(w.to_le_bytes());
        bytes.extend(h.to_le_bytes());
        bytes.extend([0x80, 0, 0, 0, 0, 0, 255, 255, 255]);
        for _ in 0..frames {
            bytes.extend([0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 0x01, 0]);
        }
        bytes.push(0x3b);
        bytes
    }
    #[test]
    fn animation_containers_charge_all_canvas_frames_before_decode() {
        assert_eq!(pixels("image/gif", &gif(100, 100, 2)), Some(20_000));
        assert!(pixels("image/gif", &gif(3000, 3000, 2)).is_none());
        let mut truncated = gif(100, 100, 2);
        truncated.pop();
        assert!(pixels("image/gif", &truncated).is_none());
        let static_png = png(1, 1, 0);
        assert_eq!(pixels("image/png", &static_png), Some(1));
        let mut apng = static_png[..33].to_vec();
        apng.extend(8u32.to_be_bytes());
        apng.extend(b"acTL");
        apng.extend(25u32.to_be_bytes());
        apng.extend(0u32.to_be_bytes());
        apng.extend([0; 4]);
        apng.extend(&static_png[33..]);
        assert_eq!(png_frames(&apng), Some(25));
        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        for _ in 0..2 {
            webp.extend(b"ANMF\0\0\0\0");
        }
        assert_eq!(webp_frames(&webp), Some(2));
        webp.pop();
        assert!(webp_frames(&webp).is_none());
    }
}
