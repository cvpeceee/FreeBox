use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use tempfile::tempdir;
use uuid::Uuid;

use crate::{commands::upload::prepare_upload, session::Session};

use super::{default_file_name, download_file, file_key_from_envelope, resolve_output_path};

#[test]
fn default_file_name_uses_last_remote_segment() {
    let encoded = STANDARD.encode("remote://docs/report.pdf");

    let file_name = default_file_name(&encoded).unwrap();

    assert_eq!(file_name, "report.pdf");
}

#[test]
fn resolve_output_path_uses_explicit_file_path() {
    let encoded = STANDARD.encode("remote://docs/report.pdf");
    let path = resolve_output_path(Some(PathBuf::from("custom.pdf")), &encoded).unwrap();

    assert_eq!(path, PathBuf::from("custom.pdf"));
}

#[test]
fn resolve_output_path_appends_default_name_to_directory() {
    let dir = tempdir().unwrap();
    let encoded = STANDARD.encode("remote://docs/report.pdf");

    let path = resolve_output_path(Some(dir.path().to_path_buf()), &encoded).unwrap();

    assert_eq!(path, dir.path().join("report.pdf"));
}

#[test]
fn file_key_from_envelope_accepts_32_byte_key() {
    let envelope = STANDARD.encode([7u8; 32]);

    let key = file_key_from_envelope(&envelope).unwrap();

    assert_eq!(key.as_bytes(), &[7u8; 32]);
}

#[test]
fn file_key_from_envelope_rejects_wrong_length() {
    let envelope = STANDARD.encode([7u8; 31]);

    assert!(file_key_from_envelope(&envelope).is_err());
}

struct DownloadMockState {
    encrypted_name: String,
    encrypted_key_envelope: String,
    chunks: Vec<Vec<u8>>,
}

#[tokio::test]
async fn download_file_fetches_chunks_and_writes_decrypted_plaintext() {
    let prepared = prepare_upload("remote://docs/report.txt", b"hello download").unwrap();
    let state = Arc::new(DownloadMockState {
        encrypted_name: prepared.encrypted_name,
        encrypted_key_envelope: prepared.encrypted_key_envelope,
        chunks: prepared
            .chunks
            .iter()
            .map(|chunk| serde_json::to_vec(chunk).unwrap())
            .collect(),
    });
    let server = start_download_mock(state).await;
    let dir = tempdir().unwrap();
    let output = dir.path().join("downloaded.txt");
    let file_id = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();

    let written = download_file(file_id, Some(output.clone()), &server, &test_session())
        .await
        .unwrap();

    assert_eq!(written, output);
    assert_eq!(tokio::fs::read(&output).await.unwrap(), b"hello download");
}

async fn start_download_mock(state: Arc<DownloadMockState>) -> String {
    async fn meta(State(state): State<Arc<DownloadMockState>>) -> impl IntoResponse {
        Json(serde_json::json!({
            "encrypted_name": state.encrypted_name,
            "total_chunks": state.chunks.len(),
            "encrypted_key_envelope": state.encrypted_key_envelope
        }))
    }

    async fn chunk(
        State(state): State<Arc<DownloadMockState>>,
        AxumPath((_file_id, chunk_index)): AxumPath<(String, usize)>,
    ) -> impl IntoResponse {
        match state.chunks.get(chunk_index) {
            Some(body) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/octet-stream")],
                Bytes::from(body.clone()),
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    let app = Router::new()
        .route("/api/v1/files/:file_id", get(meta))
        .route("/api/v1/files/:file_id/chunk/:chunk_index", get(chunk))
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
