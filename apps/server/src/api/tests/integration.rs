use std::{ops::Range, sync::Arc, time::Duration};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use bytes::Bytes;
use freebox_core::{
    async_trait,
    error::Error,
    storage::{CompletedPart, MultipartUpload, ObjectMeta, StorageCapabilities, StorageProvider},
};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    api::{self, auth::Claims, files::MAX_UPLOAD_CHUNK_BODY_BYTES},
    config::{Config, OAuthConfig},
    state::AppState,
};

const TEST_JWT_SECRET: &str = "test_secret_at_least_32_bytes_long_for_hs256";

struct NoopStorage;

#[async_trait]
impl StorageProvider for NoopStorage {
    fn id(&self) -> &str {
        "noop-storage"
    }

    async fn put(&self, _key: &str, _data: Bytes) -> freebox_core::Result<()> {
        Err(Error::storage("noop storage"))
    }

    async fn get(&self, _key: &str) -> freebox_core::Result<Bytes> {
        Err(Error::storage("noop storage"))
    }

    async fn get_range(&self, _key: &str, _range: Range<u64>) -> freebox_core::Result<Bytes> {
        Err(Error::storage("noop storage"))
    }

    async fn delete(&self, _key: &str) -> freebox_core::Result<()> {
        Err(Error::storage("noop storage"))
    }

    async fn list(&self, _prefix: &str) -> freebox_core::Result<Vec<ObjectMeta>> {
        Err(Error::storage("noop storage"))
    }

    async fn exists(&self, _key: &str) -> freebox_core::Result<bool> {
        Ok(false)
    }

    async fn create_multipart(&self, _key: &str) -> freebox_core::Result<MultipartUpload> {
        Err(Error::storage("noop storage"))
    }

    async fn upload_part(
        &self,
        _upload: &MultipartUpload,
        _part_number: u32,
        _data: Bytes,
    ) -> freebox_core::Result<CompletedPart> {
        Err(Error::storage("noop storage"))
    }

    async fn complete_multipart(
        &self,
        _upload: MultipartUpload,
        _parts: Vec<CompletedPart>,
    ) -> freebox_core::Result<()> {
        Err(Error::storage("noop storage"))
    }

    async fn abort_multipart(&self, _upload: MultipartUpload) -> freebox_core::Result<()> {
        Err(Error::storage("noop storage"))
    }

    async fn head(&self, _key: &str) -> freebox_core::Result<ObjectMeta> {
        Err(Error::storage("noop storage"))
    }

    fn capabilities(&self) -> StorageCapabilities {
        StorageCapabilities::default()
    }

    async fn presign_get(&self, _key: &str, _expires: Duration) -> freebox_core::Result<url::Url> {
        Err(Error::storage("noop storage"))
    }
}

fn build_test_app(rate_limit_requests: u32, admin_user_ids: Vec<Uuid>) -> axum::Router {
    let cfg = Config {
        host: "127.0.0.1".into(),
        port: 8080,
        database_url: "postgres://freebox:freebox_dev@127.0.0.1:5432/freebox".into(),
        db_pool_size: 1,
        redis_url: "redis://127.0.0.1:6379".into(),
        rate_limit_requests,
        rate_limit_window_secs: 60,
        oauth_reactivation_max_age_days: 30,
        admin_user_ids,
        jwt_secret: TEST_JWT_SECRET.into(),
        jwt_access_ttl_secs: 900,
        jwt_refresh_ttl_secs: 2_592_000,
        storage_provider: "local".into(),
        storage_local_root: "./tmp/freebox-server-test-storage".into(),
        storage_s3_bucket: String::new(),
        storage_s3_region: "auto".into(),
        storage_s3_endpoint: String::new(),
        storage_s3_access_key: String::new(),
        storage_s3_secret_key: String::new(),
        argon2_memory_kib: 65_536,
        argon2_iterations: 3,
        argon2_parallelism: 4,
        oauth: OAuthConfig::default(),
    };

    let db = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy(&cfg.database_url)
        .expect("test DATABASE_URL must be a valid Postgres URL");

    let storage: Arc<dyn StorageProvider> = Arc::new(NoopStorage);

    api::router(AppState::new(cfg, db, storage))
}

