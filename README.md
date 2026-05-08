# FreeBox

> **The open-source, E2EE, high-performance cloud platform** — file storage, real-time messaging, and encrypted email in one extensible system. Built for privacy, designed for performance, architected for community.

---

## Vision

FreeBox is what happens when you take the best ideas from Dropbox, Signal, ProtonMail, and VS Code — and rebuild them from scratch with:

- **Zero-knowledge E2EE** using the Signal Protocol and MLS (RFC 9420)
- **Performance-first architecture** in Rust — targeting 5x faster sync than Dropbox
- **Bring Your Own Cloud** — S3, GCS, Azure, local disk, IPFS, or any OpenDAL-supported backend
- **Plugin ecosystem** — extend with messaging, mail, calendar, notes, and more
- **100% open-source** — Apache 2.0, self-hostable, auditable

This is not a hobby project. FreeBox is architected to compete with and surpass commercial offerings on every dimension except funding — and that is where the community comes in.

---

## What FreeBox Can Do

| Feature | Status | Protocol |
|---|---|---|
| Encrypted file storage | Building | AES-256-GCM + Signal |
| Delta sync (rsync-like) | Building | BLAKE3 + CRDT |
| Real-time messaging | Building | Signal Double Ratchet |
| Encrypted email | Building | MLS RFC 9420 |
| OAuth2 login (GitHub, Google, Microsoft, Apple, Facebook) | Building | OAuth2 + PKCE |
| CLI tool (`fbx`) | Building | — |
| Desktop app (Tauri) | Planned | — |
| Mobile app (Expo) | Planned | — |
| FUSE filesystem mount | Planned | — |
| IPFS storage backend | Planned | — |

---

## Performance Targets

FreeBox is built performance-first. Every architectural decision is benchmarked.

| Metric | Dropbox (reference) | FreeBox Target |
|---|---|---|
| Upload throughput | ~10 MB/s | **>=50 MB/s** (8x parallel chunks) |
| Sync latency (small files) | 2-5 seconds | **< 500 ms** (WebSocket push) |
| Desktop app memory | ~500 MB | **< 50 MB** (Rust/Tauri) |
| Desktop app binary size | ~300 MB | **< 20 MB** |
| Hashing speed (dedup) | SHA-256 @ ~1 GB/s | **BLAKE3 @ ~3 GB/s** |

---

## Architecture Overview

FreeBox is a **plugin-based platform** with a small, auditable Rust kernel. Nothing touches plaintext except your device.

```
                            CLIENTS
   Web (Next.js 15) | Desktop (Tauri 2) | Mobile (Expo) | CLI (fbx)
                              |
                    TLS 1.3 / WebSocket
                              |
                    API GATEWAY (Rust + Axum)
              Auth . Rate limiting . Request routing
                              |
                       FREEBOX KERNEL (Rust)
    +------------+  +-------------+  +----------+  +----------+
    |  Plugin    |  |  Event Bus  |  |   Auth   |  |  Crypto  |
    |  Registry  |  | (type-erased)|  |  (JWT)   |  |  Core    |
    +------------+  +-------------+  +----------+  +----------+
                              |
                   Plugin API (Rust traits)
        +----------+-----------+----------+----------+
        |          |           |          |          |
  +----------+ +--------+ +-------+ +--------+ +----------+
  | Storage  | |Messaging| | Mail  | | Sync   | | 3rd-party|
  | Plugins  | | Plugin  | |Plugin | |Engine  | | Plugins  |
  |S3/GCS/   | |(Signal) | |(MLS)  | |(CRDT)  | |          |
  |Local/... | |         | |       | |        | |          |
  +----+-----+ +---------+ +-------+ +--------+ +----------+
       |
       v  Apache OpenDAL (50+ storage backends)
  [S3] [GCS] [Azure] [Local] [IPFS] [B2] [WebDAV] [MinIO] ...
```

---

## API Architecture Decision: REST + WebSocket (not GraphQL, not gRPC)

FreeBox uses **REST for CRUD operations** and **WebSocket for real-time events** (messaging, sync notifications). This is the same architecture used by Signal, WhatsApp, and Dropbox.

### Why not GraphQL?

