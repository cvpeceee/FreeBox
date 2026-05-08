//! File operation handlers — upload, download, list, delete.
//!
//! # Upload Protocol (Chunked, Resumable)
//!
//! FreeBox uses a three-phase upload protocol compatible with the TUS standard:
//!
//! ```
//! Phase 1: POST /files/upload/init
//!   → Server creates an upload record, returns upload_id
//!   → Client receives upload_id and chunk_size
//!
//! Phase 2 (repeated per chunk): PUT /files/upload/:upload_id
//!   → Client sends one encrypted chunk at a time
//!   → Server stores the ciphertext blob, records which chunks arrived
//!   → Can resume if interrupted — just re-send missing chunks
//!
//! Phase 3: POST /files/upload/:upload_id/complete
//!   → Server verifies all chunks received
//!   → Creates the file record in the database
//!   → Returns the permanent file_id
//! ```
//!
//! # Security
//!
//! The server never sees plaintext. Every byte stored is already encrypted
//! with AES-256-GCM on the client before transmission.
//!
//! The server stores:
//! - The encrypted chunk blobs (opaque bytes)
//! - The encrypted key envelope (opaque bytes, only the recipient can unwrap)
//! - Metadata: file_id, chunk count, upload timestamp, user_id

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    api::auth::Claims,
    error::{AppError, Result},
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct UploadInitRequest {
    /// Total number of chunks the client will upload.
    pub total_chunks: u32,
    /// Total plaintext size in bytes (for progress display on other clients).
    pub size_bytes: u64,
    /// Encrypted file key envelope — only the owner can unwrap this.
    /// Stored as base64-encoded bytes.
    pub encrypted_key_envelope: String,
    /// BLAKE3 hash of the complete plaintext file (hex string).
    /// Used for server-side deduplication lookup (the hash itself is safe to
    /// store — it reveals nothing about the content without the key).
    pub content_hash: String,
    /// Encrypted file name (base64). The server cannot read it.
    pub encrypted_name: String,
}

#[derive(Serialize)]
pub struct UploadInitResponse {
    pub upload_id: Uuid,
    /// Confirmed chunk size in bytes the server expects.
    pub chunk_size: u32,
}

#[derive(Serialize)]
pub struct UploadCompleteResponse {
    pub file_id: Uuid,
}

