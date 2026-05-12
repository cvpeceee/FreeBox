use super::*;
use chrono::Utc;

#[test]
fn storage_capabilities_default_is_all_false() {
    let caps = StorageCapabilities::default();
    assert!(!caps.versioning);
    assert!(!caps.server_side_copy);
    assert!(!caps.presigned_urls);
    assert!(!caps.multipart_upload);
    assert!(caps.max_single_put_bytes.is_none());
}

#[test]
fn object_meta_serialization_round_trip() {
    let meta = ObjectMeta {
        key: "chunks/abc123/00000000".to_owned(),
        size: 4096,
        last_modified: Utc::now(),
        etag: Some("d41d8cd98f00b204e9800998ecf8427e".to_owned()),
    };

    let json = serde_json::to_string(&meta).expect("serialize must succeed");
    let decoded: ObjectMeta = serde_json::from_str(&json).expect("deserialize must succeed");

    assert_eq!(decoded.key, meta.key);
    assert_eq!(decoded.size, meta.size);
    assert_eq!(decoded.etag, meta.etag);
}

#[test]
fn object_meta_without_etag_round_trips() {
    let meta = ObjectMeta {
        key: "keys/file-id.envelope".to_owned(),
        size: 256,
        last_modified: Utc::now(),
        etag: None,
    };

    let json = serde_json::to_string(&meta).unwrap();
    let decoded: ObjectMeta = serde_json::from_str(&json).unwrap();

    assert!(decoded.etag.is_none());
}

#[test]
fn multipart_upload_fields_are_accessible() {
    let upload = MultipartUpload {
        upload_id: "upload-abc-123".to_owned(),
        key: "chunks/test/00000000".to_owned(),
    };
    assert_eq!(upload.upload_id, "upload-abc-123");
    assert_eq!(upload.key, "chunks/test/00000000");
}

#[test]
fn completed_part_fields_are_accessible() {
    let part = CompletedPart {
        part_number: 3,
        etag: "etag-value".to_owned(),
    };
    assert_eq!(part.part_number, 3);
    assert_eq!(part.etag, "etag-value");
}

#[test]
fn storage_capabilities_can_be_fully_enabled() {
    let caps = StorageCapabilities {
        versioning: true,
        server_side_copy: true,
        presigned_urls: true,
        multipart_upload: true,
        max_single_put_bytes: Some(5 * 1024 * 1024 * 1024), // 5 GB
    };
    assert!(caps.versioning);
    assert!(caps.server_side_copy);
    assert!(caps.presigned_urls);
    assert!(caps.multipart_upload);
    assert_eq!(caps.max_single_put_bytes, Some(5 * 1024 * 1024 * 1024));
}
