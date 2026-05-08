//! Typed event bus — the communication backbone between plugins.
//!
//! Plugins **never call each other directly**. Instead they publish typed
//! events onto the bus and subscribe to events they care about. This enforces
//! the same loose coupling principle used in VS Code's extension API.
//!
//! # Design: Type-Erased Event Bus
//!
//! The [`EventBus`] trait uses **type-erased** methods (`event_type: &str` +
//! serialized `serde_json::Value`) so it can be used as `dyn EventBus` behind
//! `Arc`. The typed convenience wrappers ([`EventBusExt::publish_typed`] and
//! [`EventBusExt::subscribe_typed`]) add generic type safety on top.
//!
//! # Example
//!
//! ```rust,ignore
//! use freebox_core::event::{EventBusExt, FileUploaded};
//!
//! // Publisher (using typed extension method)
//! ctx.event_bus.publish_typed(FileUploaded {
//!     file_id: file.id,
//!     user_id: user.id,
//!     size_bytes: file.size,
//!     content_hash: hash.to_string(),
//! }).await?;
//! ```

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Result;

// ---------------------------------------------------------------------------
// Event trait
// ---------------------------------------------------------------------------

/// Marker trait for all platform events.
///
/// Every event must be `Send + Sync + Clone + 'static` so it can be passed
/// across async task boundaries and fanned out to multiple subscribers.
/// `Serialize + Deserialize` enables persistence (audit log) and remote relay.
pub trait Event: Send + Sync + Clone + Serialize + for<'de> Deserialize<'de> + 'static {
    /// A stable string identifier used for routing (e.g. `"file.uploaded"`).
    fn event_type() -> &'static str;
}

// ---------------------------------------------------------------------------
// Boxed handler (type-erased)
// ---------------------------------------------------------------------------

/// A type-erased async handler that receives serialized event payload bytes.
pub type RawHandler = Box<
    dyn Fn(serde_json::Value) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync,
>;

// ---------------------------------------------------------------------------
// Subscription handle
// ---------------------------------------------------------------------------

/// An opaque handle to an active event subscription.
///
/// Dropping this handle **cancels** the subscription — the subscriber will
/// no longer receive events. This RAII pattern prevents subscription leaks.
pub struct Subscription {
    pub(crate) id: Uuid,
    /// Called when the subscription is dropped.
    pub(crate) cancel: Box<dyn FnOnce() + Send + Sync>,
}

impl Subscription {
    /// Create a subscription for testing purposes.
    pub fn new_test(id: Uuid) -> Self {
        Self {
            id,
            cancel: Box::new(|| {}),
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        tracing::trace!(subscription_id = %self.id, "Subscription cancelled");
        let cancel = std::mem::replace(&mut self.cancel, Box::new(|| {}));
        cancel();
    }
}

// ---------------------------------------------------------------------------
// EventBus trait (OBJECT-SAFE — no generics)
// ---------------------------------------------------------------------------

/// The asynchronous typed event bus.
///
/// **Object-safe**: all methods use `&str` event type + `serde_json::Value`
/// (no generic parameters) so this trait can be used as `Arc<dyn EventBus>`.
///
/// Use the [`EventBusExt`] extension trait for type-safe publish/subscribe.
#[async_trait::async_trait]
pub trait EventBus: Send + Sync {
    /// Publish a serialized event to all subscribers of `event_type`.
    ///
    /// Failures in individual subscribers are logged but do not propagate.
    async fn publish_raw(&self, event_type: &str, payload: serde_json::Value) -> Result<()>;

    /// Subscribe to all events matching `event_type`.
    ///
    /// Returns a [`Subscription`] handle. Active until the handle is dropped.
    async fn subscribe_raw(&self, event_type: &str, handler: RawHandler) -> Result<Subscription>;
}

// ---------------------------------------------------------------------------
// EventBusExt — typed convenience wrappers (blanket impl)
// ---------------------------------------------------------------------------

/// Extension trait providing type-safe publish/subscribe on any `EventBus`.
///
/// This is automatically implemented for all `dyn EventBus` / `Arc<dyn EventBus>`.
#[async_trait::async_trait]
pub trait EventBusExt: EventBus {
    /// Publish a typed event. Serializes to JSON then delegates to [`publish_raw`].
    async fn publish_typed<E: Event>(&self, event: E) -> Result<()> {
        let payload =
            serde_json::to_value(&event).map_err(|e| crate::error::Error::Internal(e.into()))?;
        self.publish_raw(E::event_type(), payload).await
    }

