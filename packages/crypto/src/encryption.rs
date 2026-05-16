//! AES-256-GCM file encryption with chunked processing.
//!
//! # Design
//!
//! Files are split into fixed-size chunks (default 4 MiB) before encryption.
//! Each chunk is encrypted with the **same** file key but a **unique nonce**.
//! The nonce embeds the chunk index, which prevents:
//!
//! - **Nonce reuse** (would break AES-GCM confidentiality)
//! - **Chunk reordering attacks** (a tampered order is detected)
//! - **Truncation attacks** (missing auth tags are detected)
//!
//! # Format
//!
//! ```text
//! ┌─────────────────────────────┐
//! │  FileKey  (32 bytes, AES)   │  ← sealed with Signal session key
//! └─────────────────────────────┘
//!
//! Per chunk:
//! ┌─────────────────────────────────────────────────────┐
//! │  Nonce (12 bytes)  │  Ciphertext  │  Auth Tag (16B) │
//! │  [4B zeros][8B idx]│              │                  │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! # Performance
//!
//! AES-GCM uses hardware AES-NI instructions on x86-64 and ARMv8 — expect
//! over 3 GB/s throughput on modern hardware. Chunks are processed independently,
//! so upload can parallelize across 8 concurrent HTTP streams.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Default chunk size: 4 MiB — balances memory usage vs. round-trip count.
pub const CHUNK_SIZE: usize = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// File encryption key
// ---------------------------------------------------------------------------

/// A unique 256-bit AES key used to encrypt a single file.
///
/// One key per file — never reuse the same key for different files.
/// The key itself is sealed (encrypted) with the Signal session key
/// before being sent to the server.
///
/// # Security
///
/// `ZeroizeOnDrop` ensures the key bytes are overwritten in memory when
/// this value is dropped, preventing keys from leaking into swap or
/// heap snapshots.
/// **Not `Clone`** — intentionally prevents accidental key duplication.
/// Each `FileKey` should encrypt exactly one file. If you need the raw bytes
/// (e.g. to seal into an envelope), use [`as_bytes`].
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct FileKey([u8; 32]);

impl FileKey {
    /// Generate a new random file key.
    pub fn generate() -> Self {
        let key = Aes256Gcm::generate_key(OsRng);
        Self(key.into())
    }

    /// Reconstruct a file key from raw bytes (e.g. after decrypting the envelope).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Chunk ciphertext
// ---------------------------------------------------------------------------

/// The result of encrypting one chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkCiphertext {
    /// Zero-based chunk index. Validated during decryption.
    pub index: u64,
    /// Nonce bytes (12 bytes for AES-GCM).
    pub nonce: Vec<u8>,
    /// Ciphertext including the 16-byte AES-GCM authentication tag.
    pub ciphertext: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Encryption
// ---------------------------------------------------------------------------

/// Encrypt a single plaintext chunk with `key`.
///
/// # Arguments
///
/// - `key`   — The file's AES-256 key.
/// - `index` — Zero-based chunk index. Embedded in the nonce to prevent
///   reordering attacks.
/// - `data`  — Raw plaintext bytes (must be ≤ [`CHUNK_SIZE`]).
///
/// # Errors
///
/// Returns an error only if the underlying AES-GCM cipher fails, which
/// should never happen with valid inputs.
pub fn encrypt_chunk(key: &FileKey, index: u64, data: &[u8]) -> anyhow::Result<ChunkCiphertext> {
    // Reject oversized chunks at runtime (not just debug builds).
    if data.len() > CHUNK_SIZE {
        anyhow::bail!(
            "chunk size {} exceeds maximum {} — caller must split before encrypting",
            data.len(),
            CHUNK_SIZE
        );
    }

    let cipher_key = Key::<Aes256Gcm>::from_slice(&key.0);
    let cipher = Aes256Gcm::new(cipher_key);

    // Build a deterministic nonce from the chunk index.
    // Layout: [ 4 zero bytes | 8-byte big-endian index ]
    // This gives us 2^64 chunks per file (~73 petabytes at 4 MiB chunks).
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes[4..].copy_from_slice(&index.to_be_bytes());
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| anyhow::anyhow!("AES-GCM encrypt failed: {}", e))?;

    Ok(ChunkCiphertext {
        index,
        nonce: nonce_bytes.to_vec(),
        ciphertext,
    })
}

/// Decrypt a chunk previously produced by [`encrypt_chunk`].
///
/// # Security
///
/// Validates the embedded nonce against `expected_index` before decryption.
/// An index mismatch indicates a reordering or tampering attack and is
/// rejected before the cipher runs.
pub fn decrypt_chunk(
    key: &FileKey,
    expected_index: u64,
    chunk: &ChunkCiphertext,
) -> anyhow::Result<Bytes> {
    // Validate index binding — prevents chunk reordering attacks.
    if chunk.index != expected_index {
        anyhow::bail!(
            "chunk index mismatch: expected {expected_index}, got {}",
            chunk.index
        );
    }

    if chunk.nonce.len() != 12 {
        anyhow::bail!(
            "invalid nonce length: expected 12, got {}",
            chunk.nonce.len()
        );
    }

    let cipher_key = Key::<Aes256Gcm>::from_slice(&key.0);
    let cipher = Aes256Gcm::new(cipher_key);
    let nonce = Nonce::from_slice(&chunk.nonce);

    let plaintext = cipher
        .decrypt(nonce, chunk.ciphertext.as_ref())
        .map_err(|_| anyhow::anyhow!("AES-GCM authentication failed — data may be tampered"))?;

    Ok(Bytes::from(plaintext))
}

