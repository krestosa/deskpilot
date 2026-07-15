// File purpose: Implements the bounded in-process event bus used by diagnostics and event streaming.
use serde::Serialize;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Mutex;

use crate::logging::timestamp_utc;

const SUBSCRIBER_CAPACITY: usize = 128;
const MAX_SUBSCRIBERS: usize = 8;
const MAX_CONSECUTIVE_DROPS: u8 = 16;

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

#[derive(Debug)]
struct Subscriber {
    sender: SyncSender<Event>,
    consecutive_drops: u8,
}

#[derive(Debug, Default)]
pub struct EventBus {
    subscribers: Mutex<Vec<Subscriber>>,
}

impl EventBus {
    // Function purpose: Creates one bounded subscriber and rejects excessive concurrent streams.
    pub fn subscribe(&self) -> Result<Receiver<Event>, String> {
        let (sender, receiver) = mpsc::sync_channel(SUBSCRIBER_CAPACITY);
        let mut subscribers = self
            .subscribers
            .lock()
            .map_err(|_| "event subscriber lock poisoned".to_string())?;
        if subscribers.len() >= MAX_SUBSCRIBERS {
            return Err(format!(
                "event subscriber limit reached ({MAX_SUBSCRIBERS})"
            ));
        }
        subscribers.push(Subscriber {
            sender,
            consecutive_drops: 0,
        });
        Ok(receiver)
    }

    // Function purpose: Publishes without blocking the runtime and disconnects persistently slow consumers.
    pub fn publish(&self, event: Event) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain_mut(
                |subscriber| match subscriber.sender.try_send(event.clone()) {
                    Ok(()) => {
                        subscriber.consecutive_drops = 0;
                        true
                    }
                    Err(TrySendError::Full(_)) => {
                        subscriber.consecutive_drops =
                            subscriber.consecutive_drops.saturating_add(1);
                        subscriber.consecutive_drops < MAX_CONSECUTIVE_DROPS
                    }
                    Err(TrySendError::Disconnected(_)) => false,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, EventBus, MAX_SUBSCRIBERS};

    #[test]
    fn rejects_excessive_subscribers() {
        let bus = EventBus::default();
        let mut receivers = Vec::new();
        for _ in 0..MAX_SUBSCRIBERS {
            receivers.push(bus.subscribe().expect("subscriber should fit"));
        }
        assert!(bus.subscribe().is_err());
        drop(receivers);
    }

    #[test]
    fn publishes_to_connected_subscriber() {
        let bus = EventBus::default();
        let receiver = bus.subscribe().expect("subscriber should be accepted");
        bus.publish(Event::new("test", "message"));
        let event = receiver.recv().expect("event should be delivered");
        assert_eq!(event.kind, "test");
        assert_eq!(event.message, "message");
    }
}
