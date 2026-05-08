//! Unified error types for the FreeBox platform.
//!
//! All crates in this workspace use `freebox_core::Error` (via `thiserror`)
//! so that error handling is consistent across the kernel and plugins.
//!
//! Plugin authors should map their internal errors to [`Error`] variants using
//! `thiserror`'s `#[from]` attribute or `map_err`.

use thiserror::Error;

/// The top-level error type for the FreeBox platform.
#[derive(Debug, Error)]
pub enum Error {
    // --- Storage errors ---
    #[error("Object not found: {key}")]
    NotFound { key: String },

    #[error("Storage backend error: {message}")]
    Storage { message: String },

    // --- Authentication / authorization ---
    #[error("Invalid credentials")]
    Unauthenticated,

    #[error("Access denied: {reason}")]
    Unauthorized { reason: String },

    #[error("Token has expired")]
    TokenExpired,

    // --- Plugin system ---
    #[error("Plugin configuration error: {message}")]
    Config { message: String },

    #[error("Plugin '{id}' failed to load: {reason}")]
    PluginLoad { id: String, reason: String },

    #[error("No storage provider registered")]
    NoStorageProvider,

    // --- Crypto ---
    #[error("Encryption failed: {reason}")]
    Encryption { reason: String },

    #[error("Decryption failed: {reason}")]
    Decryption { reason: String },

    #[error("Key not found for user {user_id}")]
    KeyNotFound { user_id: uuid::Uuid },

    // --- Internal / unexpected ---
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Convenience alias used across the entire workspace.
pub type Result<T, E = Error> = std::result::Result<T, E>;

// Convenience constructors so call sites read cleanly without
// verbose struct syntax.
impl Error {
    pub fn not_found(key: impl Into<String>) -> Self {
        Self::NotFound { key: key.into() }
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::Storage {
            message: message.into(),
        }
    }

    pub fn unauthorized(reason: impl Into<String>) -> Self {
        Self::Unauthorized {
            reason: reason.into(),
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
        }
    }

    pub fn encryption(reason: impl Into<String>) -> Self {
        Self::Encryption {
            reason: reason.into(),
        }
    }

    pub fn decryption(reason: impl Into<String>) -> Self {
        Self::Decryption {
            reason: reason.into(),
        }
    }

    pub fn plugin_load(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::PluginLoad {
            id: id.into(),
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            Error::not_found("key1").to_string(),
            "Object not found: key1"
        );
        assert_eq!(
            Error::storage("disk full").to_string(),
            "Storage backend error: disk full"
        );
        assert_eq!(Error::Unauthenticated.to_string(), "Invalid credentials");
    }

    #[test]
    fn error_convenience_constructors() {
        let e = Error::config("missing foo");
        assert!(matches!(e, Error::Config { .. }));
        let e = Error::encryption("bad nonce");
        assert!(matches!(e, Error::Encryption { .. }));
        let e = Error::plugin_load("test", "boom");
        assert!(matches!(e, Error::PluginLoad { .. }));
    }
}

// Allow converting our Error into an Axum-compatible HTTP response.
// (Implemented in apps/server to avoid a circular dependency.)
