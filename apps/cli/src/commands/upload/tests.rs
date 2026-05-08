use std::path::Path;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{post, put},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use freebox_crypto::encryption::{decrypt_file, FileKey};
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::session::Session;

use super::{prepare_upload, remote_name, upload_files};

#[test]
fn remote_name_uses_file_name_for_root_destination() {
    let name = remote_name("remote://", Path::new("notes.txt")).unwrap();

    assert_eq!(name, "notes.txt");
}

#[test]
fn remote_name_prefixes_non_root_destination() {
    let name = remote_name("remote://docs", Path::new("notes.txt")).unwrap();

    assert_eq!(name, "remote://docs/notes.txt");
}

#[test]
fn prepare_upload_encrypts_plaintext_chunks() {
    let prepared = prepare_upload("notes.txt", b"hello freebox").unwrap();
    let key_bytes: [u8; 32] = STANDARD
        .decode(&prepared.encrypted_key_envelope)
        .unwrap()
        .try_into()
        .unwrap();
    let key = FileKey::from_bytes(key_bytes);

    let plaintext = decrypt_file(&key, &prepared.chunks).unwrap();

    assert_eq!(plaintext, b"hello freebox");
    assert_eq!(prepared.total_chunks, 1);
    assert_eq!(prepared.size_bytes, 13);
    assert_eq!(
        prepared.content_hash,
        blake3::hash(b"hello freebox").to_hex().to_string()
    );
}

#[test]
fn prepare_upload_represents_empty_file_as_one_chunk() {
    let prepared = prepare_upload("empty.txt", b"").unwrap();

    assert_eq!(prepared.total_chunks, 1);
    assert_eq!(prepared.size_bytes, 0);
}

#[derive(Default)]
struct UploadMockState {
    init_requests: Vec<serde_json::Value>,
    chunks: Vec<(u32, Vec<u8>)>,
    completed: bool,
}

#[tokio::test]
async fn upload_files_sends_init_chunks_and_complete_over_http() {
    let state = Arc::new(Mutex::new(UploadMockState::default()));
    let server = start_upload_mock(state.clone()).await;
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("hello.txt");
    tokio::fs::write(&file_path, b"hello integration")
        .await
        .unwrap();

    let uploaded = upload_files(
        vec![file_path.clone()],
        "remote://docs".into(),
        8,
        &server,
        &test_session(),
    )
    .await
    .unwrap();

    assert_eq!(uploaded.len(), 1);
    assert_eq!(
        uploaded[0].file_id,
        Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap()
    );

    let state = state.lock().await;
    assert_eq!(state.init_requests.len(), 1);
    assert_eq!(state.init_requests[0]["total_chunks"], 1);
    assert_eq!(state.chunks.len(), 1);
    assert_eq!(state.chunks[0].0, 0);
    assert!(
        serde_json::from_slice::<freebox_crypto::encryption::ChunkCiphertext>(&state.chunks[0].1)
            .is_ok()
    );
    assert!(state.completed);
}

async fn start_upload_mock(state: Arc<Mutex<UploadMockState>>) -> String {
    async fn init(
        State(state): State<Arc<Mutex<UploadMockState>>>,
        Json(body): Json<serde_json::Value>,
    ) -> impl IntoResponse {
        state.lock().await.init_requests.push(body);
        Json(json!({
            "upload_id": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        }))
    }

    async fn chunk(
        State(state): State<Arc<Mutex<UploadMockState>>>,
        AxumPath(_upload_id): AxumPath<String>,
        headers: HeaderMap,
        body: Bytes,
    ) -> impl IntoResponse {
        let index = headers
            .get("x-chunk-index")
            .unwrap()
            .to_str()
            .unwrap()
            .parse::<u32>()
            .unwrap();
        state.lock().await.chunks.push((index, body.to_vec()));
        StatusCode::NO_CONTENT
    }

    async fn complete(State(state): State<Arc<Mutex<UploadMockState>>>) -> impl IntoResponse {
        state.lock().await.completed = true;
        (
            StatusCode::CREATED,
            Json(json!({
                "file_id": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
            })),
        )
    }

    let app = Router::new()
        .route("/api/v1/files/upload/init", post(init))
        .route("/api/v1/files/upload/:upload_id", put(chunk))
        .route("/api/v1/files/upload/:upload_id/complete", post(complete))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn test_session() -> Session {
    Session {
        access_token: "access".into(),
        refresh_token: "refresh".into(),
        server: "http://localhost:8080".into(),
        username: "alice".into(),
        user_id: Uuid::new_v4(),
    }
}
