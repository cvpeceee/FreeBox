//! Shared application state threaded through every Axum handler via `State<>`.
//!
//! `AppState` is cheap to clone (all inner values are `Arc`-wrapped) and is
//! injected into every request handler automatically by Axum's extractor system.

use std::sync::Arc;

use freebox_core::storage::StorageProvider;
use sqlx::PgPool;

use crate::{
    config::{Config, OAuthConfig},
    rate_limit::{RateLimiter, SharedRateLimiter},
};

/// The shared state available to every request handler.
///
/// Keep this small. Large or rarely-used resources should be lazily initialized
/// behind an `Arc<Mutex<_>>` rather than eagerly constructed here.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: PgPool,
    /// The active storage backend (S3, local, GCS, etc.).
    /// Set during server startup via the plugin registry.
    pub storage: Arc<dyn StorageProvider>,
    /// Shared HTTP client for outbound requests (OAuth token exchange, etc.).
    /// Reusing a single client enables connection pooling and keep-alive.
    pub http_client: reqwest::Client,
    /// OAuth2 provider configuration (providers are opt-in via env vars).
    pub oauth: OAuthConfig,
    /// In-process API rate limiter.
    pub rate_limiter: SharedRateLimiter,
}

impl AppState {
    pub fn new(config: Config, db: PgPool, storage: Arc<dyn StorageProvider>) -> Self {
        let oauth = config.oauth.clone();
        let rate_limiter = Arc::new(RateLimiter::new(
            config.rate_limit_requests,
            std::time::Duration::from_secs(config.rate_limit_window_secs),
        ));
        Self {
            config: Arc::new(config),
            db,
            storage,
            http_client: reqwest::Client::new(),
            oauth,
            rate_limiter,
        }
    }
}