GraphQL is designed for APIs with complex, nested, typed data where clients benefit
from selecting specific fields. FreeBox payloads are **encrypted ciphertext** — opaque
binary blobs the server cannot inspect. GraphQL provides zero value here and adds
significant cost:

| Concern | REST | GraphQL |
|---|---|---|
| Binary file chunks | Native streaming, zero overhead | Must base64-encode (+33% bandwidth) |
| Encrypted payloads | Raw bytes, no schema needed | Schema useless on ciphertext |
| Upload throughput | Multi-part streaming | Single JSON payload |
| Caching | HTTP native (ETag, If-None-Match) | Requires custom caching layer |
| Tooling | curl, browser, any HTTP client | Requires GraphQL client library |

### Why not gRPC / Protobuf?

gRPC excels at **internal service-to-service** communication with complex typed messages.
For FreeBox's client-facing API, it introduces friction without benefit:

| Concern | REST | gRPC |
|---|---|---|
| Browser support | Native | Requires grpc-web proxy (extra hop) |
| Mobile support | Every HTTP library works | Per-platform codegen required |
| Debugging | curl, browser devtools | Needs grpcurl / Postman gRPC |
| Encrypted payloads | Raw bytes | Protobuf wrapper overhead (marginal) |
| Latency | ~1ms more than gRPC | Slightly faster serialization |

**Future hybrid**: If FreeBox grows to multiple internal microservices, gRPC will be
used for **server-to-server** calls. The client-facing API stays REST + WebSocket.

### API Route Map

```
Public (no auth):
  GET  /health                              Liveness probe
  POST /api/v1/auth/register                Create account + upload prekey bundle
  POST /api/v1/auth/login                   Authenticate, returns JWT pair
  POST /api/v1/auth/refresh                 Rotate refresh token
  GET  /api/v1/auth/salt/:username          Fetch non-secret client Argon2id salt
  GET  /api/v1/auth/oauth/:provider         Initiate OAuth2 flow (GitHub/Google/Microsoft/Apple/Facebook)
  GET  /api/v1/auth/oauth/:provider/callback  OAuth2 callback (code + PKCE exchange)

Authenticated (Bearer JWT):
  POST /api/v1/auth/logout                  Revoke refresh token
  GET  /api/v1/auth/providers               List linked OAuth providers + has_password flag
  POST /api/v1/auth/oauth/:provider/link    Link a new OAuth provider to current account
  DELETE /api/v1/auth/oauth/:provider/unlink  Unlink an OAuth provider (lockout-safe)

  GET  /api/v1/keys/:user_id               Fetch peer's prekey bundle (X3DH)
  POST /api/v1/keys/one-time               Replenish one-time prekeys

  POST /api/v1/files/upload/init            Begin chunked upload
  PUT  /api/v1/files/upload/:id             Upload one encrypted chunk
  POST /api/v1/files/upload/:id/complete    Finalize upload
  GET  /api/v1/files                        List user's files (paginated)
  GET  /api/v1/files/:id                    Get file metadata
  GET  /api/v1/files/:id/chunk/:n           Download encrypted chunk n
  DELETE /api/v1/files/:id                  Soft-delete (trash)

WebSocket (authenticated):
  WS   /ws/sync                             Real-time file change notifications
  WS   /ws/messaging                        Signal Protocol message relay
```

---

## E2EE Design — Zero Knowledge

The server **never sees your plaintext**. Ever.

### Key Hierarchy

```
User Password
    |--[Argon2id 64MiB/3iter/4-parallel]--> Master Secret (256-bit)
                          |
                          |--> Identity Key Pair (Ed25519 — signing)
                          |       Deterministically derived from master secret
                          |       via BLAKE3 domain-separated KDF
                          |
                          |--> Signed Prekey (X25519 — key exchange)
                          |       Rotated every 7 days
                          |       Signed by identity key (prevents MITM)
                          |
                          |--> One-Time Prekeys (X25519 x 100)
                          |       Single-use, server deletes after consumption
                          |
                          |--> Session Keys (Double Ratchet)
                          |       Forward secrecy + break-in recovery
                          |
                          '--> File Keys (AES-256-GCM, one per file)
                                Sealed with session key before upload
```

### File Upload Flow

