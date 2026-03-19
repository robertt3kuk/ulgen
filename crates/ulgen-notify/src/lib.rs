use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};

use ulgen_domain::{NotificationEvent, NotificationEventKind};
use ulgen_settings::NotificationsPolicy;

pub trait OsNotificationBridge: Send + Sync {
    fn send(&self, event: &NotificationEvent) -> Result<(), String>;
}

#[derive(Default)]
pub struct NoopOsNotificationBridge;

impl OsNotificationBridge for NoopOsNotificationBridge {
    fn send(&self, _event: &NotificationEvent) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishResult {
    pub in_app_deliveries: usize,
    pub os_attempted: bool,
    pub os_delivered: bool,
    pub os_error: Option<String>,
}

#[derive(Clone)]
struct Subscriber {
    kind_filter: Option<Vec<NotificationEventKind>>,
    tx: mpsc::Sender<NotificationEvent>,
}

impl Subscriber {
    fn matches(&self, event: &NotificationEvent) -> bool {
        match &self.kind_filter {
            Some(kinds) => kinds.contains(&event.kind),
            None => true,
        }
    }
}

pub struct NotificationBus {
    policy: NotificationsPolicy,
    subscribers: Mutex<Vec<Subscriber>>,
    history: Mutex<VecDeque<NotificationEvent>>,
    max_history: usize,
    os_bridge: Arc<dyn OsNotificationBridge>,
    // Process-local monotonic event ids; these reset on app restart.
    next_id: AtomicU64,
}

impl NotificationBus {
    const DEFAULT_MAX_HISTORY: usize = 1_000;

    pub fn new(policy: NotificationsPolicy) -> Self {
        Self::with_os_bridge(policy, Arc::new(NoopOsNotificationBridge))
    }

    pub fn with_os_bridge(
        policy: NotificationsPolicy,
        os_bridge: Arc<dyn OsNotificationBridge>,
    ) -> Self {
        Self::with_os_bridge_and_max_history(policy, os_bridge, Self::DEFAULT_MAX_HISTORY)
    }

    pub fn with_os_bridge_and_max_history(
        policy: NotificationsPolicy,
        os_bridge: Arc<dyn OsNotificationBridge>,
        max_history: usize,
    ) -> Self {
        Self {
            policy,
            subscribers: Mutex::new(Vec::new()),
            history: Mutex::new(VecDeque::new()),
            max_history,
            os_bridge,
            next_id: AtomicU64::new(1),
        }
    }

    pub fn policy(&self) -> NotificationsPolicy {
        self.policy
    }

    pub fn subscribe(&self) -> mpsc::Receiver<NotificationEvent> {
        self.subscribe_for_kinds(None)
    }

    pub fn subscribe_for_kinds(
        &self,
        kinds: Option<Vec<NotificationEventKind>>,
    ) -> mpsc::Receiver<NotificationEvent> {
        let (tx, rx) = mpsc::channel();
        if !self.policy_allows_in_app() {
            return rx;
        }
        let mut subscribers = lock_or_recover(&self.subscribers);
        subscribers.push(Subscriber {
            kind_filter: kinds,
            tx,
        });
        rx
    }

    pub fn history(&self) -> Vec<NotificationEvent> {
        lock_or_recover(&self.history).iter().cloned().collect()
    }

    pub fn publish(&self, event: NotificationEvent) -> PublishResult {
        let mut in_app_deliveries = 0;
        let mut os_attempted = false;
        let mut os_delivered = false;
        let mut os_error = None;

        if self.policy_allows_in_app() {
            let mut subscribers = lock_or_recover(&self.subscribers);
            subscribers.retain(|subscriber| {
                if !subscriber.matches(&event) {
                    return true;
                }

                match subscriber.tx.send(event.clone()) {
                    Ok(()) => {
                        in_app_deliveries += 1;
                        true
                    }
                    Err(_) => false,
                }
            });

            if self.max_history > 0 {
                let mut history = lock_or_recover(&self.history);
                history.push_back(event.clone());
                while history.len() > self.max_history {
                    history.pop_front();
                }
            }
        }

        if self.policy_allows_os() {
            os_attempted = true;
            match self.os_bridge.send(&event) {
                Ok(()) => {
                    os_delivered = true;
                }
                Err(error) => {
                    os_error = Some(error);
                }
            }
        }

        PublishResult {
            in_app_deliveries,
            os_attempted,
            os_delivered,
            os_error,
        }
    }

