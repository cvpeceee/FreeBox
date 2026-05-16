//! FreeBox server — entry point.
//!
//! # Startup Sequence
//!
//! 1. Load configuration from environment (`.env` file or system env vars)
//! 2. Initialize structured logging (JSON in production, pretty in dev)
//! 3. Connect to PostgreSQL and run pending migrations
//! 4. Build the plugin registry and load configured storage backend
//! 5. Build the Axum router with all API routes
//! 6. Bind the TCP listener and start serving requests
//! 7. On SIGTERM/SIGINT: graceful shutdown (drain in-flight requests, unload plugins)

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use freebox_core::{NoopEventBus, Plugin, PluginContext, StorageProvider};
use freebox_storage_s3::S3Plugin;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod api;
mod config;
mod error;
mod rate_limit;
mod state;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env before anything else so config structs can read env vars.
    dotenvy::dotenv().ok();

    // --- Structured logging ---
    // RUST_LOG controls verbosity (e.g. `freebox_server=debug,tower_http=info`).
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(fmt::layer().json()) // JSON output for log aggregators (Loki, Datadog, etc.)
        .init();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "FreeBox server starting"
    );

    // --- Configuration ---
    let cfg = config::Config::from_env()?;
    tracing::info!(host = %cfg.host, port = cfg.port, "Configuration loaded");

    // --- Database ---
    tracing::info!(url = %cfg.database_url, "Connecting to PostgreSQL");

    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(cfg.db_pool_size)
        .min_connections(0)
        // Re-check connections before handing them out.  Connections killed
        // by corporate endpoint-security software (error 10053) report an
        // immediate error on the ping, so this is fast and prevents handing
        // a dead socket to a handler.
        .test_before_acquire(true)
        // Close connections almost immediately after they're returned to the
        // pool.  Corporate endpoint-protection software on this machine
        // (hpiit policy) aborts idle TCP connections to port 5432 within
        // seconds (WSAECONNABORTED / os error 10053).  By setting a very
        // short idle timeout we close connections voluntarily before the
        // security software can kill them.  Each new request creates a fresh
        // connection (~10-40 ms for local Docker PostgreSQL).
        .idle_timeout(std::time::Duration::from_millis(500))
        // Fail fast if no connection is available.
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&cfg.database_url)
        .await?;

    // Run any pending migrations automatically on startup.
    // Migrations live in apps/server/migrations/
    sqlx::migrate!("./migrations").run(&db).await?;
    tracing::info!("Database migrations applied");

    // --- Application state ---
    let storage = build_storage_provider(&cfg).await?;
    let state = state::AppState::new(cfg.clone(), db, storage);

    // --- Background tasks ---
    // Orphaned upload cleanup: runs once per hour, hard-deletes staging upload
    // records (and their chunk blobs) that are older than 24 hours. These are
    // uploads that were never completed, e.g. due to a client crash.
    tokio::spawn(cleanup_orphaned_uploads_loop(state.clone()));

    // --- Router ---
    let app = api::router(state);

    // --- Bind & serve ---
    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!(addr = %addr, "Server listening");

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Server shutdown complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// Storage provider bootstrap
// ---------------------------------------------------------------------------