fn issue_token(user_id: Uuid, username: &str) -> String {
    let claims = Claims::new(user_id, username, 3_600);
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(TEST_JWT_SECRET.as_bytes()),
    )
    .expect("failed to issue test JWT")
}

#[tokio::test]
async fn health_route_is_rate_limited_after_configured_capacity() {
    let app = build_test_app(2, Vec::new());

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);

    let third = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(third.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn audit_and_bucket_routes_require_authentication() {
    let app = build_test_app(600, Vec::new());

    let audit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/auth/audit-events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(audit.status(), StatusCode::UNAUTHORIZED);

    let admin_audit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/audit-events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_audit.status(), StatusCode::UNAUTHORIZED);

    let list_buckets = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/storage/buckets")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_buckets.status(), StatusCode::UNAUTHORIZED);

    let create_bucket = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/storage/buckets")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"name\":\"freebox-test\"}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_bucket.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_audit_events_forbids_non_admin_user() {
    let configured_admin = Uuid::new_v4();
    let requester = Uuid::new_v4();
    let token = issue_token(requester, "regular-user");

    let app = build_test_app(600, vec![configured_admin]);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/audit-events")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn upload_chunk_route_rejects_oversized_body() {
    let uploader = Uuid::new_v4();
    let token = issue_token(uploader, "uploader");

    let app = build_test_app(600, Vec::new());

    let oversized_body = vec![0u8; MAX_UPLOAD_CHUNK_BODY_BYTES + 1];
    let upload_id = Uuid::new_v4();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/files/upload/{upload_id}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("x-chunk-index", "0")
                .body(Body::from(oversized_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn register_route_rejects_invalid_prekey_bundle() {
    let app = build_test_app(600, Vec::new());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "username": "alice",
                        "email": "alice@example.com",
                        "password_hash": "client-side-password-hash",
                        "argon2_salt": "client-side-salt",
                        "prekey_bundle": {
                            "identity_key": [1, 2, 3],
                            "signed_prekey": {},
                            "one_time_prekeys": []
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Trash, restore, rename — new endpoint tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn trash_route_requires_authentication() {
    let app = build_test_app(600, Vec::new());

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/files/trash")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn restore_route_requires_authentication() {
    let app = build_test_app(600, Vec::new());
    let file_id = Uuid::new_v4();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/files/{file_id}/restore"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rename_route_requires_authentication() {
    let app = build_test_app(600, Vec::new());
    let file_id = Uuid::new_v4();

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v1/files/{file_id}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"encrypted_name\":\"newname\"}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rename_route_rejects_empty_encrypted_name() {
    let user_id = Uuid::new_v4();
    let token = issue_token(user_id, "alice");
    let app = build_test_app(600, Vec::new());
    let file_id = Uuid::new_v4();

    // Validation fires before any DB call, so NoopStorage/no-op DB is fine.
    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v1/files/{file_id}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"encrypted_name\":\"\"}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rename_route_rejects_whitespace_only_encrypted_name() {
    let user_id = Uuid::new_v4();
    let token = issue_token(user_id, "alice");
    let app = build_test_app(600, Vec::new());
    let file_id = Uuid::new_v4();

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v1/files/{file_id}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"encrypted_name\":\"   \"}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn list_files_route_requires_authentication() {
    let app = build_test_app(600, Vec::new());

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/files")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_files_with_pagination_params_requires_authentication() {
    let app = build_test_app(600, Vec::new());

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/files?limit=10&offset=20")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn replenish_one_time_route_rejects_duplicate_key_ids() {
    let user_id = Uuid::new_v4();
    let token = issue_token(user_id, "alice");
    let app = build_test_app(600, Vec::new());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/keys/one-time")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!([
                        { "id": 7, "public_key": vec![1u8; 32] },
                        { "id": 7, "public_key": vec![2u8; 32] }
                    ])
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