```
1. File -> split into 4 MiB chunks
2. Each chunk -> compress (Zstd) -> encrypt (AES-256-GCM, unique key per file)
   - Nonce embeds chunk index (prevents reordering attacks)
   - Auth tag detects tampering per-chunk
3. File key -> seal with Signal session key -> EncryptedKeyEnvelope
4. Upload via REST API: [encrypted chunks] + [EncryptedKeyEnvelope] -> server
5. Server stores only ciphertext. Cannot read file names, contents, or keys.
```

### Signal Protocol Implementation

FreeBox implements the full X3DH + Double Ratchet protocol:

**X3DH (Extended Triple Diffie-Hellman)** — asynchronous key agreement:
- Allows Alice to start a session with Bob without Bob being online
- Performs 3-4 DH operations using a single ephemeral key (not multiple — this is critical)
- Verifies the Ed25519 signature on Bob's signed prekey (prevents MITM)
- Provides both initiator (`x3dh_initiate`) and responder (`x3dh_respond`) functions

**Double Ratchet** — per-message key rotation:
- Each message advances the ratchet, deriving a new AES-256-GCM key
- Chain length guard at 1,000,000 messages (prevents u32 overflow)
- Domain separation strings centralized in `signal::domains` module
- All key material implements `ZeroizeOnDrop`

### MLS Protocol (Group E2EE)

Files shared with groups and group email use **MLS (RFC 9420)** — a modern
group key agreement protocol that scales to thousands of members with
logarithmic overhead, unlike Signal's Sender Keys.

---

## Repository Structure

```
freebox/
+-- apps/
|   +-- server/          # Rust backend (Axum 0.7, Tokio, SQLx)
|   +-- web/             # Next.js 15 (App Router, React 19) [planned]
|   +-- desktop/         # Tauri 2.0 (Rust + WebView) [planned]
|   +-- mobile/          # Expo / React Native [planned]
|   +-- cli/             # Rust CLI binary (`fbx`)
|
+-- packages/
|   +-- core/            # Plugin API traits, event bus, storage abstraction
|   |                    # Dual-licensed: Apache-2.0 OR MIT
|   +-- crypto/          # Signal Protocol, AES-GCM, Argon2id, BLAKE3
|   |                    # Single auditable crate for ALL cryptography
|   +-- sync-engine/     # CRDT delta-sync, chunking [planned]
|
+-- plugins/             # First-party plugins (Apache 2.0)
|   +-- storage-local/   # Local filesystem (OpenDAL Fs backend)
|   +-- storage-s3/      # S3 / MinIO / R2 / B2 (OpenDAL S3 backend)
|   +-- storage-gcs/     # Google Cloud Storage [planned]
|   +-- messaging/       # Real-time chat (Signal Double Ratchet) [planned]
|   +-- mail/            # Encrypted email (MLS + SMTP/IMAP) [planned]
|
+-- infra/
|   +-- docker-compose.dev.yml   # Postgres 17, Dragonfly, MinIO, NATS
|
+-- docs/
    +-- architecture.md          # ADRs, DB schema, deployment
```

---

## Tech Stack

| Layer | Technology | Rationale |
|---|---|---|
| Backend language | **Rust** | Memory safety, zero-cost abstractions, 2-5x faster than Go |
| HTTP framework | **Axum 0.7** | Tower-compatible, async-native, excellent ergonomics |
| API protocol | **REST + WebSocket** | Binary streaming, browser-native, same as Signal/Dropbox |
| Async runtime | **Tokio** | De-facto standard; excellent ecosystem |
| Database | **PostgreSQL 17** | Metadata, users, audit log |
| Cache | **Dragonfly** | Redis-compatible, 25x faster, better memory efficiency |
| File storage | **Apache OpenDAL** | Unified abstraction over 50+ backends |
| Sync protocol | **Automerge (CRDT)** | Conflict-free, offline-first |
| E2EE (1:1) | **Signal Protocol** | X3DH + Double Ratchet, audited |
| E2EE (groups) | **OpenMLS** | RFC 9420 implementation in Rust |
| Hashing | **BLAKE3** | 3x faster than SHA-256, parallelizable |
| Password KDF | **Argon2id** | Memory-hard (64 MiB), OWASP 2023 recommended |
| Compression | **Zstd** | Best compression ratio/speed trade-off |
| Message queue | **NATS JetStream** | Lower latency than Kafka for this workload |
| Web framework | **Next.js 15** | App Router, React Server Components |
| Desktop | **Tauri 2.0** | Rust backend, 10x smaller than Electron |
| Mobile | **Expo (RN)** | Code sharing with web, OTA updates |
| Monorepo | **Turborepo + pnpm** | Fast builds, remote caching |