#[derive(Serialize)]
pub struct FileMetaResponse {
    pub file_id: Uuid,
    pub encrypted_name: String,
    pub size_bytes: u64,
    pub total_chunks: u32,
    pub encrypted_key_envelope: String,
    pub content_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct FileListResponse {
    pub files: Vec<FileMetaResponse>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/v1/files/upload/init`  — Phase 1: initialise a chunked upload.
pub async fn upload_init(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<UploadInitRequest>,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;
    let upload_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO uploads (
            id, user_id, total_chunks, chunks_received,
            size_bytes, encrypted_key_envelope, content_hash,
            encrypted_name, created_at
        )
        VALUES ($1, $2, $3, 0, $4, $5, $6, $7, NOW())
        "#,
    )
    .bind(upload_id)
    .bind(user_id)
    .bind(req.total_chunks as i32)
    .bind(req.size_bytes as i64)
    .bind(&req.encrypted_key_envelope)
    .bind(&req.content_hash)
    .bind(&req.encrypted_name)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    tracing::debug!(upload_id = %upload_id, user_id = %user_id, "Upload initialised");

    Ok((
        StatusCode::CREATED,
        Json(UploadInitResponse {
            upload_id,
            chunk_size: 4 * 1024 * 1024, // 4 MiB
        }),
    ))
}

/// `PUT /api/v1/files/upload/:upload_id` — Phase 2: upload one encrypted chunk.
///
/// The chunk index is specified via the `X-Chunk-Index` header.
/// The body is raw ciphertext bytes (AES-256-GCM output).
pub async fn upload_chunk(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(upload_id): Path<Uuid>,
    // Body handled by storage plugin — placeholder here.
    body: axum::body::Bytes,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;

    // Verify the upload belongs to this user.
    let _upload = sqlx::query(
        "SELECT id, total_chunks, chunks_received FROM uploads WHERE id = $1 AND user_id = $2",
    )
    .bind(upload_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    .ok_or(AppError::NotFound(format!("upload {upload_id}")))?;

    // In production: stream `body` directly to the storage provider.
    // The storage key is deterministic: `chunks/{upload_id}/{chunk_index}`.
    // Here we validate the body is non-empty.
    if body.is_empty() {
        return Err(AppError::BadRequest("chunk body must not be empty".into()));
    }

    // Update the received chunk count.
    sqlx::query("UPDATE uploads SET chunks_received = chunks_received + 1 WHERE id = $1")
        .bind(upload_id)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    tracing::trace!(upload_id = %upload_id, "Chunk received");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/files/upload/:upload_id/complete` — Phase 3: finalise upload.
pub async fn upload_complete(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(upload_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;

    let upload = sqlx::query(
        r#"
        SELECT id, total_chunks, chunks_received,
               size_bytes, encrypted_key_envelope,
               content_hash, encrypted_name
        FROM uploads
        WHERE id = $1 AND user_id = $2
        "#,
    )
    .bind(upload_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    .ok_or(AppError::NotFound(format!("upload {upload_id}")))?;

    // Verify all chunks arrived.
    if upload.get::<i32, _>("chunks_received") < upload.get::<i32, _>("total_chunks") {
        return Err(AppError::BadRequest(format!(
            "missing chunks: expected {}, received {}",
            upload.get::<i32, _>("total_chunks"),
            upload.get::<i32, _>("chunks_received")
        )));
    }

    // Promote the upload to a permanent file record.
    let file_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO files (
            id, user_id, total_chunks, size_bytes,
            encrypted_key_envelope, content_hash, encrypted_name, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        "#,
    )
    .bind(file_id)
    .bind(user_id)
    .bind(upload.get::<i32, _>("total_chunks"))
    .bind(upload.get::<i64, _>("size_bytes"))
    .bind(upload.get::<String, _>("encrypted_key_envelope"))
    .bind(upload.get::<String, _>("content_hash"))
    .bind(upload.get::<String, _>("encrypted_name"))
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    // Clean up the staging upload record.
    sqlx::query("DELETE FROM uploads WHERE id = $1")
        .bind(upload_id)
        .execute(&state.db)
        .await
        .ok(); // best-effort

    tracing::info!(file_id = %file_id, user_id = %user_id, "File upload complete");
    Ok((
        StatusCode::CREATED,
        Json(UploadCompleteResponse { file_id }),
    ))
}

/// `GET /api/v1/files` — list all files for the authenticated user.
pub async fn list_files(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;

    let rows = sqlx::query(
        r#"
        SELECT id, encrypted_name, size_bytes, total_chunks,
               encrypted_key_envelope, content_hash, created_at
        FROM files
        WHERE user_id = $1 AND deleted_at IS NULL
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    let files = rows
        .into_iter()
        .map(|r| FileMetaResponse {
            file_id: r.get::<Uuid, _>("id"),
            encrypted_name: r.get::<String, _>("encrypted_name"),
            size_bytes: r.get::<i64, _>("size_bytes") as u64,
            total_chunks: r.get::<i32, _>("total_chunks") as u32,
            encrypted_key_envelope: r.get::<String, _>("encrypted_key_envelope"),
            content_hash: r.get::<String, _>("content_hash"),
            created_at: r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        })
        .collect();

    Ok(Json(FileListResponse { files }))
}

/// `GET /api/v1/files/:file_id` — get metadata for a single file.
pub async fn get_file_meta(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(file_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;

    let row = sqlx::query(
        r#"
        SELECT id, encrypted_name, size_bytes, total_chunks,
               encrypted_key_envelope, content_hash, created_at
        FROM files
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(file_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    .ok_or(AppError::NotFound(format!("file {file_id}")))?;

    Ok(Json(FileMetaResponse {
        file_id: row.get::<Uuid, _>("id"),
        encrypted_name: row.get::<String, _>("encrypted_name"),
        size_bytes: row.get::<i64, _>("size_bytes") as u64,
        total_chunks: row.get::<i32, _>("total_chunks") as u32,
        encrypted_key_envelope: row.get::<String, _>("encrypted_key_envelope"),
        content_hash: row.get::<String, _>("content_hash"),
        created_at: row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
    }))
}

/// `GET /api/v1/files/:file_id/chunk/:n` — download a single encrypted chunk.
pub async fn download_chunk(
    State(_state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((file_id, chunk_index)): Path<(Uuid, u32)>,
) -> Result<impl IntoResponse> {
    // TODO: Stream the chunk bytes from the storage provider.
    // The storage key is: format!("chunks/{file_id}/{chunk_index:08}")
    tracing::debug!(file_id = %file_id, chunk_index, user = %claims.sub, "Chunk download");
    Err::<StatusCode, _>(AppError::NotFound("storage provider not yet wired".into()))
}

/// `DELETE /api/v1/files/:file_id` — soft-delete (moves to trash).
pub async fn delete_file(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(file_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;

    let affected = sqlx::query(
        "UPDATE files SET deleted_at = NOW() WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
    )
    .bind(file_id)
    .bind(user_id)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound(format!("file {file_id}")));
    }

    tracing::info!(file_id = %file_id, user_id = %user_id, "File deleted (soft)");
    Ok(StatusCode::NO_CONTENT)
}
