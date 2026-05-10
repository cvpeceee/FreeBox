//! Axum router - assembles all API route groups into a single [`Router`].
//!
//! # Route Layout
//!
//! ```
//! GET  /health                  - liveness probe (no auth)
//! POST /api/v1/auth/register    - create account + upload prekey bundle
//! POST /api/v1/auth/login       - password auth, returns JWT pair
//! POST /api/v1/auth/refresh     - refresh access token
//! POST /api/v1/auth/logout      - revoke refresh token
//!
//! GET  /api/v1/auth/oauth/:provider           - initiate OAuth2 flow
//! GET  /api/v1/auth/oauth/:provider/callback  - OAuth2 callback (code exchange)
//! GET  /api/v1/auth/providers                 - list linked OAuth providers (auth required)
//! GET  /api/v1/auth/oauth/configured           - list server-configured OAuth providers (public)
//! GET  /api/v1/auth/audit-events              - list account audit events (auth required)
//! GET  /api/v1/admin/audit-events             - list account audit events across users (admin only)
//! POST /api/v1/auth/oauth/:provider/link      - link a new OAuth provider (auth required)
//! DELETE /api/v1/auth/oauth/:provider/unlink  - unlink an OAuth provider (auth required)
//!
//! GET  /api/v1/keys/:user_id            - fetch prekey bundle (auth required)
//! POST /api/v1/keys/one-time            - replenish one-time prekeys
//!
//! POST /api/v1/files/upload/init        - begin chunked upload, returns upload_id
//! PUT  /api/v1/files/upload/:upload_id  - upload a single encrypted chunk
//! POST /api/v1/files/upload/:upload_id/complete - finalise upload
//! GET  /api/v1/files                   - list user files (metadata only)
//! GET  /api/v1/files/:file_id          - get file metadata + chunk manifest
//! GET  /api/v1/files/:file_id/chunk/:n - download encrypted chunk n
//! DELETE /api/v1/files/:file_id        - move file to trash
//!
//! GET  /api/v1/storage/datasources      - list storage datasources
//! GET  /api/v1/storage/buckets          - list all buckets (auth required)
//! POST /api/v1/storage/buckets          - create a new bucket (auth required)
//! ```

use axum::{
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, patch, post, put},
    Router,
};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    request_id::{MakeRequestUuid, SetRequestIdLayer},
    trace::TraceLayer,
};

use crate::state::AppState;

pub mod auth;
pub mod files;
pub mod health;
pub mod keys;
pub mod oauth;
pub mod storage;

#[cfg(test)]
mod tests;

/// Build and return the complete Axum router.
pub fn router(state: AppState) -> Router {
    // Middleware stack (applied outermost-first, so request-id is set first):
    let middleware = ServiceBuilder::new()
        // Attach a unique X-Request-Id to every request for log correlation.
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        // Structured HTTP access logs via tracing.
        .layer(TraceLayer::new_for_http())
        // Gzip/Brotli response compression.
        .layer(CompressionLayer::new())
        // CORS — restrict in production via config.
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        );

    // Public routes (no authentication required).
    let public = Router::new()
        .route("/health", get(health::liveness))
        .route(
            "/api/v1/storage/datasources",
            get(storage::list_datasources),
        )
        .route("/api/v1/auth/register", post(auth::register))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/refresh", post(auth::refresh))
        .route("/api/v1/auth/salt/:username", get(auth::get_salt))
        // OAuth2 third-party authentication (GitHub, Google, Microsoft, Apple, Facebook).
        .route("/api/v1/auth/oauth/configured", get(oauth::list_configured_providers))
        .route("/api/v1/auth/oauth/:provider", get(oauth::initiate))
        .route(
            "/api/v1/auth/oauth/:provider/callback",
            get(oauth::callback),
        );

    // Authenticated routes — require a valid Bearer JWT.
    let authenticated = Router::new()
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/providers", get(oauth::list_providers))
        .route("/api/v1/auth/audit-events", get(oauth::list_account_audit_events))
        .route(
            "/api/v1/admin/audit-events",
            get(oauth::list_account_audit_events_admin),
        )
        .route(
            "/api/v1/auth/oauth/:provider/link",
            post(oauth::link_provider),
        )
        .route(
            "/api/v1/auth/oauth/:provider/unlink",
            delete(oauth::unlink_provider),
        )
        .route("/api/v1/keys/:user_id", get(keys::get_bundle))
        .route("/api/v1/keys/one-time", post(keys::replenish_one_time))
        .route("/api/v1/files/upload/init", post(files::upload_init))
        .route(
            "/api/v1/files/upload/:upload_id",
            put(files::upload_chunk)
                .layer(DefaultBodyLimit::max(files::MAX_UPLOAD_CHUNK_BODY_BYTES)),
        )
        .route(
            "/api/v1/files/upload/:upload_id/complete",
            post(files::upload_complete),
        )
        .route("/api/v1/files", get(files::list_files))
        // Static sub-paths must be registered before the dynamic :file_id param.
        .route("/api/v1/files/trash", get(files::list_trash))
        .route("/api/v1/files/:file_id", get(files::get_file_meta))
        .route("/api/v1/files/:file_id", patch(files::rename_file))
        .route("/api/v1/files/:file_id", delete(files::delete_file))
        .route(
            "/api/v1/files/:file_id/chunk/:n",
            get(files::download_chunk),
        )
        .route(
            "/api/v1/files/:file_id/restore",
            post(files::restore_file),
        )
        .route("/api/v1/storage/buckets", get(storage::list_buckets))
        .route("/api/v1/storage/buckets", post(storage::create_bucket))
        // Auth middleware applied to all routes above.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    // Merge public + authenticated under shared middleware.
    Router::new()
        .merge(public)
        .merge(authenticated)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::rate_limit::enforce,
        ))
        .layer(middleware)
        .with_state(state)
}
