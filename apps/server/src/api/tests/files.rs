use axum::http::{HeaderMap, HeaderValue};
use uuid::Uuid;

use crate::api::files::{
    chunk_storage_key, parse_chunk_index, validate_chunk_body_size, MAX_UPLOAD_CHUNK_BODY_BYTES,
};
use crate::error::AppError;

fn headers_with_chunk_index(value: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-chunk-index", HeaderValue::from_static(value));
    headers
}

#[test]
fn parse_chunk_index_accepts_valid_header() {
    let headers = headers_with_chunk_index("42");

    let index = parse_chunk_index(&headers).unwrap();

    assert_eq!(index, 42);
}

#[test]
fn parse_chunk_index_accepts_canonical_header_name() {
    let mut headers = HeaderMap::new();
    headers.insert("X-Chunk-Index", HeaderValue::from_static("7"));

    let index = parse_chunk_index(&headers).unwrap();

    assert_eq!(index, 7);
}

#[test]
fn parse_chunk_index_rejects_missing_header() {
    let err = parse_chunk_index(&HeaderMap::new()).unwrap_err();

    assert!(matches!(err, AppError::BadRequest(_)));
    assert!(err.to_string().contains("missing X-Chunk-Index"));
}

#[test]
fn parse_chunk_index_rejects_non_integer_header() {
    let headers = headers_with_chunk_index("not-a-number");

    let err = parse_chunk_index(&headers).unwrap_err();

    assert!(matches!(err, AppError::BadRequest(_)));
    assert!(err.to_string().contains("non-negative integer"));
}

#[test]
fn chunk_storage_key_is_stable_and_zero_padded() {
    let file_id = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();

    let key = chunk_storage_key(file_id, 12);

    assert_eq!(key, "chunks/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/00000012");
}

#[test]
fn validate_chunk_body_size_accepts_configured_limit() {
    validate_chunk_body_size(MAX_UPLOAD_CHUNK_BODY_BYTES).unwrap();
}

#[test]
fn validate_chunk_body_size_rejects_oversized_chunk() {
    let err = validate_chunk_body_size(MAX_UPLOAD_CHUNK_BODY_BYTES + 1).unwrap_err();

    assert!(matches!(err, AppError::PayloadTooLarge(_)));
}
