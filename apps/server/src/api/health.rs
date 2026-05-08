//! Health check handlers — used by load balancers and container orchestration.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

/// `GET /health` — liveness probe.
///
/// Returns 200 OK immediately. No auth, no DB check — just proves the process
/// is running. Kubernetes/Docker uses this to decide whether to restart the pod.
pub async fn liveness() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") })),
    )
}