    pub fn publish_task_done(
        &self,
        title: impl Into<String>,
        message: impl Into<String>,
        block_id: Option<String>,
    ) -> PublishResult {
        self.publish(self.make_event(
            NotificationEventKind::TaskDone,
            title.into(),
            message.into(),
            block_id,
        ))
    }

    pub fn publish_task_failed(
        &self,
        title: impl Into<String>,
        message: impl Into<String>,
        block_id: Option<String>,
    ) -> PublishResult {
        self.publish(self.make_event(
            NotificationEventKind::TaskFailed,
            title.into(),
            message.into(),
            block_id,
        ))
    }

    pub fn publish_approval_required(
        &self,
        title: impl Into<String>,
        message: impl Into<String>,
        block_id: Option<String>,
    ) -> PublishResult {
        self.publish(self.make_event(
            NotificationEventKind::ApprovalRequired,
            title.into(),
            message.into(),
            block_id,
        ))
    }

    fn make_event(
        &self,
        kind: NotificationEventKind,
        title: String,
        message: String,
        block_id: Option<String>,
    ) -> NotificationEvent {
        NotificationEvent {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            kind,
            title,
            message,
            block_id,
        }
    }

    fn policy_allows_in_app(&self) -> bool {
        matches!(
            self.policy,
            NotificationsPolicy::InAppOnly | NotificationsPolicy::InAppAndOs
        )
    }

    fn policy_allows_os(&self) -> bool {
        matches!(
            self.policy,
            NotificationsPolicy::OsOnly | NotificationsPolicy::InAppAndOs
        )
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingBridge {
        sent: Mutex<Vec<NotificationEvent>>,
        fail: bool,
    }

    impl OsNotificationBridge for RecordingBridge {
        fn send(&self, event: &NotificationEvent) -> Result<(), String> {
            if self.fail {
                return Err("bridge failure".to_string());
            }
            let mut sent = lock_or_recover(&self.sent);
            sent.push(event.clone());
            Ok(())
        }
    }

    #[test]
    fn fanout_to_subscribers() {
        let bus = NotificationBus::new(NotificationsPolicy::InAppAndOs);
        let rx1 = bus.subscribe();
        let rx2 = bus.subscribe();

        let event = NotificationEvent {
            id: 7,
            kind: NotificationEventKind::TaskDone,
            title: "build complete".to_string(),
            message: "all checks passed".to_string(),
            block_id: Some("block-1".to_string()),
        };

        let result = bus.publish(event.clone());

        assert_eq!(result.in_app_deliveries, 2);
        assert!(result.os_attempted);
        assert!(result.os_delivered);
        assert!(result.os_error.is_none());
        assert_eq!(rx1.recv().unwrap(), event);
        assert_eq!(rx2.recv().unwrap(), event);
    }

    #[test]
    fn kind_filtered_subscription_receives_matching_events_only() {
        let bus = NotificationBus::new(NotificationsPolicy::InAppOnly);
        let rx = bus.subscribe_for_kinds(Some(vec![NotificationEventKind::TaskFailed]));

        bus.publish_task_done("done", "ok", None);
        bus.publish_task_failed("failed", "boom", Some("block-2".to_string()));

        let received = rx.recv().unwrap();
        assert_eq!(received.kind, NotificationEventKind::TaskFailed);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn os_only_policy_skips_in_app_history_and_subscribers() {
        let bridge = Arc::new(RecordingBridge::default());
        let bus = NotificationBus::with_os_bridge(NotificationsPolicy::OsOnly, bridge.clone());
        let rx = bus.subscribe();

        let result = bus.publish_task_done("done", "ok", None);

        assert_eq!(result.in_app_deliveries, 0);
        assert!(result.os_attempted);
        assert!(result.os_delivered);
        assert!(result.os_error.is_none());
        assert!(rx.try_recv().is_err());
        assert!(bus.history().is_empty());

        let sent = bridge.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, NotificationEventKind::TaskDone);
    }

    #[test]
    fn in_app_only_policy_skips_os_bridge() {
        let bridge = Arc::new(RecordingBridge::default());
        let bus = NotificationBus::with_os_bridge(NotificationsPolicy::InAppOnly, bridge.clone());
        let rx = bus.subscribe();

        let result = bus.publish_task_failed("failed", "boom", None);

        assert_eq!(result.in_app_deliveries, 1);
        assert!(!result.os_attempted);
        assert!(!result.os_delivered);
        assert!(result.os_error.is_none());
        assert_eq!(rx.recv().unwrap().kind, NotificationEventKind::TaskFailed);

        let sent = bridge.sent.lock().unwrap().clone();
        assert!(sent.is_empty());
    }

    #[test]
    fn helper_methods_emit_expected_kinds_and_increment_ids() {
        let bus = NotificationBus::new(NotificationsPolicy::InAppOnly);
        let rx = bus.subscribe();

        bus.publish_task_done("done", "ok", None);
        bus.publish_task_failed("failed", "boom", None);
        bus.publish_approval_required("approval", "needs user", None);

        let e1 = rx.recv().unwrap();
        let e2 = rx.recv().unwrap();
        let e3 = rx.recv().unwrap();

        assert_eq!(e1.kind, NotificationEventKind::TaskDone);
        assert_eq!(e2.kind, NotificationEventKind::TaskFailed);
        assert_eq!(e3.kind, NotificationEventKind::ApprovalRequired);
        assert!(e1.id < e2.id && e2.id < e3.id);

        let history = bus.history();
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn history_is_bounded_and_keeps_latest_events() {
        let bus = NotificationBus::with_os_bridge_and_max_history(
            NotificationsPolicy::InAppOnly,
            Arc::new(NoopOsNotificationBridge),
            2,
        );

        bus.publish_task_done("done", "ok", None);
        bus.publish_task_failed("failed", "boom", None);
        bus.publish_approval_required("approval", "needs user", None);

        let history = bus.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].kind, NotificationEventKind::TaskFailed);
        assert_eq!(history[1].kind, NotificationEventKind::ApprovalRequired);
    }

