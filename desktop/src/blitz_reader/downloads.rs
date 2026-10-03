use super::{
    image_pixels, NetState, MAX_IMAGE_BYTES, MAX_MESSAGE_BYTES, MAX_MESSAGE_IMAGES,
    MAX_SURFACE_PIXELS,
};
use blitz_traits::net::{Bytes, NetHandler};
use lightmail_core::PlatformServices;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
};

struct Job {
    url: String,
    handler: Box<dyn NetHandler>,
    images: Images,
    state: Arc<Mutex<NetState>>,
    platform: Arc<dyn PlatformServices>,
}
#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    closed: bool,
}
pub(super) struct Downloader {
    queue: Arc<(Mutex<Queue>, Condvar)>,
}
#[derive(Clone)]
pub(super) struct Images {
    downloader: Arc<Downloader>,
    pub cancelled: Arc<AtomicBool>,
    pub generation: Arc<std::sync::atomic::AtomicU64>,
    ready: Arc<dyn Fn(u64) + Send + Sync>,
}
impl Downloader {
    pub fn new() -> Arc<Self> {
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let work = queue.clone();
        std::thread::Builder::new()
            .name("html-images".into())
            .spawn(move || loop {
                let job = {
                    let mut state = work.0.lock().unwrap();
                    while !state.closed && state.jobs.is_empty() {
                        state = work.1.wait(state).unwrap();
                    }
                    if state.closed {
                        break;
                    }
                    state.jobs.pop_front().unwrap()
                };
                if job.images.cancelled.load(Ordering::Acquire) {
                    continue;
                }
                let limit = {
                    let state = job.state.lock().unwrap();
                    MAX_IMAGE_BYTES.min(MAX_MESSAGE_BYTES - state.bytes)
                };
                let fetched = if limit > 0 {
                    lightmail_core::fetch_resource(job.platform, job.url.clone(), limit)
                        .ok()
                        .and_then(|(bytes, mime)| {
                            image_pixels(&mime, &bytes).map(|pixels| (bytes, pixels))
                        })
                } else {
                    None
                };
                if job.images.cancelled.load(Ordering::Acquire) {
                    continue;
                }
                let bytes = {
                    let mut state = job.state.lock().unwrap();
                    state.finish(job.url.clone(), fetched)
                };
                job.handler.bytes(job.url, bytes);
                (job.images.ready)(job.images.generation.load(Ordering::Acquire));
            })
            .expect("start HTML image worker");
        Arc::new(Self { queue })
    }
    pub fn images(
        self: &Arc<Self>,
        generation: u64,
        ready: Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Images {
        Images {
            downloader: self.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(std::sync::atomic::AtomicU64::new(generation)),
            ready,
        }
    }
}
impl Images {
    pub fn fetch(
        &self,
        url: String,
        handler: Box<dyn NetHandler>,
        state: Arc<Mutex<NetState>>,
        platform: Arc<dyn PlatformServices>,
    ) {
        let mut budget = state.lock().unwrap();
        if self.cancelled.load(Ordering::Acquire) {
            drop(budget);
            handler.bytes(url, Bytes::new());
            return;
        }
        if let Some(bytes) = budget.images.get(&url) {
            let bytes = bytes.clone();
            drop(budget);
            handler.bytes(url, bytes);
            return;
        }
        if budget.images.len() >= MAX_MESSAGE_IMAGES
            || budget.bytes >= MAX_MESSAGE_BYTES
            || budget.pixels >= MAX_SURFACE_PIXELS
        {
            drop(budget);
            handler.bytes(url, Bytes::new());
            return;
        }
        let mut pending = self.downloader.queue.0.lock().unwrap();
        pending
            .jobs
            .retain(|job| !job.images.cancelled.load(Ordering::Acquire));
        if pending.closed || pending.jobs.len() >= MAX_MESSAGE_IMAGES {
            drop(pending);
            drop(budget);
            handler.bytes(url, Bytes::new());
            return;
        }
        budget.images.insert(url.clone(), Bytes::new()); // reserves a slot before IO
        drop(budget);
        pending.jobs.push_back(Job {
            url,
            handler,
            images: self.clone(),
            state,
            platform,
        });
        self.downloader.queue.1.notify_one();
    }
}
impl Drop for Downloader {
    fn drop(&mut self) {
        let mut state = self.queue.0.lock().unwrap();
        state.closed = true;
        state.jobs.clear();
        self.queue.1.notify_one();
    }
}
