//! Storage provider abstraction.
//!
//! All storage backends — local disk, Amazon S3, Google Cloud Storage, IPFS,
//! or anything else — implement the [`StorageProvider`] trait. The kernel
//! and plugins program against this abstraction; they never reference a
//! specific backend directly.
//!
//! **Important:** All data reaching a [`StorageProvider`] is already
//! encrypted by the client. The provider only ever sees opaque bytes.
//! This "encrypted at the edge" design means any backend is safe to use,
//! even a third-party cloud service.

use std::ops::Range;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Result;

// ---------------------------------------------------------------------------
// Object metadata
// ---------------------------------------------------------------------------

/// Metadata describing a stored object (file chunk, key envelope, etc.).
///
/// The `key` is an opaque storage path chosen by the kernel; plugins must
/// treat it as an arbitrary string, not a file path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMeta {
    /// Storage key (e.g. `"chunks/uuid/00000001"`).
    pub key: String,

    /// Size of the stored object in bytes.
    pub size: u64,

    /// Server-side last-modified timestamp (UTC).
    pub last_modified: DateTime<Utc>,

    /// Content hash as returned by the backend (may be empty if unsupported).
    pub etag: Option<String>,
}

// ---------------------------------------------------------------------------
// Multi-part upload types
// ---------------------------------------------------------------------------

/// Handle returned by [`StorageProvider::create_multipart`].
/// Passed to subsequent [`StorageProvider::upload_part`] calls.
#[derive(Debug, Clone)]
pub struct MultipartUpload {
    pub upload_id: String,
    pub key: String,
}

/// A completed part of a multi-part upload.
#[derive(Debug, Clone)]
pub struct CompletedPart {
    pub part_number: u32,
    pub etag: String,
}

// ---------------------------------------------------------------------------
// Capability flags
// ---------------------------------------------------------------------------

/// Optional capabilities that a storage backend may or may not support.
///
/// The kernel queries these before attempting advanced operations so it can
/// fall back gracefully (e.g. emulating versioning in the database when the
/// backend does not support it natively).
#[derive(Debug, Clone, Default)]
pub struct StorageCapabilities {
    /// Backend supports object versioning (e.g. S3 versioned buckets).
    pub versioning: bool,

    /// Backend supports server-side copy (avoids re-uploading for renames).
    pub server_side_copy: bool,

    /// Backend supports pre-signed download URLs.
    pub presigned_urls: bool,

    /// Backend supports multi-part uploads (required for files > 100 MB).
    pub multipart_upload: bool,

    /// Maximum single-put object size in bytes (`None` = unlimited).
    pub max_single_put_bytes: Option<u64>,
}

// ---------------------------------------------------------------------------
// StorageProvider trait
// ---------------------------------------------------------------------------

/// The unified storage backend abstraction.
///
/// # Thread safety
///
/// Implementations **must** be `Send + Sync`. The kernel may call methods
/// from multiple async tasks concurrently. Use `Arc<Mutex<_>>` internally
/// only when truly necessary — prefer lock-free structures.
///
/// # Error handling
///
/// All methods return [`crate::error::Result`]. Map backend-specific errors
/// to [`crate::error::Error::Storage`] with a descriptive message.
#[async_trait::async_trait]
pub trait StorageProvider: Send + Sync {
    /// A stable identifier for this provider instance (e.g. `"s3-us-east-1"`).
    fn id(&self) -> &str;

    // --- Basic CRUD ---

    /// Store `data` at `key`. Overwrites silently if the key already exists.
    ///
    /// For large objects use [`create_multipart`] / [`upload_part`] instead.
    async fn put(&self, key: &str, data: Bytes) -> Result<()>;

    /// Retrieve the object at `key`.
    ///
    /// Returns [`Error::NotFound`] if the key does not exist.
    async fn get(&self, key: &str) -> Result<Bytes>;

    /// Retrieve a byte range of the object at `key` (for chunked downloads).
    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Bytes>;

    /// Delete the object at `key`. No-op if the key does not exist.
    async fn delete(&self, key: &str) -> Result<()>;

    /// List objects whose key starts with `prefix`.
    ///
    /// Results are unordered. Callers must not assume any ordering.
    async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>>;

    /// Check whether `key` exists without downloading the object.
    async fn exists(&self, key: &str) -> Result<bool>;

    // --- Multi-part upload (for files > 5 MB) ---

    /// Initiate a multi-part upload. Returns an upload handle.
    ///
    /// Only available when [`StorageCapabilities::multipart_upload`] is `true`.
    async fn create_multipart(&self, key: &str) -> Result<MultipartUpload>;

    /// Upload a single part. Parts are numbered from 1.
    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: u32,
        data: Bytes,
    ) -> Result<CompletedPart>;

    /// Commit all uploaded parts and finalise the object.
    async fn complete_multipart(
        &self,
        upload: MultipartUpload,
        parts: Vec<CompletedPart>,
    ) -> Result<()>;

    /// Abort an in-progress multi-part upload, releasing any partial data.
    async fn abort_multipart(&self, upload: MultipartUpload) -> Result<()>;

    // --- Metadata & capabilities ---

    /// Return the metadata for `key` without downloading the object body.
    async fn head(&self, key: &str) -> Result<ObjectMeta>;

    /// Return the capability flags for this backend.
    fn capabilities(&self) -> StorageCapabilities;

    /// Generate a pre-signed GET URL valid for `expires` duration.
    ///
    /// Only available when [`StorageCapabilities::presigned_urls`] is `true`.
    async fn presign_get(&self, key: &str, expires: std::time::Duration) -> Result<url::Url>;
}