    #[test]
    fn max_history_zero_disables_history_storage() {
        let bus = NotificationBus::with_os_bridge_and_max_history(
            NotificationsPolicy::InAppOnly,
            Arc::new(NoopOsNotificationBridge),
            0,
        );

        bus.publish_task_done("done", "ok", None);
        bus.publish_task_failed("failed", "boom", None);

        assert!(bus.history().is_empty());
    }

    #[test]
    fn dead_subscribers_are_cleaned_and_publish_continues() {
        let bus = NotificationBus::new(NotificationsPolicy::InAppOnly);
        let dropped = bus.subscribe();
        let live = bus.subscribe();
        drop(dropped);

        let first = bus.publish_task_done("done", "ok", None);
        assert_eq!(first.in_app_deliveries, 1);
        assert_eq!(live.recv().unwrap().kind, NotificationEventKind::TaskDone);

        let second = bus.publish_task_failed("failed", "boom", None);
        assert_eq!(second.in_app_deliveries, 1);
        assert_eq!(live.recv().unwrap().kind, NotificationEventKind::TaskFailed);
    }

    #[test]
    fn os_only_policy_does_not_store_subscribers() {
        let bus = NotificationBus::new(NotificationsPolicy::OsOnly);

        for _ in 0..8 {
            let rx = bus.subscribe();
            drop(rx);
        }

        assert_eq!(lock_or_recover(&bus.subscribers).len(), 0);
    }

    #[test]
    fn os_bridge_error_is_reported_without_panicking() {
        let bridge = Arc::new(RecordingBridge {
            sent: Mutex::new(Vec::new()),
            fail: true,
        });
        let bus = NotificationBus::with_os_bridge(NotificationsPolicy::InAppAndOs, bridge);

        let result = bus.publish_task_done("done", "ok", None);

        assert!(result.os_attempted);
        assert!(!result.os_delivered);
        assert_eq!(result.os_error.as_deref(), Some("bridge failure"));
    }
}
