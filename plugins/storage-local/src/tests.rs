use bytes::Bytes;
use freebox_core::{
    error::Error,
    event::NoopEventBus,
    plugin::PluginContext,
    storage::StorageProvider,
};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

use super::LocalPlugin;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `PluginContext` pointing the plugin at `root`.
fn make_context(root: &std::path::Path) -> PluginContext {
    let mut config = HashMap::new();
    config.insert(
        "root".to_owned(),
        serde_json::Value::String(root.to_string_lossy().into_owned()),
    );
    PluginContext {
        event_bus: Arc::new(NoopEventBus),
        config,
        instance_id: Uuid::new_v4(),
    }
}

/// Create a unique temporary directory per test.
fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("freebox-local-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Remove the test directory.
fn cleanup(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn on_load_creates_root_directory() {
    let root = temp_dir().join("subdir-that-does-not-exist");
    let ctx = make_context(&root);
    let plugin = LocalPlugin::new();

    plugin.on_load(&ctx).await.expect("on_load must succeed");

    assert!(root.exists(), "on_load must create the root directory");
    cleanup(&root);
}

#[tokio::test]
async fn on_load_fails_when_root_config_missing() {
    let ctx = PluginContext {
        event_bus: Arc::new(NoopEventBus),
        config: HashMap::new(), // no "root" key
        instance_id: Uuid::new_v4(),
    };
    let plugin = LocalPlugin::new();

    assert!(
        plugin.on_load(&ctx).await.is_err(),
        "missing root config must be an error"
    );
}

#[tokio::test]
async fn put_and_get_round_trip() {
    let root = temp_dir();
    let plugin = LocalPlugin::new();
    plugin.on_load(&make_context(&root)).await.unwrap();

    let data = Bytes::from_static(b"encrypted-chunk-data");
    plugin.put("chunks/abc/00000000", data.clone()).await.unwrap();

    let retrieved = plugin.get("chunks/abc/00000000").await.unwrap();
    assert_eq!(retrieved, data);

    cleanup(&root);
}

#[tokio::test]
async fn get_returns_not_found_for_missing_key() {
    let root = temp_dir();
    let plugin = LocalPlugin::new();
    plugin.on_load(&make_context(&root)).await.unwrap();

    let err = plugin.get("nonexistent/key").await.unwrap_err();
    assert!(
        matches!(err, Error::NotFound(_)),
        "missing key must return Error::NotFound, got: {err:?}"
    );

    cleanup(&root);
}

#[tokio::test]
async fn delete_removes_object() {
    let root = temp_dir();
    let plugin = LocalPlugin::new();
    plugin.on_load(&make_context(&root)).await.unwrap();

    plugin
        .put("chunks/del/00000000", Bytes::from_static(b"data"))
        .await
        .unwrap();

    plugin.delete("chunks/del/00000000").await.unwrap();

    let exists = plugin.exists("chunks/del/00000000").await.unwrap();
    assert!(!exists, "deleted key must not exist");

    cleanup(&root);
}

#[tokio::test]
async fn exists_returns_false_for_missing_key() {
    let root = temp_dir();
    let plugin = LocalPlugin::new();
    plugin.on_load(&make_context(&root)).await.unwrap();

    assert!(!plugin.exists("no/such/key").await.unwrap());

    cleanup(&root);
}

#[tokio::test]
async fn list_returns_written_objects() {
    let root = temp_dir();
    let plugin = LocalPlugin::new();
    plugin.on_load(&make_context(&root)).await.unwrap();

    plugin
        .put("chunks/lst/00000000", Bytes::from_static(b"a"))
        .await
        .unwrap();
    plugin
        .put("chunks/lst/00000001", Bytes::from_static(b"b"))
        .await
        .unwrap();

    let entries = plugin.list("chunks/lst/").await.unwrap();
    assert_eq!(entries.len(), 2, "list must return exactly two objects");

    cleanup(&root);
}

#[tokio::test]
async fn multipart_upload_reassembles_parts() {
    let root = temp_dir();
    let plugin = LocalPlugin::new();
    plugin.on_load(&make_context(&root)).await.unwrap();

    let upload = plugin.create_multipart("chunks/mp/file").await.unwrap();

    let part1 = plugin
        .upload_part(&upload, 1, Bytes::from_static(b"Hello, "))
        .await
        .unwrap();
    let part2 = plugin
        .upload_part(&upload, 2, Bytes::from_static(b"world!"))
        .await
        .unwrap();

    plugin
        .complete_multipart(upload, vec![part1, part2])
        .await
        .unwrap();

    let result = plugin.get("chunks/mp/file").await.unwrap();
    assert_eq!(&result[..], b"Hello, world!");

    cleanup(&root);
}

#[tokio::test]
async fn head_returns_correct_size() {
    let root = temp_dir();
    let plugin = LocalPlugin::new();
    plugin.on_load(&make_context(&root)).await.unwrap();

    let data = Bytes::from(vec![0u8; 1234]);
    plugin.put("chunks/head/obj", data).await.unwrap();

    let meta = plugin.head("chunks/head/obj").await.unwrap();
    assert_eq!(meta.size, 1234);
    assert_eq!(meta.key, "chunks/head/obj");

    cleanup(&root);
}

#[test]
fn capabilities_reports_multipart_supported() {
    let plugin = LocalPlugin::new();
    let caps = plugin.capabilities();
    assert!(caps.multipart_upload, "local backend must support multipart");
    assert!(!caps.presigned_urls, "local backend must not support presigned URLs");
    assert!(!caps.versioning);
}

#[test]
fn provider_id_is_stable() {
    let plugin = LocalPlugin::new();
    assert_eq!(plugin.id(), "storage-local");
}
