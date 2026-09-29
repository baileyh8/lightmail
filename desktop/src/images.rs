//! Remote images for the native reader. Nothing is requested until the user
//! allows images for the current message; switching messages forgets them.
//! Fetches go through the core, so they use the system proxy and same-origin
//! redirects only, and each one is bounded in size and time. Only raster
//! formats are decoded: SVG could reference further resources.
use crate::reader::{Images, PLACEHOLDER};
use base64::Engine as _;
use gpui_kit::{Image, ImageFormat, ImageSource, SharedUri};
use lightmail_core::PlatformServices;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{mpsc, Arc, Mutex},
};

const IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MESSAGE_BYTES: usize = 32 * 1024 * 1024;
const MESSAGE_IMAGES: usize = 64;

enum Slot {
    Pending,
    Ready(Arc<Image>),
    Failed,
}

#[derive(Default)]
struct State {
    generation: u64,
    slots: HashMap<String, Slot>,
    requested: usize,
    bytes: usize,
}

pub struct RemoteImages {
    state: Mutex<State>,
    queue: mpsc::SyncSender<(u64, String)>,
}

fn raster(mime: &str, bytes: &[u8]) -> Option<ImageFormat> {
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
    // The bytes decide; a declared type only has to agree when it names an image.
    let declared = ImageFormat::from_mime_type(mime.split(';').next().unwrap_or("").trim());
    match (sniffed, declared) {
        (Some(format), None) => Some(format),
        (Some(format), Some(declared)) if format == declared => Some(format),
        _ => None,
    }
}

fn data_url(text: &str) -> Option<(ImageFormat, Vec<u8>)> {
    let (header, payload) = text.strip_prefix("data:")?.split_once(',')?;
    let (mime, encoding) = header.split_once(';')?;
    if encoding != "base64" || payload.len() > IMAGE_BYTES / 3 * 4 + 4 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .ok()?;
    Some((raster(mime, &bytes)?, bytes))
}

impl RemoteImages {
    /// `ready` runs on the worker thread after each image settles.
    pub fn new(
        platform: Arc<dyn PlatformServices>,
        ready: impl Fn() + Send + 'static,
    ) -> Arc<Self> {
        let (queue, jobs) = mpsc::sync_channel::<(u64, String)>(MESSAGE_IMAGES);
        let images = Arc::new(Self {
            state: Mutex::new(State::default()),
            queue,
        });
        let weak = Arc::downgrade(&images);
        std::thread::Builder::new()
            .name("reader-images".into())
            .spawn(move || {
                for (generation, url) in jobs {
                    let fetched =
                        lightmail_core::fetch_resource(platform.clone(), url.clone(), IMAGE_BYTES)
                            .ok()
                            .and_then(|(bytes, mime)| Some((raster(&mime, &bytes)?, bytes)));
                    let Some(images) = weak.upgrade() else {
                        break;
                    };
                    let mut state = images.state.lock().unwrap();
                    if state.generation != generation {
                        continue;
                    }
                    let slot = match fetched {
                        Some((format, bytes)) if state.bytes + bytes.len() <= MESSAGE_BYTES => {
                            state.bytes += bytes.len();
                            Slot::Ready(Arc::new(Image::from_bytes(format, bytes)))
                        }
                        _ => Slot::Failed,
                    };
                    state.slots.insert(url, slot);
                    drop(state);
                    ready();
                }
            })
            .expect("start the reader image worker");
        images
    }

    /// Forgets every image of the previous message, including pending fetches.
    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        *state = State {
            generation: state.generation + 1,
            ..State::default()
        };
    }

    pub fn resolver(self: &Arc<Self>) -> Images {
        let images = self.clone();
        Arc::new(move |uri: &SharedUri| images.resolve(uri))
    }

    fn resolve(&self, uri: &SharedUri) -> ImageSource {
        let text: &str = uri.as_ref();
        let placeholder = || ImageSource::from(PLACEHOLDER);
        let mut state = self.state.lock().unwrap();
        if text.starts_with("data:") {
            // Embedded images need no network; decode once per message.
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut hasher);
            let key = format!("data#{:x}", hasher.finish());
            if !state.slots.contains_key(&key) {
                let slot = match data_url(text) {
                    Some((format, bytes)) if state.bytes + bytes.len() <= MESSAGE_BYTES => {
                        state.bytes += bytes.len();
                        Slot::Ready(Arc::new(Image::from_bytes(format, bytes)))
                    }
                    _ => Slot::Failed,
                };
                state.slots.insert(key.clone(), slot);
            }
            return match state.slots.get(&key) {
                Some(Slot::Ready(image)) => ImageSource::Image(image.clone()),
                _ => placeholder(),
            };
        }
        if !url::Url::parse(text).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
            return placeholder();
        }
        match state.slots.get(text) {
            Some(Slot::Ready(image)) => return ImageSource::Image(image.clone()),
            Some(Slot::Pending | Slot::Failed) => return placeholder(),
            None => {}
        }
        let generation = state.generation;
        let queued = state.requested < MESSAGE_IMAGES
            && self.queue.try_send((generation, text.to_string())).is_ok();
        state.requested += 1;
        state.slots.insert(
            text.to_string(),
            if queued { Slot::Pending } else { Slot::Failed },
        );
        placeholder()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_matching_raster_formats_are_decoded() {
        let png = b"\x89PNG\r\n\x1a\nrest";
        assert!(matches!(raster("image/png", png), Some(ImageFormat::Png)));
        assert!(matches!(raster("", png), Some(ImageFormat::Png)));
        assert!(
            raster("image/jpeg", png).is_none(),
            "declared and sniffed must agree"
        );
        assert!(raster(
            "image/svg+xml",
            b"<svg xmlns='http://www.w3.org/2000/svg'/>"
        )
        .is_none());
        assert!(raster("text/html", b"<html>").is_none());
        let encoded = base64::engine::general_purpose::STANDARD.encode(png);
        assert!(data_url(&format!("data:image/png;base64,{encoded}")).is_some());
        assert!(data_url("data:image/svg+xml;base64,PHN2Zy8+").is_none());
        assert!(data_url("data:image/png,raw").is_none());
    }
}
