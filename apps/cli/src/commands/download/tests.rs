use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD, Engine};
use tempfile::tempdir;

use super::{default_file_name, file_key_from_envelope, resolve_output_path};

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