---

## Security Design

Security is not a feature — it is a foundation.

### Defensive Measures

- **Double password hashing**: Client hashes with Argon2id, server re-hashes with Argon2id. Database leak reveals only double-hashed values.
- **OAuth2 with PKCE**: All OAuth flows use PKCE (S256) to prevent authorization code interception. CSRF state tokens are single-use with 5-minute TTL.
- **OAuth2 third-party login**: GitHub, Google, Microsoft, Apple, Facebook. Providers are opt-in via environment variables. Account linking auto-detects existing users by email.
- **Lockout prevention**: Users cannot unlink their last authentication method (must have a password or another OAuth provider before unlinking).
- **Apple Sign In**: id_token JWT verified against Apple's JWKS public keys (RS256 signature, expiry, issuer). Keys fetched from `https://appleid.apple.com/auth/keys`. Supports Apple's "Hide My Email" private relay addresses.
- **Input validation**: Username charset restricted (alphanumeric + `_-`), email format checked, hash/bundle size limits enforced.
- **Atomic operations**: Registration uses database transactions (user + prekey bundle created atomically).
- **JWT security**: Algorithm restricted to HS256 (prevents algorithm confusion attacks). Tokens validated with constant-time comparison.
- **Key material safety**: All secrets implement `ZeroizeOnDrop` (wiped from RAM on drop). `FileKey` is intentionally non-`Clone` to prevent accidental duplication.
- **Chunk integrity**: AES-GCM authentication tag per chunk. Nonce embeds chunk index (prevents reordering). Oversized chunks rejected at runtime (not just debug builds).
- **Ratchet overflow guard**: `MAX_CHAIN_LENGTH` (1M messages) prevents u32 wraparound that would cause key reuse.
- **Domain separation**: All BLAKE3 key derivations use unique domain strings, centralized in `signal::domains`.

### What the server knows

| Data | Server Sees |
|---|---|
| File contents | Encrypted ciphertext only |
| File names | Encrypted |
| Message contents | Encrypted ciphertext only |
| Email contents | Encrypted ciphertext only |
| File/message metadata (size, timestamp) | Yes (minimized, necessary for sync) |
| Identity (username, email address) | Yes (required for delivery) |

### Code Review and Audit Trail

The codebase has been through a detailed security review. Key fixes applied:

- EventBus redesigned to be object-safe (type-erased `publish_raw`/`subscribe_raw`)
- X3DH fixed to use single `StaticSecret` for all DH operations (was using separate ephemeral keys)
- X3DH signature verification added (was missing, allowing MITM attacks)
- X3DH responder function added (was completely missing)
- bcrypt replaced with Argon2id for server-side password verification
- Refresh token persistence implemented (was never stored to database)
- `StorageProvider` wired into `AppState` (was missing)
- Apple Sign In id_token decoding with JWKS signature verification (was stub)

See `docs/architecture.md` for the full ADR index.

---

## Getting Started

### Prerequisites

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown   # for crypto WASM build

# Node.js tooling
npm install -g pnpm@latest
npm install -g turbo

# Rust dev tools
cargo install cargo-watch   # hot reload
cargo install sqlx-cli      # database migrations
cargo install cargo-audit   # security audit
```

### Start development environment

```bash
# 1. Clone the repository
git clone https://github.com/freebox-io/freebox
cd freebox

# 2. Start infrastructure (Postgres, Dragonfly, MinIO, NATS)
docker compose -f infra/docker-compose.dev.yml up -d

# 3. Run database migrations
cd apps/server && sqlx migrate run

# 4. Run the test suite (crypto crate has no infra dependency)
cargo test --workspace

# 5. Start the Rust backend (hot reload)
cargo watch -x "run -p freebox-server"

