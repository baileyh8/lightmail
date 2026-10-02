//! Remote images for the native reader. Nothing is requested until the user
//! allows images for the current message; switching messages forgets them.
//! Fetches go through the core, so they use the system proxy and same-origin
//! redirects only, and each one is bounded in size and time. Only raster
//! formats are decoded: SVG could reference further resources.
pub(crate) use crate::image_types::raster;
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
    pixels: u64,
}

pub struct RemoteImages {
    state: Mutex<State>,
    platform: Mutex<Option<Arc<dyn PlatformServices>>>,
    queue: mpsc::SyncSender<(u64, String, Arc<dyn PlatformServices>)>,
}

fn data_url(text: &str) -> Option<(ImageFormat, Vec<u8>, u64)> {
    let (header, payload) = text.strip_prefix("data:")?.split_once(',')?;
    let (mime, encoding) = header.split_once(';')?;
    if encoding != "base64" || payload.len() > IMAGE_BYTES / 3 * 4 + 4 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .ok()?;
    let pixels = crate::image_types::pixels(mime, &bytes)?;
    Some((raster(mime, &bytes)?, bytes, pixels))
}

impl RemoteImages {
    /// `ready` runs on the worker thread after each image settles.
    pub fn new(
        platform: Arc<dyn PlatformServices>,
        ready: impl Fn() + Send + 'static,
    ) -> Arc<Self> {
        let (queue, jobs) =
            mpsc::sync_channel::<(u64, String, Arc<dyn PlatformServices>)>(MESSAGE_IMAGES);
        let images = Arc::new(Self {
            state: Mutex::new(State::default()),
            platform: Mutex::new(Some(platform)),
            queue,
        });
        let weak = Arc::downgrade(&images);
        std::thread::Builder::new()
            .name("reader-images".into())
            .spawn(move || {
                for (generation, url, platform) in jobs {
                    let Some(current) = weak.upgrade() else {
                        break;
                    };
                    if current.state.lock().unwrap().generation != generation {
                        continue;
                    }
                    drop(current);
                    let fetched =
                        lightmail_core::fetch_resource(platform.clone(), url.clone(), IMAGE_BYTES)
                            .ok()
                            .and_then(|(bytes, mime)| {
                                Some((
                                    raster(&mime, &bytes)?,
                                    crate::image_types::pixels(&mime, &bytes)?,
                                    bytes,
                                ))
                            });
                    let Some(images) = weak.upgrade() else {
                        break;
                    };
                    let mut state = images.state.lock().unwrap();
                    if state.generation != generation {
                        continue;
                    }
                    let slot = match fetched {
                        Some((format, pixels, bytes))
                            if state.bytes + bytes.len() <= MESSAGE_BYTES
                                && state.pixels + pixels <= crate::image_types::MAX_PIXELS =>
                        {
                            state.bytes += bytes.len();
                            state.pixels += pixels;
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

    /// Bind the current message to its account's route. Invalid policies deny downloads.
    pub fn reset_for_account(&self, platform: Option<Arc<dyn PlatformServices>>) {
        let mut state = self.state.lock().unwrap();
        *self.platform.lock().unwrap() = platform;
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
                if state.slots.len() >= MESSAGE_IMAGES {
                    return placeholder();
                }
                let slot = match data_url(text) {
                    Some((format, bytes, pixels))
                        if state.bytes + bytes.len() <= MESSAGE_BYTES
                            && state.pixels + pixels <= crate::image_types::MAX_PIXELS =>
                    {
                        state.bytes += bytes.len();
                        state.pixels += pixels;
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
        if state.slots.len() >= MESSAGE_IMAGES {
            return placeholder();
        }
        let queued = state.requested < MESSAGE_IMAGES
            && self
                .platform
                .lock()
                .unwrap()
                .clone()
                .is_some_and(|platform| {
                    self.queue
                        .try_send((generation, text.to_string(), platform))
                        .is_ok()
                });
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
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(crate::image_types::tests::png(1, 1, 0));
        assert!(data_url(&format!("data:image/png;base64,{encoded}")).is_some());
        assert!(data_url("data:image/svg+xml;base64,PHN2Zy8+").is_none());
        assert!(data_url("data:image/png,raw").is_none());
    }

    #[test]
    fn markdown_images_reject_high_compression_dimensions_and_charge_total_pixels() {
        let platform = Arc::new(crate::platform::DesktopPlatform::preview());
        let images = RemoteImages::new(platform, || {});
        let uri = |bytes: Vec<u8>| {
            SharedUri::from(format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ))
        };
        let large = crate::image_types::tests::png(6000, 4000, 0);
        assert!(large.len() < 200_000);
        assert!(!matches!(
            images.resolve(&uri(large)),
            ImageSource::Image(_)
        ));
        for i in 0..40 {
            assert!(matches!(
                images.resolve(&uri(crate::image_types::tests::png(640, 640, i))),
                ImageSource::Image(_)
            ));
        }
        assert!(!matches!(
            images.resolve(&uri(crate::image_types::tests::png(640, 640, 42))),
            ImageSource::Image(_)
        ));
        assert!(images.state.lock().unwrap().pixels <= crate::image_types::MAX_PIXELS);
    }
}