    /// Subscribe to a typed event. Deserializes from JSON for each delivery.
    async fn subscribe_typed<E, F, Fut>(&self, handler: F) -> Result<Subscription>
    where
        E: Event,
        F: Fn(E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let raw_handler: RawHandler =
            Box::new(move |value| match serde_json::from_value::<E>(value) {
                Ok(event) => Box::pin(handler(event)),
                Err(e) => Box::pin(async move {
                    tracing::error!(error = %e, "Failed to deserialize event");
                    Err(crate::error::Error::Serde(e))
                }),
            });
        self.subscribe_raw(E::event_type(), raw_handler).await
    }
}

/// Blanket impl: every EventBus automatically gets the typed wrappers.
impl<T: EventBus + ?Sized> EventBusExt for T {}

// ---------------------------------------------------------------------------
// NoopEventBus — for testing and CLI (no event system needed)
// ---------------------------------------------------------------------------

/// A no-op event bus that silently discards all published events.
///
/// Use this in tests and in the CLI where the event system is not needed.
pub struct NoopEventBus;

#[async_trait::async_trait]
impl EventBus for NoopEventBus {
    async fn publish_raw(&self, _event_type: &str, _payload: serde_json::Value) -> Result<()> {
        Ok(())
    }

    async fn subscribe_raw(&self, _event_type: &str, _handler: RawHandler) -> Result<Subscription> {
        Ok(Subscription {
            id: Uuid::new_v4(),
            cancel: Box::new(|| {}),
        })
    }
}

// ---------------------------------------------------------------------------
// Platform events — canonical list
// ---------------------------------------------------------------------------

// --- File events ---

/// Emitted after a file's encrypted chunks have been committed to storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileUploaded {
    pub file_id: Uuid,
    pub user_id: Uuid,
    /// Total plaintext size in bytes (known to the uploader, encrypted on wire).
    pub size_bytes: u64,
    /// BLAKE3 hash of the plaintext file (used for dedup).
    pub content_hash: String,
}
impl Event for FileUploaded {
    fn event_type() -> &'static str {
        "file.uploaded"
    }
}

/// Emitted when a file is deleted (moved to trash or permanently removed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDeleted {
    pub file_id: Uuid,
    pub user_id: Uuid,
    pub permanent: bool,
}
impl Event for FileDeleted {
    fn event_type() -> &'static str {
        "file.deleted"
    }
}

// --- Messaging events ---

/// Emitted when an encrypted message is delivered to the server relay.
/// The `payload` field is opaque ciphertext — the server never decrypts it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageDelivered {
    pub message_id: Uuid,
    pub chat_id: Uuid,
    pub sender_id: Uuid,
    /// Encrypted ciphertext (Signal Double Ratchet output).
    pub encrypted_payload: Vec<u8>,
}
impl Event for MessageDelivered {
    fn event_type() -> &'static str {
        "message.delivered"
    }
}

// --- User events ---

/// Emitted when a new user completes registration and uploads their prekey bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserRegistered {
    pub user_id: Uuid,
    pub username: String,
}
impl Event for UserRegistered {
    fn event_type() -> &'static str {
        "user.registered"
    }
}

// --- Storage events ---

/// Emitted when a storage provider runs low on space (< 10% free).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageCapacityWarning {
    pub provider_id: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
}
impl Event for StorageCapacityWarning {
    fn event_type() -> &'static str {
        "storage.capacity_warning"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_strings_are_stable() {
        assert_eq!(FileUploaded::event_type(), "file.uploaded");
        assert_eq!(FileDeleted::event_type(), "file.deleted");
        assert_eq!(MessageDelivered::event_type(), "message.delivered");
        assert_eq!(UserRegistered::event_type(), "user.registered");
        assert_eq!(
            StorageCapacityWarning::event_type(),
            "storage.capacity_warning"
        );
    }

    #[test]
    fn events_serialize_round_trip() {
        let event = FileUploaded {
            file_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            size_bytes: 42,
            content_hash: "abc123".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        let back: FileUploaded = serde_json::from_value(json).unwrap();
        assert_eq!(event, back);
    }

    #[tokio::test]
    async fn noop_event_bus_accepts_publish() {
        let bus = NoopEventBus;
        let result = bus
            .publish_typed(FileUploaded {
                file_id: Uuid::new_v4(),
                user_id: Uuid::new_v4(),
                size_bytes: 0,
                content_hash: String::new(),
            })
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn noop_event_bus_returns_subscription() {
        let bus = NoopEventBus;
        let sub = bus
            .subscribe_raw("file.uploaded", Box::new(|_| Box::pin(async { Ok(()) })))
            .await;
        assert!(sub.is_ok());
    }

    #[test]
    fn subscription_can_be_dropped_safely() {
        let sub = Subscription::new_test(Uuid::new_v4());
        drop(sub); // should not panic
    }
}
