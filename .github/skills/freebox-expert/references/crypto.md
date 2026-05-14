# FreeBox Cryptography Deep-Dive

This reference teaches the cryptographic protocols used in FreeBox, grounded in actual code.

---

## The Crypto Stack (from bottom to top)

```
User Password
    └──[Argon2id, 64 MiB, 3 iter, 4 lanes]──► Master Secret (256-bit)
                                                    ├──► Identity Key Pair  (Ed25519 — signing)
                                                    ├──► Signed Prekey      (X25519 — key exchange)
                                                    └──► One-Time Prekeys   (X25519 × N)
                                                                │
                                                         [X3DH Key Agreement]
                                                                │
                                                         Shared Secret (32 bytes)
                                                                │
                                                         [Double Ratchet]
                                                                │
                                                         Session Message Keys
                                                                │
                                                    [Seal FileKey with session key]
                                                                │
                                                         FileKey (AES-256-GCM, per-file)
                                                                │
                                                    [AES-256-GCM per chunk, unique nonce]
                                                                │
                                                         Encrypted chunks → server
```

**Key insight**: The server only ever receives the bottom: encrypted chunks + a sealed FileKey. It has no access to any layer above.

---

## 1. Argon2id — Password to Master Secret

**File**: `packages/crypto/src/keys.rs` → `MasterSecret::derive()`

### Why Argon2id?
- **Memory-hard**: Costs attacker 64 MiB RAM per guess — GPU brute-force becomes economically infeasible
- **Argon2id** (hybrid): Resists both GPU attacks (Argon2d) and side-channel attacks (Argon2i)
- **OWASP 2023 parameters**: 64 MiB memory, 3 iterations, 4 parallel lanes

### Parameters Explained
```
m = 64 MiB  → Memory cost. Each guess allocates this. 10,000 GPU cores × 64 MiB = 640 GB VRAM needed.
t = 3       → Time cost (iterations). Triples the time per guess.
p = 4       → Parallelism (lanes). Uses multiple CPU cores — also costs attacker more.
output = 32 → 256-bit master secret (input to Ed25519/X25519 key derivation).
```

### Common Mistake to Avoid
The salt is **not secret** but **must be unique per user** and **stored at registration**. FreeBox stores it in the DB and serves it via `/api/v1/auth/salt/:username`. Without a unique salt, two users with the same password would get the same master secret.

### Design Question to Consider
> Can an attacker enumerate salts via the `/auth/salt/:username` endpoint and launch pre-computation attacks?

The endpoint is intentionally public because the client needs it before it can authenticate. The Argon2id parameters make pre-computation impractical even knowing the salt — this is by design.

---

## 2. Key Hierarchy — Ed25519 + X25519

**File**: `packages/crypto/src/keys.rs`

### Why Two Key Types?

| Key Type | Curve | Use | Why |
|----------|-------|-----|-----|
| Identity Key | Ed25519 | Signing | Compact signatures, fast verify |
| Signed Prekey | X25519 | ECDH | DH only, no signing |
| One-Time Prekeys | X25519 | ECDH (one-use) | Forward secrecy per session |

Ed25519 and X25519 are both based on **Curve25519** but serve different mathematical operations:
- Ed25519 = Edwards-form Curve25519 → used for **signatures** (EdDSA)
- X25519 = Montgomery-form Curve25519 → used for **Diffie-Hellman**

### `zeroize` — Memory Safety for Secrets
```rust
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MasterSecret([u8; 32]);
```
When `MasterSecret` goes out of scope, the `[u8; 32]` array is overwritten with zeros before deallocation. Without this, the bytes might sit in freed heap memory and be readable via a heap dump or memory inspection tool.

**Teaching point**: `ZeroizeOnDrop` is a derive macro that implements `Drop` to call `zeroize()`. All secret types in FreeBox use this pattern.

---

## 3. X3DH — Asynchronous Key Agreement

**File**: `packages/crypto/src/signal.rs` → `x3dh_initiate()`, `x3dh_respond()`

### The Problem X3DH Solves
How can Alice encrypt a message to Bob **when Bob is offline**? Bob can't participate in a live DH exchange if he's not online.

### X3DH Solution
Bob pre-uploads a **prekey bundle** to the server:
```
IdentityKey (IK_B)     — long-term Ed25519 key
SignedPrekey (SPK_B)   — medium-term X25519 key, signed by IK_B
One-TimePrekey (OPK_B) — ephemeral X25519 key (single-use)
```

Alice then runs **4 DH computations** against Bob's bundle:
```
DH1 = DH(IK_A,  SPK_B)   — Alice's identity    × Bob's signed prekey
DH2 = DH(EK_A,  IK_B)    — Alice's ephemeral   × Bob's identity key
DH3 = DH(EK_A,  SPK_B)   — Alice's ephemeral   × Bob's signed prekey
DH4 = DH(EK_A,  OPK_B)   — Alice's ephemeral   × Bob's one-time prekey

SharedSecret = BLAKE3(DH1 || DH2 || DH3 || DH4, domain="freebox x3dh v1")
```