async fn build_storage_provider(cfg: &config::Config) -> anyhow::Result<Arc<dyn StorageProvider>> {
    match cfg.storage_provider.trim().to_ascii_lowercase().as_str() {
        "local" => {
            let plugin = Arc::new(freebox_storage_local::LocalPlugin::new());
            let mut plugin_config = HashMap::new();
            plugin_config.insert(
                "root".to_owned(),
                serde_json::Value::String(cfg.storage_local_root.clone()),
            );

            let ctx = PluginContext {
                event_bus: Arc::new(NoopEventBus),
                config: plugin_config,
                instance_id: uuid::Uuid::new_v4(),
            };

            plugin
                .on_load(&ctx)
                .await
                .map_err(|e| anyhow::anyhow!("failed to load local storage provider: {e}"))?;

            tracing::info!(
                provider = plugin.id(),
                root = %cfg.storage_local_root,
                "Storage provider loaded"
            );

            Ok(plugin)
        }

        "s3" | "r2" => {
            if cfg.storage_s3_bucket.is_empty() {
                anyhow::bail!("STORAGE_S3_BUCKET must be set when STORAGE_PROVIDER=s3");
            }

            let plugin = Arc::new(S3Plugin::new());
            let mut plugin_config = HashMap::new();

            macro_rules! insert_str {
                ($key:expr, $val:expr) => {
                    plugin_config.insert($key.to_owned(), serde_json::Value::String($val.clone()));
                };
            }

            insert_str!("bucket", cfg.storage_s3_bucket);
            insert_str!("region", cfg.storage_s3_region);

            if !cfg.storage_s3_endpoint.is_empty() {
                insert_str!("endpoint", cfg.storage_s3_endpoint);
            }
            if !cfg.storage_s3_access_key.is_empty() {
                insert_str!("access_key", cfg.storage_s3_access_key);
                insert_str!("secret_key", cfg.storage_s3_secret_key);
            }

            let ctx = PluginContext {
                event_bus: Arc::new(NoopEventBus),
                config: plugin_config,
                instance_id: uuid::Uuid::new_v4(),
            };

            plugin
                .on_load(&ctx)
                .await
                .map_err(|e| anyhow::anyhow!("failed to load S3 storage provider: {e}"))?;

            tracing::info!(
                provider = plugin.id(),
                bucket = %cfg.storage_s3_bucket,
                endpoint = %cfg.storage_s3_endpoint,
                "Storage provider loaded"
            );

            Ok(plugin)
        }
        provider => {
            anyhow::bail!(
                "unsupported STORAGE_PROVIDER `{provider}`; currently supported: local, s3"
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Background: orphaned upload cleanup
// ---------------------------------------------------------------------------

/// Runs forever; calls [`purge_orphaned_uploads`] once per hour.
async fn cleanup_orphaned_uploads_loop(state: state::AppState) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    // The first tick fires immediately, but we skip it to avoid running at
    // startup before the server has finished initialising.
    interval.tick().await;
    loop {
        interval.tick().await;
        purge_orphaned_uploads(&state).await;
    }
}

/// Delete uploads older than 24 hours plus all their stored chunk blobs.
///
/// These are uploads where the client started a chunked upload but never
/// sent the `/complete` request. This can happen if the client crashes,
/// loses network connectivity, or is simply abandoned.
async fn purge_orphaned_uploads(state: &state::AppState) {
    // Fetch all orphaned upload records.
    let rows = match sqlx::query(
        r#"
        SELECT id, total_chunks
        FROM uploads
        WHERE created_at < NOW() - INTERVAL '24 hours'
        "#,
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "Failed to fetch orphaned uploads");
            return;
        }
    };

    if rows.is_empty() {
        return;
    }

    tracing::info!(count = rows.len(), "Purging orphaned uploads");

    for row in &rows {
        use sqlx::Row;
        let upload_id: uuid::Uuid = row.get("id");
        let total_chunks: i32 = row.get("total_chunks");

        // Delete every chunk blob that may have been stored for this upload.
        // We use list() so we only touch keys that actually exist, avoiding
        // spurious errors for partial uploads.
        let prefix = format!("chunks/{upload_id}/");
        match state.storage.list(&prefix).await {
            Ok(objects) => {
                for obj in objects {
                    if let Err(e) = state.storage.delete(&obj.key).await {
                        tracing::warn!(
                            upload_id = %upload_id,
                            key = %obj.key,
                            error = %e,
                            "Failed to delete orphaned chunk blob"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    upload_id = %upload_id,
                    error = %e,
                    "Failed to list chunks for orphaned upload; skipping blob cleanup"
                );
            }
        }

        // Remove the DB record regardless of whether blob cleanup succeeded.
        // Missing blobs are harmless (they simply won't exist); a lingering
        // upload record would cause the cleanup to retry indefinitely.
        if let Err(e) = sqlx::query("DELETE FROM uploads WHERE id = $1")
            .bind(upload_id)
            .execute(&state.db)
            .await
        {
            tracing::error!(
                upload_id = %upload_id,
                error = %e,
                "Failed to delete orphaned upload record"
            );
        } else {
            tracing::info!(
                upload_id = %upload_id,
                chunks_possible = total_chunks,
                "Orphaned upload purged"
            );
        }
    }
}

/// Wait for SIGTERM or SIGINT (Ctrl-C) to trigger a graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    // Windows has no SIGTERM; only listen for Ctrl-C.
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c    => tracing::info!("Received Ctrl-C, shutting down"),
        _ = terminate => tracing::info!("Received SIGTERM, shutting down"),
    }
}