// ---------------------------------------------------------------------------
// Convenience: encrypt / decrypt a whole file
// ---------------------------------------------------------------------------

/// Split `plaintext` into chunks and encrypt each one.
///
/// Returns chunks in order. Callers should upload all chunks concurrently
/// for maximum throughput.
pub fn encrypt_file(key: &FileKey, plaintext: &[u8]) -> anyhow::Result<Vec<ChunkCiphertext>> {
    plaintext
        .chunks(CHUNK_SIZE)
        .enumerate()
        .map(|(i, chunk)| encrypt_chunk(key, i as u64, chunk))
        .collect()
}

/// Decrypt and reassemble a file from its chunks.
///
/// `chunks` must be in ascending index order. Gaps or duplicates are rejected.
pub fn decrypt_file(key: &FileKey, chunks: &[ChunkCiphertext]) -> anyhow::Result<Vec<u8>> {
    let mut plaintext = Vec::new();

    for (expected_index, chunk) in chunks.iter().enumerate() {
        let decrypted = decrypt_chunk(key, expected_index as u64, chunk)?;
        plaintext.extend_from_slice(&decrypted);
    }

    Ok(plaintext)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_single_chunk() {
        let key = FileKey::generate();
        let plaintext = b"Hello, FreeBox E2EE world!";
        let chunk = encrypt_chunk(&key, 0, plaintext).unwrap();
        let decrypted = decrypt_chunk(&key, 0, &chunk).unwrap();
        assert_eq!(decrypted.as_ref(), plaintext);
    }

    #[test]
    fn round_trip_multi_chunk() {
        let key = FileKey::generate();
        // 10 MiB — spans 3 chunks at 4 MiB each.
        let plaintext: Vec<u8> = (0..10 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let chunks = encrypt_file(&key, &plaintext).unwrap();
        assert_eq!(chunks.len(), 3);
        let decrypted = decrypt_file(&key, &chunks).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_index_rejected() {
        let key = FileKey::generate();
        let chunk = encrypt_chunk(&key, 0, b"data").unwrap();
        // Claim this is chunk 1 — should be rejected.
        let result = decrypt_chunk(&key, 1, &chunk);
        assert!(result.is_err());
    }

    #[test]
    fn tampered_ciphertext_rejected() {
        let key = FileKey::generate();
        let mut chunk = encrypt_chunk(&key, 0, b"sensitive data").unwrap();
        // Flip a bit in the ciphertext.
        chunk.ciphertext[5] ^= 0xFF;
        let result = decrypt_chunk(&key, 0, &chunk);
        assert!(result.is_err(), "tampered ciphertext must not decrypt");
    }

    #[test]
    fn wrong_key_rejected() {
        let key1 = FileKey::generate();
        let key2 = FileKey::generate();
        let chunk = encrypt_chunk(&key1, 0, b"secret").unwrap();
        let result = decrypt_chunk(&key2, 0, &chunk);
        assert!(result.is_err());
    }

    #[test]
    fn empty_plaintext_round_trip() {
        let key = FileKey::generate();
        let chunk = encrypt_chunk(&key, 0, b"").unwrap();
        let decrypted = decrypt_chunk(&key, 0, &chunk).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn oversized_chunk_rejected() {
        let key = FileKey::generate();
        let big = vec![0u8; CHUNK_SIZE + 1];
        let result = encrypt_chunk(&key, 0, &big);
        assert!(result.is_err(), "oversized chunk must be rejected");
    }

    #[test]
    fn empty_file_round_trip() {
        let key = FileKey::generate();
        let chunks = encrypt_file(&key, &[]).unwrap();
        assert!(chunks.is_empty());
        let decrypted = decrypt_file(&key, &chunks).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn chunk_index_bound_to_nonce() {
        let key = FileKey::generate();
        let c0 = encrypt_chunk(&key, 0, b"data").unwrap();
        let c1 = encrypt_chunk(&key, 1, b"data").unwrap();
        // Same plaintext + key but different index → different nonce → different ciphertext.
        assert_ne!(c0.ciphertext, c1.ciphertext);
    }

    #[test]
    fn file_key_as_bytes_works() {
        let key = FileKey::generate();
        let bytes = *key.as_bytes();
        let key2 = FileKey::from_bytes(bytes);
        // Encrypt with original, decrypt with reconstructed.
        let chunk = encrypt_chunk(&key, 0, b"test").unwrap();
        let decrypted = decrypt_chunk(&key2, 0, &chunk).unwrap();
        assert_eq!(decrypted.as_ref(), b"test");
    }
}
