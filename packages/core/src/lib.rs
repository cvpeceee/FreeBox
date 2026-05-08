//! # freebox-core
//!
//! The plugin API contracts for the FreeBox platform. This crate is the
//! **single source of truth** for all extension points in the system.
//!
//! ## Design Philosophy
//!
//! - **Traits over structs**: Every capability is expressed as a Rust trait.
//!   Plugins implement traits; the kernel dispatches through trait objects.
//! - **Event-driven**: Plugins communicate via a typed event bus, never by
//!   calling each other directly. This enables loose coupling and testability.
//! - **Dual-licensed** (Apache-2.0 OR MIT): Third-party plugin authors can
//!   embed this crate in proprietary code without license friction.
//!
//! ## Crate Layout
//!
//! | Module | Contents |
//! |--------|----------|
//! | [`plugin`] | Plugin manifest, lifecycle, registry |
//! | [`storage`] | [`StorageProvider`] trait + capability flags |
//! | [`event`] | Typed event bus contracts |
//! | [`error`] | Unified [`Error`] and [`Result`] types |

pub mod error;
pub mod event;
pub mod plugin;
pub mod storage;

// Re-export the most commonly used types at the crate root so plugin authors
// can write `use freebox_core::*;` and get everything they need.
pub use async_trait::async_trait;
pub use error::{Error, Result};
pub use event::{Event, EventBus, EventBusExt, NoopEventBus, Subscription};
pub use plugin::{Capabilities, Plugin, PluginContext, PluginManifest};
pub use storage::{ObjectMeta, StorageCapabilities, StorageProvider};