### Why 4 DH operations?
- **DH1**: Mutual authentication (both identity keys involved)
- **DH2**: Alice's freshness (ephemeral key)
- **DH3**: Bob's medium-term freshness (signed prekey rotates periodically)
- **DH4**: One-time forward secrecy (OPK deleted after use; protects this specific session)

### Critical Security Check: Signature Verification
```rust
fn verify_signed_prekey(identity_key_bytes, signed_prekey_public, signature) -> Result<()>
```
If this check is skipped, a malicious key server could swap Bob's signed prekey with its own and intercept the session. FreeBox performs this check before any DH computation.

---

## 4. Double Ratchet — Forward Secrecy + Break-in Recovery

**File**: `packages/crypto/src/signal.rs` → `RatchetSession`

### The Two Ratchets

**KDF Chain Ratchet** (symmetric): Each message key is derived from the previous chain key:
```
ChainKey_n → BLAKE3(ChainKey_{n-1}, domain="freebox chain advance v1")
MsgKey_n   → BLAKE3(ChainKey_n,     domain="freebox msg key v1")
```

**Diffie-Hellman Ratchet**: When Bob replies, he generates a new ephemeral key and does a DH with Alice's latest ephemeral. This "ratchets" the root key, healing the session:
```
(RootKey', ChainKey') = BLAKE3_KDF(RootKey, DH(EK_Alice, EK_Bob))
```

### Security Properties

| Property | Mechanism |
|----------|-----------|
| **Forward secrecy** | Old message keys are deleted after decryption; past sessions can't be recovered even if current state leaks |
| **Break-in recovery** | After a compromise, the DH ratchet generates a fresh root key on the next reply; attacker is locked out |
| **Message ordering** | Each message is numbered; out-of-order delivery is handled via saved message keys |

---

## 5. AES-256-GCM File Encryption

**File**: `packages/crypto/src/encryption.rs`

### Nonce Construction
```
Nonce (12 bytes) = [0x00, 0x00, 0x00, 0x00] || [chunk_index as u64 big-endian]
```
- **Unique per chunk**: Index prevents nonce reuse across chunks of the same file
- **Prevents reordering**: Chunk 3 decrypted with nonce for chunk 5 will fail (wrong index in nonce ≠ wrong auth tag)
- **Nonce reuse = catastrophic**: AES-GCM nonce reuse lets an attacker recover the keystream XOR, revealing plaintext differences

### The GCM Auth Tag (16 bytes)
Each chunk carries a 16-byte **authentication tag** (GHASH). If any single byte of the ciphertext is modified, decryption fails. This provides **authenticated encryption** — you get confidentiality AND integrity for free.

### Why Chunking (4 MiB default)?
- Memory bounded: Never load the full file into RAM
- Parallel upload: 8 concurrent HTTP streams, each carrying independent chunks
- Delta sync: Only re-upload chunks whose BLAKE3 hash changed

### Design Flaw to Watch: Chunk Count Authentication
The format does not include a **chunk count** in the sealed metadata. A truncation attack could remove trailing chunks — the client would decrypt fewer chunks than expected without detecting the missing ones. This should be addressed by including the total chunk count in the encrypted file metadata envelope.

---

## 6. BLAKE3 — Content Hashing for Deduplication

**Never sent to the server** — computed client-side on plaintext only.

- **3× faster** than SHA-256 on modern hardware
- **Tree-parallelizable**: Internally splits into 1 KiB chunks and hashes in parallel
- Used by FreeBox for **delta sync**: Compare chunk hashes to determine which 4 MiB blocks changed

**Teaching point**: Using BLAKE3 on plaintext before encryption, and never transmitting the hash, is intentional. If the hash were sent to the server, it could be used to detect if two users store the same file (cross-user deduplication) — a privacy leak called **hash-based probing** (cloud storage providers have been exploited this way).

---

## Design Flaws Checklist — Crypto

- [ ] **Nonce uniqueness**: AES-GCM nonces must never repeat for the same key. FreeBox uses chunk index — verify it's never reset for a re-upload of the same key.
- [ ] **FileKey reuse**: Each file must get a new `FileKey::generate()`. Sharing a key across files allows cross-file ciphertext analysis.
- [ ] **Signed prekey rotation**: SPK should rotate periodically. FreeBox code doesn't yet show automatic rotation scheduling.
- [ ] **One-time prekey exhaustion**: If the server runs out of OPKs, X3DH falls back to not using OPK (DH4 omitted). This weakens forward secrecy for that session. The server should refuse session initiation when OPKs are exhausted rather than silently downgrading.
- [ ] **Chunk count in envelope**: Missing total chunk count in the encrypted metadata allows truncation attacks.
- [ ] **Domain separation**: FreeBox centralises BLAKE3 domain strings in `signal::domains` — verify no two derivations share a domain string.
