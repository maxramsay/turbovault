//! NATS event publisher with disk-backed queue fallback.
//!
//! When a NATS connection is available events are published directly. If NATS
//! is unavailable (client is `None` or publish fails) the event is written to
//! the [`LocalEventQueue`] so it can be replayed later via [`drain_queue`].

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::RwLock;
use tracing;

use crate::event_queue::LocalEventQueue;
use crate::events::VaultEvent;

/// Publishes vault events to NATS with automatic queue fallback.
pub struct VaultEventPublisher {
    nats: Arc<RwLock<Option<async_nats::Client>>>,
    queue: Arc<LocalEventQueue>,
    org_id: String,
}

impl VaultEventPublisher {
    /// Create a publisher. If `nats_client` is `None`, events go straight to
    /// the local queue until a connection is established.
    pub fn new(
        nats_client: Option<async_nats::Client>,
        queue: Arc<LocalEventQueue>,
        org_id: String,
    ) -> Self {
        Self {
            nats: Arc::new(RwLock::new(nats_client)),
            queue,
            org_id,
        }
    }

    /// Create a no-op publisher for testing. Events are silently discarded.
    pub fn noop() -> Self {
        let tmp = std::env::temp_dir().join("turbovault_noop_events");
        Self {
            nats: Arc::new(RwLock::new(None)),
            queue: Arc::new(LocalEventQueue::new(&tmp)),
            org_id: "noop".to_string(),
        }
    }

    /// The organisation ID this publisher emits events for.
    pub fn org_id(&self) -> &str {
        &self.org_id
    }

    /// Publish a pre-built [`VaultEvent`]. Tries NATS first; falls back to the
    /// local disk queue on failure.
    pub async fn publish(&self, event: &VaultEvent) {
        let subject = event.subject();
        let payload = match serde_json::to_vec(event) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("failed to serialize event: {e}");
                return;
            }
        };

        let nats_guard = self.nats.read().await;
        if let Some(client) = nats_guard.as_ref() {
            match client
                .publish(subject.clone(), payload.clone().into())
                .await
            {
                Ok(()) => {
                    tracing::debug!(subject = %subject, "published event to NATS");
                    return;
                }
                Err(e) => {
                    tracing::warn!(subject = %subject, "NATS publish failed, queueing locally: {e}");
                }
            }
        }
        drop(nats_guard);

        // Fallback: persist to disk queue.
        self.queue.enqueue(event);
    }

    /// Convenience: build a [`VaultEvent`] from a type string and payload,
    /// then publish it.
    pub async fn emit(&self, event_type: &str, data: &impl Serialize) {
        let event = VaultEvent::new(event_type, &self.org_id, data);
        self.publish(&event).await;
    }

    /// Drain the local queue and attempt to publish each event to NATS.
    ///
    /// Events that still fail to publish are re-enqueued.
    pub async fn drain_queue(&self) {
        let events = self.queue.drain();
        if events.is_empty() {
            return;
        }

        tracing::info!(count = events.len(), "draining local event queue to NATS");

        let nats_guard = self.nats.read().await;
        for event in &events {
            let subject = event.subject();
            let payload = match serde_json::to_vec(event) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("failed to serialize queued event: {e}");
                    self.queue.enqueue(event);
                    continue;
                }
            };

            if let Some(client) = nats_guard.as_ref() {
                if let Err(e) = client.publish(subject.clone(), payload.into()).await {
                    tracing::warn!(subject = %subject, "re-queuing event after NATS failure: {e}");
                    self.queue.enqueue(event);
                }
            } else {
                // Still no NATS — put it back.
                self.queue.enqueue(event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NoteReadEvent;
    use tempfile::TempDir;

    #[tokio::test]
    async fn noop_does_not_panic() {
        let publisher = VaultEventPublisher::noop();
        assert_eq!(publisher.org_id(), "noop");

        publisher
            .emit("vault.note.read", &NoteReadEvent {
                path: "test.md".into(),
            })
            .await;
        // No panic == pass
    }

    #[tokio::test]
    async fn publisher_queues_when_no_nats() {
        let tmp = TempDir::new().unwrap();
        let queue = Arc::new(LocalEventQueue::new(tmp.path()));
        let publisher = VaultEventPublisher::new(None, Arc::clone(&queue), "test-org".into());

        publisher
            .emit("vault.note.read", &NoteReadEvent {
                path: "hello.md".into(),
            })
            .await;

        assert!(queue.has_pending());
        let drained = queue.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].event_type, "vault.note.read");
    }
}
