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
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
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

/// The CLI currently serializes encrypted chunks as JSON, so a 4 MiB plaintext
/// chunk can become much larger on the wire. Keep this explicit until the
/// upload protocol switches to a compact binary chunk envelope.
pub const MAX_UPLOAD_CHUNK_BODY_BYTES: usize = 32 * 1024 * 1024;

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

#[derive(Deserialize)]
pub struct FileListQuery {
    /// Maximum number of files to return (1–200, default 50).
    pub limit: Option<i64>,
    /// Number of files to skip for pagination (default 0).
    pub offset: Option<i64>,
}

#[derive(Serialize)]
pub struct FileListResponse {
    pub files: Vec<FileMetaResponse>,
    /// Total number of files matching the query (ignores limit/offset).
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

/// File metadata extended with soft-delete timestamp — used in trash listing.
#[derive(Serialize)]
pub struct TrashFileResponse {
    pub file_id: Uuid,
    pub encrypted_name: String,
    pub size_bytes: u64,
    pub total_chunks: u32,
    pub content_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct TrashListResponse {
    pub files: Vec<TrashFileResponse>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Deserialize)]
pub struct RenameFileRequest {
    /// New encrypted file name (AES-GCM encrypted, base64). Must not be empty.
    pub encrypted_name: String,
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
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;
    let chunk_index = parse_chunk_index(&headers)?;

