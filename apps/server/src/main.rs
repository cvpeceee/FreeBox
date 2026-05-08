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
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod api;
mod config;
mod error;
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
        .connect(&cfg.database_url)
        .await?;

    // Run any pending migrations automatically on startup.
    // Migrations live in apps/server/migrations/
    sqlx::migrate!("./migrations").run(&db).await?;
    tracing::info!("Database migrations applied");

    // --- Application state ---
    let storage = build_storage_provider(&cfg).await?;
    let state = state::AppState::new(cfg.clone(), db, storage);

    // --- Router ---
    let app = api::router(state);

    // --- Bind & serve ---
    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!(addr = %addr, "Server listening");

    axum::serve(listener, app)
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
        provider => {
            anyhow::bail!("unsupported STORAGE_PROVIDER `{provider}`; currently supported: local")
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
