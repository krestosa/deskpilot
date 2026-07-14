// File purpose: Implements the bounded in-process event bus used by diagnostics and event streaming.
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
    // Function purpose: Constructs a new initialized value for this type.
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
    // Function purpose: Performs the subscribe operation required by this module.
    pub fn subscribe(&self) -> Receiver<Event> {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(sender);
        }
        receiver
    }

    // Function purpose: Performs the publish operation required by this module.
    pub fn publish(&self, event: Event) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| subscriber.send(event.clone()).is_ok());
        }
    }
}
