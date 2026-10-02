use super::{downloads, Point, ReaderViewport, Rendered, Select, Selected, Session};
use lightmail_core::PlatformServices;
use std::sync::{atomic::Ordering, Arc, Condvar, Mutex};

struct Load {
    generation: u64,
    key: String,
    html: String,
    viewport: ReaderViewport,
    images: bool,
    platform: Arc<dyn PlatformServices>,
    reply: async_channel::Sender<anyhow::Result<Rendered>>,
    drain: async_channel::Receiver<anyhow::Result<Rendered>>,
}
struct Selection {
    generation: u64,
    selection: Select,
    reply: async_channel::Sender<Selected>,
}
#[derive(Default)]
struct Pending {
    generation: u64,
    load: Option<Load>,
    selection: Option<Selection>,
    clear: bool,
    repaint: bool,
    closed: bool,
    scroll: Option<(u64, Point)>,
}
struct Active {
    generation: u64,
    key: String,
    images: bool,
    session: Session,
    resources: downloads::Images,
    reply: async_channel::Sender<anyhow::Result<Rendered>>,
    drain: async_channel::Receiver<anyhow::Result<Rendered>>,
}
fn publish(
    reply: &async_channel::Sender<anyhow::Result<Rendered>>,
    drain: &async_channel::Receiver<anyhow::Result<Rendered>>,
    result: anyhow::Result<Rendered>,
) {
    if let Err(async_channel::TrySendError::Full(result)) = reply.try_send(result) {
        let _ = drain.try_recv();
        let _ = reply.try_send(result);
    }
}
/// One DOM thread, one limited image downloader, and one pending layout,
/// selection and repaint. Owned geometry/pixels cross threads; DOMs do not.
pub struct Worker {
    pending: Arc<(Mutex<Pending>, Condvar)>,
}
impl Worker {
    pub fn new() -> Self {
        let pending = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let work = pending.clone();
        std::thread::Builder::new()
            .name("html-reader".into())
            .spawn(move || {
                let downloader = downloads::Downloader::new();
                let weak = Arc::downgrade(&work);
                let ready: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |generation| {
                    if let Some(work) = weak.upgrade() {
                        let mut state = work.0.lock().unwrap();
                        if !state.closed && state.generation == generation {
                            state.repaint = true;
                            work.1.notify_one();
                        }
                    }
                });
                let mut active: Option<Active> = None;
                loop {
                    let (load, selection, clear, repaint, scroll) = {
                        let mut state = work.0.lock().unwrap();
                        while !state.closed
                            && state.load.is_none()
                            && state.selection.is_none()
                            && !state.clear
                            && !state.repaint
                            && state.scroll.is_none()
                        {
                            state = work.1.wait(state).unwrap();
                        }
                        if state.closed {
                            break;
                        }
                        (
                            state.load.take(),
                            state.selection.take(),
                            std::mem::take(&mut state.clear),
                            std::mem::take(&mut state.repaint),
                            state.scroll.take(),
                        )
                    };
                    if clear {
                        drop(active.take());
                    }
                    let had_load = load.is_some();
                    if let Some(load) = load {
                        let reply = load.reply.clone();
                        let drain = load.drain.clone();
                        let generation = load.generation;
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            let reuse = active.as_ref().is_some_and(|a| {
                                a.key == load.key
                                    && a.images == load.images
                                    && a.session.source.as_ref() == load.html
                            });
                            if !reuse {
                                drop(active.take());
                                let resources = downloader.images(generation, ready.clone());
                                active = Some(Active {
                                    generation,
                                    key: load.key,
                                    images: load.images,
                                    session: Session::with_images(
                                        load.html,
                                        load.viewport,
                                        load.images,
                                        load.platform,
                                        Some(resources.clone()),
                                    ),
                                    resources,
                                    reply: load.reply,
                                    drain: load.drain,
                                });
                            }
                            let a = active.as_mut().unwrap();
                            a.generation = generation;
                            a.resources.generation.store(generation, Ordering::Release);
                            a.reply = reply.clone();
                            a.drain = drain.clone();
                            if a.session.viewport != load.viewport {
                                a.session.resize(load.viewport);
                            }
                            a.session.paint()
                        }))
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("HTML 排版失败")));
                        let state = work.0.lock().unwrap();
                        if state.closed || state.generation != generation {
                            drop(state);
                            drop(active.take());
                            continue;
                        }
                        drop(state);
                        if result.is_err() {
                            drop(active.take());
                        }
                        publish(&reply, &drain, result);
                    }
                    if let Some(selection) = selection {
                        if let Some(a) = active.as_mut() {
                            if a.generation == selection.generation
                                && work.0.lock().unwrap().generation == a.generation
                            {
                                let selected =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        a.session.select(selection.selection)
                                    }))
                                    .unwrap_or_default();
                                let _ = selection.reply.try_send(selected);
                            }
                        }
                    }
                    if (repaint && !had_load) || scroll.is_some() {
                        if let Some(a) = active.as_mut() {
                            if work.0.lock().unwrap().generation == a.generation {
                                if let Some((generation, point)) = scroll {
                                    if generation == a.generation {
                                        a.session.scroll = point;
                                    }
                                }
                                let result =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        if repaint && !had_load {
                                            a.session.paint()
                                        } else {
                                            a.session.paint_region(false)
                                        }
                                    }))
                                    .unwrap_or_else(|_| Err(anyhow::anyhow!("图片重排失败")));
                                let failed = result.is_err();
                                publish(&a.reply, &a.drain, result);
                                if failed {
                                    drop(active.take());
                                }
                            }
                        }
                    }
                }
            })
            .expect("start HTML reader worker");
        Self { pending }
    }
    pub fn load(
        &self,
        generation: u64,
        key: String,
        html: String,
        viewport: ReaderViewport,
        images: bool,
        platform: Arc<dyn PlatformServices>,
    ) -> async_channel::Receiver<anyhow::Result<Rendered>> {
        let (reply, receiver) = async_channel::bounded(1);
        let mut state = self.pending.0.lock().unwrap();
        state.generation = generation;
        state.load = Some(Load {
            generation,
            key,
            html,
            viewport,
            images,
            platform,
            reply,
            drain: receiver.clone(),
        });
        state.selection = None;
        state.clear = false;
        state.repaint = false;
        state.scroll = None;
        self.pending.1.notify_one();
        receiver
    }
    pub fn select(&self, generation: u64, selection: Select) -> async_channel::Receiver<Selected> {
        let (reply, receiver) = async_channel::bounded(1);
        let mut state = self.pending.0.lock().unwrap();
        if state.generation == generation {
            state.selection = Some(Selection {
                generation,
                selection,
                reply,
            });
            self.pending.1.notify_one();
        }
        receiver
    }
    pub fn scroll(&self, generation: u64, point: Point) {
        let mut state = self.pending.0.lock().unwrap();
        if state.generation == generation {
            state.scroll = Some((generation, point));
            self.pending.1.notify_one();
        }
    }
    pub fn clear(&self, generation: u64) {
        let mut state = self.pending.0.lock().unwrap();
        state.generation = generation;
        state.load = None;
        state.selection = None;
        state.clear = true;
        state.scroll = None;
        self.pending.1.notify_one();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let mut state = self.pending.0.lock().unwrap();
        state.closed = true;
        state.load = None;
        state.selection = None;
        self.pending.1.notify_one();
    }
}
