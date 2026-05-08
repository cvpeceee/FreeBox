use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine};
use freebox_crypto::encryption::{decrypt_file, FileKey};

use super::{prepare_upload, remote_name};

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
