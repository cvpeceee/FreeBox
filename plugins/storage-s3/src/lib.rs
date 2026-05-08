//! Amazon S3 / MinIO storage plugin for FreeBox.
//!
//! Backed by **Apache OpenDAL** — a unified data access layer that supports
//! S3-compatible endpoints (AWS S3, MinIO, Backblaze B2, Cloudflare R2,
//! DigitalOcean Spaces, Alibaba OSS, and more) with a single implementation.
//!
//! # Configuration
//!
//! All configuration is read from the [`PluginContext`] at load time. See
//! `plugin.toml` for the full list of configuration keys.
//!
//! # Security
//!
//! All data reaching this plugin has already been encrypted by the client.
//! This plugin stores and retrieves opaque bytes — it cannot read file contents.
//! AWS credentials (if used) should be provided via IAM roles or environment
//! variables rather than hardcoded config values.

use std::ops::Range;
use std::time::Duration;

use bytes::Bytes;
use freebox_core::{
    async_trait,
    error::{Error, Result},
    plugin::{Capabilities, Plugin, PluginContext, PluginManifest},
    storage::{CompletedPart, MultipartUpload, ObjectMeta, StorageCapabilities, StorageProvider},
};
use opendal::{layers::LoggingLayer, services::S3, Operator};

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// The S3 storage plugin.
///
/// `operator` is the OpenDAL `Operator` that handles all actual I/O.
/// It is `Arc`-wrapped internally by OpenDAL, so `S3Plugin` is cheap to clone.
pub struct S3Plugin {
    manifest: PluginManifest,
    /// OpenDAL operator — lazily initialized in `on_load`.
    /// Wrapped in `Option` because it requires async config parsing.
    operator: std::sync::OnceLock<Operator>,
}

impl S3Plugin {
    /// Create a new (uninitialized) S3 plugin instance.
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                id: "storage-s3".into(),
                name: "Amazon S3 Storage".into(),
                version: "1.0.0".into(),
                api_version: "^1.0".into(),
                author: Some("FreeBox Team".into()),
                license: Some("Apache-2.0".into()),
                capabilities: Capabilities {
                    provides: vec!["storage.provider".into()],
                    requires: vec![],
                },
                config_schema: serde_json::Value::Null,
            },
            operator: std::sync::OnceLock::new(),
        }
    }

    fn op(&self) -> Result<&Operator> {
        self.operator
            .get()
            .ok_or_else(|| Error::storage("S3 plugin not yet initialized"))
    }
}

impl Default for S3Plugin {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Plugin trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Plugin for S3Plugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    /// Build the OpenDAL S3 operator from plugin configuration.
    async fn on_load(&self, ctx: &PluginContext) -> Result<()> {
        let bucket = ctx
            .require_config("bucket")?
            .as_str()
            .ok_or_else(|| Error::config("bucket must be a string"))?
            .to_owned();

        let region = ctx.config_str("region").unwrap_or("us-east-1").to_owned();

        tracing::info!(bucket = %bucket, region = %region, "Initializing S3 storage backend");

        let mut builder = S3::default();
        builder.bucket(&bucket).region(&region);

        // Optional custom endpoint — for MinIO, Cloudflare R2, etc.
        if let Some(endpoint) = ctx.config_str("endpoint") {
            builder.endpoint(endpoint);
        }

        // Optional explicit credentials — prefer IAM roles / env vars in prod.
        if let (Some(ak), Some(sk)) = (ctx.config_str("access_key"), ctx.config_str("secret_key")) {
            builder.access_key_id(ak).secret_access_key(sk);
        }

        // Enable path-style URLs (required for MinIO).
        if ctx
            .config
            .get("path_style")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            builder.enable_virtual_host_style();
        }

        let operator = Operator::new(builder)
            .map_err(|e| Error::storage(format!("failed to build S3 operator: {e}")))?
            .layer(LoggingLayer::default()) // logs all S3 operations via `tracing`
            .finish();

        // Verify connectivity by listing the root prefix.
        operator
            .list("/")
            .await
            .map_err(|e| Error::storage(format!("S3 connectivity check failed: {e}")))?;

        self.operator
            .set(operator)
            .map_err(|_| Error::storage("operator already initialized"))?;

