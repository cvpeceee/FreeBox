//! Local filesystem storage plugin for FreeBox.
//!
//! Uses Apache OpenDAL's `Fs` service to read/write encrypted chunk files.
//! Suitable for:
//! - Local development (default backend in `docker-compose.dev.yml`)
//! - Self-hosted deployments on a single machine with attached storage
//! - NAS / network-mounted filesystems (CIFS, NFS)
//!
//! # Storage Layout
//!
//! ```text
//! {root}/
//!   chunks/
//!     {file_id}/
//!       00000000   ← encrypted chunk 0 (AES-256-GCM)
//!       00000001   ← encrypted chunk 1
//!       ...
//!   keys/
//!     {file_id}.envelope   ← sealed file key (Signal Protocol)
//! ```
//!
//! # Security
//!
//! All files stored here are already encrypted by the client. Even with full
//! filesystem access, an attacker cannot read file contents without the keys
//! stored exclusively on user devices.

use std::ops::Range;
use std::time::Duration;

use bytes::Bytes;
use freebox_core::{
    async_trait,
    error::{Error, Result},
    plugin::{Capabilities, Plugin, PluginContext, PluginManifest},
    storage::{CompletedPart, MultipartUpload, ObjectMeta, StorageCapabilities, StorageProvider},
};
use opendal::{services::Fs, Operator};

pub struct LocalPlugin {
    manifest: PluginManifest,
    operator: std::sync::OnceLock<Operator>,
}

impl LocalPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                id: "storage-local".into(),
                name: "Local Filesystem Storage".into(),
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
            .ok_or_else(|| Error::storage("Local plugin not yet initialized"))
    }
}

impl Default for LocalPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for LocalPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    async fn on_load(&self, ctx: &PluginContext) -> Result<()> {
        let root = ctx
            .require_config("root")?
            .as_str()
            .ok_or_else(|| Error::config("root must be a string"))?
            .to_owned();

        // Create the root directory if it does not exist.
        std::fs::create_dir_all(&root)
            .map_err(|e| Error::storage(format!("cannot create storage root {root}: {e}")))?;

        tracing::info!(root = %root, "Initializing local storage backend");

        let mut builder = Fs::default();
        builder.root(&root);

        let operator = Operator::new(builder)
            .map_err(|e| Error::storage(format!("failed to build Fs operator: {e}")))?
            .finish();

        self.operator
            .set(operator)
            .map_err(|_| Error::storage("already initialized"))?;

        tracing::info!("Local storage backend ready");
        Ok(())
    }

    async fn on_unload(&self) -> Result<()> {
        tracing::info!("Local storage backend unloaded");
        Ok(())
    }

    fn is_storage_provider(&self) -> bool {
        true
    }
    fn as_storage_provider(&self) -> Option<&dyn StorageProvider> {
        Some(self)
    }
}

#[async_trait]
impl StorageProvider for LocalPlugin {
    fn id(&self) -> &str {
        "storage-local"
    }

    async fn put(&self, key: &str, data: Bytes) -> Result<()> {
        self.op()?
            .write(key, data)
            .await
            .map_err(|e| Error::storage(format!("local put({key}): {e}")))
    }

    async fn get(&self, key: &str) -> Result<Bytes> {
        let data = self.op()?.read(key).await.map_err(|e| {
            if e.kind() == opendal::ErrorKind::NotFound {
                Error::not_found(key)
            } else {
                Error::storage(format!("local get({key}): {e}"))
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
            .map_err(|e| Error::storage(format!("local get_range({key}): {e}")))?;
        Ok(Bytes::from(data.to_vec()))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.op()?
            .delete(key)
            .await
            .map_err(|e| Error::storage(format!("local delete({key}): {e}")))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>> {
        let entries = self
            .op()?
            .list_with(prefix)
            .await
            .map_err(|e| Error::storage(format!("local list({prefix}): {e}")))?;

        Ok(entries
            .into_iter()
            .filter(|e| e.metadata().is_file())
            .map(|e| {
                let m = e.metadata();
                ObjectMeta {
                    key: e.path().to_owned(),
                    size: m.content_length(),
                    last_modified: m.last_modified().unwrap_or_default(),
                    etag: None,
                }
            })
            .collect())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        self.op()?
            .is_exist(key)
            .await
            .map_err(|e| Error::storage(format!("local exists({key}): {e}")))
    }

    async fn create_multipart(&self, key: &str) -> Result<MultipartUpload> {
        Ok(MultipartUpload {
            upload_id: uuid::Uuid::new_v4().to_string(),
            key: key.to_owned(),
        })
    }

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: u32,
        data: Bytes,
    ) -> Result<CompletedPart> {
        // Append part to a staging file.
        let staging_key = format!("{}.part.{:08}", upload.key, part_number);
        self.put(&staging_key, data).await?;
        Ok(CompletedPart {
            part_number,
            etag: String::new(),
        })
    }

    async fn complete_multipart(
        &self,
        upload: MultipartUpload,
        parts: Vec<CompletedPart>,
    ) -> Result<()> {
        // Concatenate staging parts into the final object.
        let mut final_data = Vec::new();
        let mut sorted = parts;
        sorted.sort_by_key(|p| p.part_number);
        for part in sorted {
            let staging = format!("{}.part.{:08}", upload.key, part.part_number);
            let chunk = self.get(&staging).await?;
            final_data.extend_from_slice(&chunk);
            self.delete(&staging).await.ok();
        }
        self.put(&upload.key, Bytes::from(final_data)).await
    }

    async fn abort_multipart(&self, upload: MultipartUpload) -> Result<()> {
        self.delete(&upload.key).await
    }

    async fn head(&self, key: &str) -> Result<ObjectMeta> {
        let meta = self.op()?.stat_with(key).await.map_err(|e| {
            if e.kind() == opendal::ErrorKind::NotFound {
                Error::not_found(key)
            } else {
                Error::storage(format!("local head({key}): {e}"))
            }
        })?;
        Ok(ObjectMeta {
            key: key.to_owned(),
            size: meta.content_length(),
            last_modified: meta.last_modified().unwrap_or_default(),
            etag: None,
        })
    }

    fn capabilities(&self) -> StorageCapabilities {
        StorageCapabilities {
            versioning: false,
            server_side_copy: false,
            presigned_urls: false,
            multipart_upload: true,
            max_single_put_bytes: None, // filesystem limit only
        }
    }

    async fn presign_get(&self, _key: &str, _expires: Duration) -> Result<url::Url> {
        Err(Error::storage(
            "local storage does not support presigned URLs",
        ))
    }
}

#[cfg(test)]
mod tests;
