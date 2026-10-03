use async_channel::{Receiver, Sender};
use lightmail_core::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

pub enum Event {
    Core(ApplicationEvent),
    Translation(u64, u32, u32),
    /// A reader image settled; only a redraw is needed.
    Image,
}
pub struct Events {
    queue: Mutex<VecDeque<(String, Event)>>,
    wake: Sender<()>,
}
impl Events {
    pub fn new() -> (Arc<Self>, Receiver<()>) {
        let (wake, receiver) = async_channel::bounded(1);
        (
            Arc::new(Self {
                queue: Mutex::new(VecDeque::new()),
                wake,
            }),
            receiver,
        )
    }
    fn push(&self, key: String, event: Event) {
        let mut queue = self.queue.lock().unwrap();
        if let Some(slot) = queue.iter_mut().find(|(k, _)| k == &key) {
            slot.1 = event;
        } else {
            queue.push_back((key, event));
        }
        drop(queue);
        let _ = self.wake.try_send(());
    }
    pub fn image_ready(&self) {
        self.push("image".into(), Event::Image);
    }
    pub fn drain(&self) -> Vec<Event> {
        self.queue
            .lock()
            .unwrap()
            .drain(..)
            .map(|(_, e)| e)
            .collect()
    }
}
impl ApplicationObserver for Events {
    fn changed(&self, event: ApplicationEvent) {
        let kind = match event.kind {
            ApplicationEventKind::DataChanged => "data",
            ApplicationEventKind::SyncStarted | ApplicationEventKind::SyncFinished => "sync",
            ApplicationEventKind::DraftChanged => "draft",
            ApplicationEventKind::Problem => "problem",
        };
        self.push(format!("{kind}:{}", event.account_id), Event::Core(event));
    }
}
pub struct TranslationProgress {
    pub events: Arc<Events>,
    pub generation: u64,
}
impl TranslationObserver for TranslationProgress {
    fn progress(&self, done: u32, total: u32, _blocks: Vec<TranslationBlock>) {
        self.events.push(
            "translation".into(),
            Event::Translation(self.generation, done, total),
        );
    }
}