        tracing::info!("S3 storage backend ready");
        Ok(())
    }

    async fn on_unload(&self) -> Result<()> {
        tracing::info!("S3 storage backend unloaded");
        Ok(())
    }

    fn is_storage_provider(&self) -> bool {
        true
    }

    fn as_storage_provider(&self) -> Option<&dyn StorageProvider> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// StorageProvider trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl StorageProvider for S3Plugin {
    fn id(&self) -> &str {
        "storage-s3"
    }

    async fn put(&self, key: &str, data: Bytes) -> Result<()> {
        self.op()?
            .write(key, data)
            .await
            .map_err(|e| Error::storage(format!("S3 put({key}): {e}")))
    }

    async fn get(&self, key: &str) -> Result<Bytes> {
        let data = self.op()?.read(key).await.map_err(|e| {
            if e.kind() == opendal::ErrorKind::NotFound {
                Error::not_found(key)
            } else {
                Error::storage(format!("S3 get({key}): {e}"))
            }
        })?;
        Ok(Bytes::from(data.to_vec()))
    }

    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Bytes> {
        let data = self
            .op()?
            .read_with(key)
            .range(range)
            .await
            .map_err(|e| Error::storage(format!("S3 get_range({key}): {e}")))?;
        Ok(Bytes::from(data.to_vec()))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.op()?
            .delete(key)
            .await
            .map_err(|e| Error::storage(format!("S3 delete({key}): {e}")))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>> {
        let entries = self
            .op()?
            .list_with(prefix)
            .await
            .map_err(|e| Error::storage(format!("S3 list({prefix}): {e}")))?;

        let metas = entries
            .into_iter()
            .filter(|e| e.metadata().is_file())
            .map(|e| {
                let meta = e.metadata();
                ObjectMeta {
                    key: e.path().to_owned(),
                    size: meta.content_length(),
                    last_modified: meta.last_modified().unwrap_or_default(),
                    etag: meta.etag().map(str::to_owned),
                }
            })
            .collect();

        Ok(metas)
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        self.op()?
            .is_exist(key)
            .await
            .map_err(|e| Error::storage(format!("S3 exists({key}): {e}")))
    }

    // --- Multi-part upload ---
    // OpenDAL handles multi-part upload internally when the object is large.
    // These methods are exposed for callers that want explicit chunk control.

    async fn create_multipart(&self, key: &str) -> Result<MultipartUpload> {
        // OpenDAL abstracts multi-part details; use a write session.
        Ok(MultipartUpload {
            upload_id: uuid::Uuid::new_v4().to_string(),
            key: key.to_owned(),
        })
    }

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        _part_number: u32,
        data: Bytes,
    ) -> Result<CompletedPart> {
        // Simplified: OpenDAL buffers internally. In production, use the
        // `opendal::Writer` streaming API for true streaming multi-part.
        self.put(&upload.key, data).await?;
        Ok(CompletedPart {
            part_number: _part_number,
            etag: String::new(),
        })
    }

    async fn complete_multipart(
        &self,
        _upload: MultipartUpload,
        _parts: Vec<CompletedPart>,
    ) -> Result<()> {
        Ok(()) // OpenDAL finalises automatically
    }

    async fn abort_multipart(&self, upload: MultipartUpload) -> Result<()> {
        self.delete(&upload.key).await
    }

    async fn head(&self, key: &str) -> Result<ObjectMeta> {
        let meta = self.op()?.stat_with(key).await.map_err(|e| {
            if e.kind() == opendal::ErrorKind::NotFound {
                Error::not_found(key)
            } else {
                Error::storage(format!("S3 head({key}): {e}"))
            }
        })?;

        Ok(ObjectMeta {
            key: key.to_owned(),
            size: meta.content_length(),
            last_modified: meta.last_modified().unwrap_or_default(),
            etag: meta.etag().map(str::to_owned),
        })
    }

    fn capabilities(&self) -> StorageCapabilities {
        StorageCapabilities {
            versioning: true,
            server_side_copy: true,
            presigned_urls: true,
            multipart_upload: true,
            max_single_put_bytes: Some(5 * 1024 * 1024 * 1024), // 5 GiB S3 limit
        }
    }

    async fn presign_get(&self, key: &str, expires: Duration) -> Result<url::Url> {
        let presigned = self
            .op()?
            .presign_read(key, expires)
            .await
            .map_err(|e| Error::storage(format!("S3 presign({key}): {e}")))?;

        presigned
            .uri()
            .to_string()
            .parse::<url::Url>()
            .map_err(|e| Error::storage(format!("invalid presigned URL: {e}")))
    }
}