# 6. Start the web app (separate terminal)
cd apps/web && pnpm dev
```

### CLI Quick Start

```bash
# Install the CLI
cargo install --path apps/cli

# Register and login
fbx auth register --server https://freebox.io
fbx auth login

# Upload a file (automatically E2EE)
fbx upload ./document.pdf remote://documents/

# Sync a folder
fbx sync ./projects remote://projects --watch

# Send an encrypted message
fbx msg send @alice "Hey, this is E2EE!"

# Manage storage providers
fbx provider add s3 --bucket my-bucket --region us-east-1
fbx provider list

# Compose encrypted email
fbx mail compose --to bob@example.com
```

---

## Plugin Development

Third-party plugins implement Rust traits from the `freebox-core` crate.
Plugins are sandboxed via WASM (using `wasmtime`) for untrusted code.

```toml
# my-plugin/plugin.toml
[plugin]
id      = "my-storage-plugin"
name    = "My Custom Storage"
version = "1.0.0"
api_version = "^1.0"

[plugin.capabilities]
provides = ["storage.provider"]
```

```rust
// my-plugin/src/lib.rs
use freebox_core::{StorageProvider, PluginManifest, async_trait};

pub struct MyStorage;

#[async_trait]
impl StorageProvider for MyStorage {
    fn id(&self) -> &str { "my-storage" }

    async fn put(&self, key: &str, data: bytes::Bytes) -> freebox_core::Result<()> {
        // Your implementation — all data is already encrypted
        Ok(())
    }
    // ... other trait methods
}
```

### Event Bus

Plugins communicate via a type-erased event bus (never direct calls).
The bus is object-safe (`Arc<dyn EventBus>`) with type-safe wrappers via `EventBusExt`:

```rust
use freebox_core::{EventBusExt, event::FileUploaded};

// Publish a typed event
ctx.event_bus.publish_typed(FileUploaded {
    file_id, user_id, size_bytes, content_hash,
}).await?;

// Subscribe to typed events
ctx.event_bus.subscribe_typed(|event: FileUploaded| async move {
    tracing::info!("File {} uploaded", event.file_id);
    Ok(())
}).await?;
```

A `NoopEventBus` is provided for unit testing plugins without a live NATS connection.

See [`docs/plugin-api.md`](docs/plugin-api.md) for the full specification.

---

## Impact

FreeBox addresses a critical need: **the world lacks a fast, modern, open-source, E2EE productivity platform**.

- **ProtonMail / Proton Drive** — E2EE but closed-source, expensive, no plugin ecosystem
- **Nextcloud** — Open-source but sluggish (PHP), no E2EE by default, poor mobile experience
- **Keybase** — E2EE but acquired by Zoom, stagnant, limited storage
- **Dropbox / Google Drive** — Fast and polished but surveils your data

FreeBox fills all four quadrants: **open + fast + E2EE + extensible**.

Estimated addressable community:
- 50M+ developers and privacy-conscious users currently underserved
- Enterprises blocked from cloud due to data sovereignty requirements
- Journalists, activists, and researchers requiring genuine privacy

---

## Contributing

We welcome contributions at every level.

1. Read `CONTRIBUTING.md`
2. Check open issues tagged `good first issue`
3. Join the discussion on Matrix: `#freebox:matrix.org`
4. For large changes, open an RFC in `docs/rfcs/` first

---

## License

Apache License 2.0 — See [LICENSE](LICENSE)

The plugin API contracts (`packages/core`) are additionally licensed under MIT
to maximize ecosystem compatibility.

---

## Acknowledgements

- [Signal Protocol](https://signal.org/docs/) — Double Ratchet and X3DH
- [OpenMLS](https://github.com/openmls/openmls) — RFC 9420 MLS implementation
- [Apache OpenDAL](https://opendal.apache.org/) — Unified storage abstraction
- [Automerge](https://automerge.org/) — CRDT for conflict-free sync
- [Axum](https://github.com/tokio-rs/axum) — Ergonomic Rust web framework
- [Tauri](https://tauri.app/) — Lightweight cross-platform desktop
- [RustCrypto](https://github.com/RustCrypto) — AES-GCM, Argon2, Ed25519, X25519
- [BLAKE3](https://github.com/BLAKE3-team/BLAKE3) — Fast cryptographic hashing