    // Verify the upload belongs to this user.
    let upload = sqlx::query(
        "SELECT id, total_chunks, chunks_received FROM uploads WHERE id = $1 AND user_id = $2",
    )
    .bind(upload_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    .ok_or(AppError::NotFound(format!("upload {upload_id}")))?;

    let total_chunks = upload.get::<i32, _>("total_chunks") as u32;
    if chunk_index >= total_chunks {
        return Err(AppError::BadRequest(format!(
            "chunk index {chunk_index} out of range for upload with {total_chunks} chunks"
        )));
    }

    if body.is_empty() {
        return Err(AppError::BadRequest("chunk body must not be empty".into()));
    }
    validate_chunk_body_size(body.len())?;

    let storage_key = chunk_storage_key(upload_id, chunk_index);
    let already_received = state.storage.exists(&storage_key).await?;

    state.storage.put(&storage_key, body).await?;

    // Re-sending a chunk is allowed for resume/retry, but should not make the
    // upload look more complete than it is.
    if !already_received {
        sqlx::query("UPDATE uploads SET chunks_received = chunks_received + 1 WHERE id = $1")
            .bind(upload_id)
            .execute(&state.db)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    }

    tracing::trace!(
        upload_id = %upload_id,
        chunk_index,
        storage_key = %storage_key,
        "Chunk stored"
    );
    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn parse_chunk_index(headers: &HeaderMap) -> Result<u32> {
    let raw = headers
        .get("x-chunk-index")
        .ok_or_else(|| AppError::BadRequest("missing X-Chunk-Index header".into()))?
        .to_str()
        .map_err(|_| AppError::BadRequest("X-Chunk-Index must be valid ASCII".into()))?;

    raw.parse::<u32>()
        .map_err(|_| AppError::BadRequest("X-Chunk-Index must be a non-negative integer".into()))
}

pub(super) fn chunk_storage_key(file_id: Uuid, chunk_index: u32) -> String {
    format!("chunks/{file_id}/{chunk_index:08}")
}

pub(super) fn validate_chunk_body_size(size: usize) -> Result<()> {
    if size > MAX_UPLOAD_CHUNK_BODY_BYTES {
        return Err(AppError::PayloadTooLarge(format!(
            "chunk body exceeds {} bytes",
            MAX_UPLOAD_CHUNK_BODY_BYTES
        )));
    }
    Ok(())
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

    // Promote the upload to a permanent file record. The upload ID becomes the
    // file ID so chunks written during upload already live at their final key.
    let file_id = upload_id;
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

/// `GET /api/v1/files` — paginated file list for the authenticated user.
///
/// Query parameters: `limit` (1–200, default 50), `offset` (default 0).
pub async fn list_files(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<FileListQuery>,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let offset = params.offset.unwrap_or(0).max(0);

    let rows = sqlx::query(
        r#"
        SELECT id, encrypted_name, size_bytes, total_chunks,
               encrypted_key_envelope, content_hash, created_at,
               COUNT(*) OVER() AS total_count
        FROM files
        WHERE user_id = $1 AND deleted_at IS NULL
        ORDER BY created_at DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    let total = rows.first().map(|r| r.get::<i64, _>("total_count")).unwrap_or(0);

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

    Ok(Json(FileListResponse { files, total, limit, offset }))
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
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((file_id, chunk_index)): Path<(Uuid, u32)>,
) -> Result<impl IntoResponse> {
    let row = sqlx::query(
        r#"
        SELECT total_chunks
        FROM files
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(file_id)
    .bind(claims.sub)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    .ok_or(AppError::NotFound(format!("file {file_id}")))?;

    let total_chunks = row.get::<i32, _>("total_chunks") as u32;
    if chunk_index >= total_chunks {
        return Err(AppError::BadRequest(format!(
            "chunk index {chunk_index} out of range for file with {total_chunks} chunks"
        )));
    }

    let storage_key = chunk_storage_key(file_id, chunk_index);
    let chunk = state.storage.get(&storage_key).await?;

    tracing::debug!(
        file_id = %file_id,
        chunk_index,
        storage_key = %storage_key,
        user = %claims.sub,
        "Chunk downloaded"
    );

    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], chunk))
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

/// `GET /api/v1/files/trash` — list soft-deleted files (trash) for the user.
///
/// Only files deleted within the past 30 days are shown (after that they are
/// eligible for hard deletion by a background job).
/// Query parameters: `limit` (1–200, default 50), `offset` (default 0).
pub async fn list_trash(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<FileListQuery>,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let offset = params.offset.unwrap_or(0).max(0);

    let rows = sqlx::query(
        r#"
        SELECT id, encrypted_name, size_bytes, total_chunks,
               content_hash, created_at, deleted_at,
               COUNT(*) OVER() AS total_count
        FROM files
        WHERE user_id = $1
          AND deleted_at IS NOT NULL
          AND deleted_at > NOW() - INTERVAL '30 days'
        ORDER BY deleted_at DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    let total = rows.first().map(|r| r.get::<i64, _>("total_count")).unwrap_or(0);

    let files = rows
        .into_iter()
        .map(|r| TrashFileResponse {
            file_id: r.get::<Uuid, _>("id"),
            encrypted_name: r.get::<String, _>("encrypted_name"),
            size_bytes: r.get::<i64, _>("size_bytes") as u64,
            total_chunks: r.get::<i32, _>("total_chunks") as u32,
            content_hash: r.get::<String, _>("content_hash"),
            created_at: r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            deleted_at: r.get::<chrono::DateTime<chrono::Utc>, _>("deleted_at"),
        })
        .collect();

    Ok(Json(TrashListResponse { files, total, limit, offset }))
}

/// `POST /api/v1/files/:file_id/restore` — restore a file from trash.
///
/// Only succeeds if the file is currently soft-deleted and was deleted within
/// the past 30 days. After the retention window the file is no longer
/// restorable and will be hard-deleted by the cleanup job.
pub async fn restore_file(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(file_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;

    let affected = sqlx::query(
        r#"
        UPDATE files
        SET deleted_at = NULL
        WHERE id = $1
          AND user_id = $2
          AND deleted_at IS NOT NULL
          AND deleted_at > NOW() - INTERVAL '30 days'
        "#,
    )
    .bind(file_id)
    .bind(user_id)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    .rows_affected();

    if affected == 0 {
        // Could be: not found, not in trash, or outside retention window.
        return Err(AppError::NotFound(format!(
            "file {file_id} is not in trash or the 30-day restore window has expired"
        )));
    }

    tracing::info!(file_id = %file_id, user_id = %user_id, "File restored from trash");
    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /api/v1/files/:file_id` — update the encrypted name of a file.
///
/// The server cannot read the new name — it stores only the encrypted bytes.
/// The client is responsible for encrypting the new name with the same key
/// envelope that was provided during upload.
pub async fn rename_file(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(file_id): Path<Uuid>,
    Json(req): Json<RenameFileRequest>,
) -> Result<impl IntoResponse> {
    if req.encrypted_name.trim().is_empty() {
        return Err(AppError::BadRequest("encrypted_name must not be empty".into()));
    }

    let affected = sqlx::query(
        r#"
        UPDATE files
        SET encrypted_name = $1
        WHERE id = $2 AND user_id = $3 AND deleted_at IS NULL
        "#,
    )
    .bind(&req.encrypted_name)
    .bind(file_id)
    .bind(claims.sub)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound(format!("file {file_id}")));
    }

    tracing::info!(file_id = %file_id, user_id = %claims.sub, "File renamed");
    Ok(StatusCode::NO_CONTENT)
}
