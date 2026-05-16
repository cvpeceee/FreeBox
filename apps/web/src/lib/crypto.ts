/**
 * Client-side E2EE helpers for the FreeBox web app.
 *
 * Mirrors the logic in the Rust CLI (apps/cli/src/commands/download.rs) and
 * the freebox-crypto crate (packages/crypto/src/encryption.rs) using the
 * standard Web Crypto API so no WASM is required.
 *
 * Chunk wire format (set by the CLI upload path):
 *   serde_json::to_vec(&ChunkCiphertext { index, nonce, ciphertext })
 *   → JSON: { "index": 0, "nonce": [0,0,...], "ciphertext": [1,234,...] }
 *   where nonce and ciphertext are arrays of integers (serde_json's default
 *   serialization of Vec<u8>).
 */

import { files } from '@/lib/api';
import type { FileMeta } from '@/lib/api';

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/** Encode bytes to standard base64. Reverse of base64ToBytes. */
function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

/**
 * Build the 12-byte AES-GCM nonce for chunk `index`.
 * Layout: [4 zero bytes | 8-byte big-endian u64 index]
 * Mirrors Rust: nonce_bytes[4..].copy_from_slice(&index.to_be_bytes())
 */
function buildNonce(index: number): Uint8Array<ArrayBuffer> {
  const buf = new ArrayBuffer(12);
  const view = new DataView(buf);
  // JavaScript numbers are safe integers up to 2^53; split index into hi/lo u32.
  view.setUint32(4, Math.floor(index / 0x1_0000_0000), false);
  view.setUint32(8, index >>> 0, false);
  return new Uint8Array(buf);
}

