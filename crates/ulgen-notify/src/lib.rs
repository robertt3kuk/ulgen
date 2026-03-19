use std::sync::{mpsc, Mutex};

use ulgen_domain::NotificationEvent;

#[derive(Default)]
pub struct NotificationBus {
    subscribers: Mutex<Vec<mpsc::Sender<NotificationEvent>>>,
}

impl NotificationBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self) -> mpsc::Receiver<NotificationEvent> {
        let (tx, rx) = mpsc::channel();
        let mut subscribers = self.subscribers.lock().expect("poisoned mutex");
        subscribers.push(tx);
        rx
    }

    pub fn publish(&self, event: NotificationEvent) {
        let mut subscribers = self.subscribers.lock().expect("poisoned mutex");
        subscribers.retain(|tx| tx.send(event.clone()).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulgen_domain::{NotificationEvent, NotificationEventKind};

    #[test]
    fn fanout_to_subscribers() {
        let bus = NotificationBus::new();
        let rx1 = bus.subscribe();
        let rx2 = bus.subscribe();

        let event = NotificationEvent {
            id: 1,
            kind: NotificationEventKind::TaskDone,
            title: "build complete".to_string(),
            message: "all checks passed".to_string(),
            block_id: Some("block-1".to_string()),
        };

        bus.publish(event.clone());

        assert_eq!(rx1.recv().unwrap(), event);
        assert_eq!(rx2.recv().unwrap(), event);
    }
}
