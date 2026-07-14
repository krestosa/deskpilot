use serde::Serialize;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;

use crate::logging::timestamp_utc;

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub timestamp: String,
    pub kind: String,
    pub message: String,
}

impl Event {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            timestamp: timestamp_utc(),
            kind: kind.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Default)]
pub struct EventBus {
    subscribers: Mutex<Vec<Sender<Event>>>,
}

impl EventBus {
    pub fn subscribe(&self) -> Receiver<Event> {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(sender);
        }
        receiver
    }

    pub fn publish(&self, event: Event) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| subscriber.send(event.clone()).is_ok());
        }
    }
}