/** Decode a standard base64 string to a Uint8Array. */
function base64ToBytes(b64: string): Uint8Array<ArrayBuffer> {
  const binary = atob(b64);
  const bytes = new Uint8Array(new ArrayBuffer(binary.length));
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/**
 * Decode the stored file name from its base64 envelope.
 * Mirrors CLI's `default_file_name()`: base64-decode → UTF-8 string →
 * take the last non-empty path segment.
 */
function decodeFileName(encodedName: string): string {
  try {
    const decoded = new TextDecoder().decode(base64ToBytes(encodedName));
    return decoded.split('/').filter(Boolean).pop() ?? 'download.bin';
  } catch {
    return 'download.bin';
  }
}

/** Shape of a ChunkCiphertext as serialised by serde_json (Vec<u8> → number[]). */
interface RawChunk {
  index: number;
  nonce: number[];
  ciphertext: number[];
}

// ---------------------------------------------------------------------------
// Upload — E2EE encrypt then upload
// ---------------------------------------------------------------------------

/**
 * Prepare a file for E2EE upload: generate a random AES-256-GCM key, compute
 * the SHA-256 of the full plaintext, and return per-chunk encrypt helpers.
 *
 * Mirrors the CLI's prepare_upload() in apps/cli/src/commands/upload.rs.
 *
 * NOTE: The CLI seals the file key with a Signal/X3DH session key before
 * storing the envelope. The web client stores the raw key bytes as the
 * envelope for now (Signal key agreement is not yet implemented in JS).
 */
export async function prepareUpload(file: File): Promise<{
  encrypted_key_envelope: string;
  content_hash: string;
  encrypted_name: string;
  encryptChunk: (index: number, plaintext: Uint8Array<ArrayBuffer>) => Promise<Uint8Array<ArrayBuffer>>;
}> {
  // 1. Generate a fresh random 32-byte AES-256 key for this file.
  const rawKey = crypto.getRandomValues(new Uint8Array(new ArrayBuffer(32)));
  const cryptoKey = await crypto.subtle.importKey(
    'raw',
    rawKey,
    { name: 'AES-GCM' },
    false,
    ['encrypt'],
  );

  // 2. SHA-256 of the full plaintext for content-addressable deduplication.
  const fileBuffer = await file.arrayBuffer();
  const hashBytes = new Uint8Array(await crypto.subtle.digest('SHA-256', fileBuffer));

  return {
    // Raw key bytes base64-encoded — the download side expects exactly 32 bytes.
    encrypted_key_envelope: bytesToBase64(rawKey),
    content_hash: bytesToBase64(hashBytes),
    encrypted_name: bytesToBase64(new TextEncoder().encode(file.name)),

    async encryptChunk(
      index: number,
      plaintext: Uint8Array<ArrayBuffer>,
    ): Promise<Uint8Array<ArrayBuffer>> {
      const nonce = buildNonce(index);
      const ciphertext = new Uint8Array(
        await crypto.subtle.encrypt({ name: 'AES-GCM', iv: nonce }, cryptoKey, plaintext),
      );
      // Serialize as JSON matching serde_json::to_vec(&ChunkCiphertext):
      // { "index": 0, "nonce": [0,...,12 ints], "ciphertext": [1,...] }
      const json = JSON.stringify({
        index,
        nonce: Array.from(nonce),
        ciphertext: Array.from(ciphertext),
      });
      return new Uint8Array(new TextEncoder().encode(json)) as Uint8Array<ArrayBuffer>;
    },
  };
}

// ---------------------------------------------------------------------------
// Download — decrypt then save
// ---------------------------------------------------------------------------

/**
 * Download all encrypted chunks for a file, decrypt them with AES-256-GCM,
 * and trigger a browser save-as dialog.
 *
 * This is the TypeScript equivalent of the CLI's `download_file()` in
 * `apps/cli/src/commands/download.rs`.
 *
 * @param meta - File metadata returned by `files.getMeta()` or the file list.
 * @param onProgress - Optional callback receiving a 0–100 progress value.
 */
export async function downloadAndDecrypt(
  meta: FileMeta,
  onProgress?: (pct: number) => void,
): Promise<void> {
  // 1. Decode the 32-byte AES key from its base64 envelope.
  //    Mirrors CLI: file_key_from_envelope() → STANDARD.decode() → [u8; 32]
  const keyBytes = base64ToBytes(meta.encrypted_key_envelope);
  if (keyBytes.length !== 32) {
    throw new Error(
      `Invalid key envelope: expected 32 bytes, got ${keyBytes.length}`,
    );
  }

  // 2. Import as a non-extractable AES-256-GCM CryptoKey.
  const cryptoKey = await crypto.subtle.importKey(
    'raw',
    keyBytes,
    { name: 'AES-GCM' },
    false,       // non-extractable — key bytes cannot be read back out
    ['decrypt'],
  );

  // 3. Download and decrypt each chunk in order.
  //    Each chunk is stored as JSON produced by serde_json::to_vec(&ChunkCiphertext).
  //    Mirrors CLI: serde_json::from_slice::<ChunkCiphertext>(&body)
  //    then freebox_crypto::decrypt_chunk(key, expected_index, &chunk)
  const plaintextParts: Uint8Array[] = [];

  for (let i = 0; i < meta.total_chunks; i++) {
    const buffer = await files.downloadChunk(meta.file_id, i);

    // Parse the JSON-encoded ChunkCiphertext.
    const text = new TextDecoder().decode(buffer);
    const chunk = JSON.parse(text) as RawChunk;

    // Validate chunk index before decrypting — mirrors decrypt_chunk's
    // index binding check that prevents reordering attacks.
    if (chunk.index !== i) {
      throw new Error(
        `Chunk reordering detected: expected index ${i}, got ${chunk.index}`,
      );
    }

    const nonce = Uint8Array.from(chunk.nonce);       // 12 bytes
    const ciphertext = Uint8Array.from(chunk.ciphertext); // includes 16-byte AES-GCM tag

    const plaintext = await crypto.subtle.decrypt(
      { name: 'AES-GCM', iv: nonce },
      cryptoKey,
      ciphertext,
    );

    plaintextParts.push(new Uint8Array(plaintext));
    onProgress?.(Math.round(((i + 1) / meta.total_chunks) * 100));
  }

  // 4. Concatenate all plaintext parts into a single buffer.
  const totalLength = plaintextParts.reduce((sum, p) => sum + p.length, 0);
  const fullPlaintext = new Uint8Array(totalLength);
  let offset = 0;
  for (const part of plaintextParts) {
    fullPlaintext.set(part, offset);
    offset += part.length;
  }

  // 5. Decode the file name — mirrors CLI's default_file_name().
  const fileName = decodeFileName(meta.encrypted_name);

  // 6. Trigger a browser save-as download.
  const blob = new Blob([fullPlaintext], { type: 'application/octet-stream' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = fileName;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
