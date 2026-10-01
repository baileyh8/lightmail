//! Raster format validation shared by the native reader adapters.
use gpui_kit::ImageFormat;

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
