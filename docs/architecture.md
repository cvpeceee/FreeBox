# FreeBox Architecture

## Overview

FreeBox is a **plugin-based platform** with a small, auditable Rust kernel.
Every capability — storage, messaging, email — is a plugin that implements
well-defined trait interfaces. The kernel provides auth, routing, rate limiting,
and an event bus; plugins provide all domain-specific functionality.

---

## Core Principles

### 1. Zero-Knowledge Server

The server stores only encrypted ciphertext. Private keys never leave the
client device. This means:

- Server compromise = attacker gets encrypted blobs (useless without keys)
- FreeBox the company cannot read your files (important for the hosted version)
- Open-source + self-hosted = no need to trust anyone

### 2. Performance First

Every architectural decision is benchmarked:

- **Rust** over Go/Python — zero-cost abstractions, no GC pauses
- **BLAKE3** over SHA-256 — 3× faster content hashing, tree-parallelizable
- **Zstd** over gzip — better ratio/speed at level 3
- **Dragonfly** over Redis — 25× faster cache, same API
- **Parallel chunk upload** — 8 concurrent streams per file
- **Delta sync** — only changed 4 MB chunks are transferred

### 3. Plugin Ecosystem

Modelled after VS Code's extension system:

- Small, auditable kernel (~5 KLOC)
- All capabilities provided by plugins
- Plugins communicate via typed event bus (never direct calls)
- WASM sandboxing for untrusted third-party plugins
- Plugin manifest schema validated at load time

### 4. Hardened for Direct Internet

The server is designed to be reachable directly (no trusted reverse proxy
required). Security measures that depend on request headers (e.g.,
`X-Forwarded-For`) are deliberately not used — all rate limiting and IP
attribution uses the authenticated TCP peer address extracted by Axum's
`ConnectInfo<SocketAddr>`.

---

## Request Lifecycle

```
Client
  │
  │  HTTPS (TLS 1.3 minimum)
  ▼
API Gateway (Axum)
  │  SetRequestIdLayer (X-Request-Id for log correlation)
  │  TraceLayer (structured access logs)
  │  CompressionLayer (Brotli / Gzip responses)
  │  CorsLayer
  ▼
Rate Limit Middleware (RateLimiter)
  │  In-process token-bucket per bearer-token hash or peer IP
  │  Ignores X-Forwarded-For (spoofable); uses ConnectInfo peer IP
  │  Returns HTTP 429 when bucket exhausted
  ▼
Auth Middleware (authenticated routes only)
  │  Validates JWT Bearer token
  │  Injects Claims into request extensions
  ▼
Route Handler (e.g. files::upload_chunk)
  │  Pulls Claims, validates ownership
  │  Enforces body size limit (32 MiB per chunk via DefaultBodyLimit)
  │  Validates prekey bundle / one-time prekey crypto on key routes
  │  Calls StorageProvider::put()
  ▼
Plugin Registry → StorageProvider (S3/R2 / Local)
  │
  │  Publishes FileUploaded event
  ▼
Event Bus (NATS JetStream)
  │  Fan-out to subscribers
  ▼
Other Plugins (messaging, mail, sync notifications...)
```

---

## API Routes

```
GET  /health                                       Liveness probe (no auth)

# Auth — password
POST /api/v1/auth/register                         Create account + upload prekey bundle
POST /api/v1/auth/login                            Password auth → JWT pair
POST /api/v1/auth/refresh                          Refresh access token
POST /api/v1/auth/logout                           Revoke refresh token
GET  /api/v1/auth/salt/:username                   Fetch Argon2 salt for client key derivation

# Auth — OAuth2 (GitHub, Google, Microsoft, Apple, Facebook)
GET  /api/v1/auth/oauth/:provider                  Initiate OAuth2 + PKCE flow
GET  /api/v1/auth/oauth/:provider/callback         OAuth2 code exchange
POST /api/v1/auth/oauth/:provider/link             Link provider to existing account (auth)
DELETE /api/v1/auth/oauth/:provider/unlink         Unlink provider (auth)
GET  /api/v1/auth/providers                        List linked OAuth providers (auth)

# Audit events
GET  /api/v1/auth/audit-events                     Per-user audit log (auth, paginated)
GET  /api/v1/admin/audit-events                    All-users audit log (admin only, filterable)

# Key server
GET  /api/v1/keys/:user_id                         Fetch prekey bundle (auth)
POST /api/v1/keys/one-time                         Replenish one-time prekeys (auth)

# File operations
POST /api/v1/files/upload/init                     Begin chunked upload → upload_id
PUT  /api/v1/files/upload/:upload_id               Upload a single encrypted chunk (32 MiB max)
POST /api/v1/files/upload/:upload_id/complete      Finalise upload
GET  /api/v1/files                                 List user files — ?limit=&offset= (paginated)
GET  /api/v1/files/trash                           List soft-deleted files in trash (30-day window, paginated)
GET  /api/v1/files/:file_id                        File metadata + chunk manifest
PATCH /api/v1/files/:file_id                       Update encrypted file name (rename)
GET  /api/v1/files/:file_id/chunk/:n               Download encrypted chunk n
DELETE /api/v1/files/:file_id                      Move file to trash (soft-delete)
POST /api/v1/files/:file_id/restore                Restore file from trash (within 30-day window)

# Storage
GET  /api/v1/storage/datasources                   List storage datasources (public)
GET  /api/v1/storage/buckets                       List buckets (auth)
POST /api/v1/storage/buckets                       Create bucket (auth)
```

### Admin Routes

Admin-only routes require the caller's user ID to appear in the
`ADMIN_USER_IDS` environment variable (comma-separated UUID list). A request
from a non-admin authenticated user returns HTTP 403.

---

## Rate Limiting

Implemented in `apps/server/src/rate_limit.rs` as an in-process token-bucket
limiter applied globally via Axum middleware.

| Property | Value |
|---|---|
| Algorithm | Sliding-window token bucket |
| Key | SHA hash of Bearer token (auth'd), or TCP peer IP (unauth'd) |
| Limit | Configurable (`RATE_LIMIT_REQUESTS` / `RATE_LIMIT_WINDOW_SECS`) |
| Default | 200 requests per 60 seconds |
| Bucket cap | 100 000 buckets (evicts LRU when exceeded) |
| Trusted headers | None — `X-Forwarded-For` / `X-Real-IP` are **ignored** |
| Response | HTTP 429 `Too Many Requests` |

---

## Security Hardening

### Upload Size Limits
Chunked upload requests are capped at **32 MiB** per chunk, enforced at two
layers:
1. Axum `DefaultBodyLimit` on the upload route (rejects oversized requests early).
2. `validate_chunk_body_size` in the handler (defence-in-depth).

### Prekey Crypto Validation
Registration validates the submitted prekey bundle using
`PrekeyBundle::validate_public` from `packages/crypto`, which performs real
elliptic-curve signature verification (Curve25519). One-time prekey replenishment
rejects empty batches, duplicate IDs, zero-valued keys, and batches exceeding the
per-request limit.

### OAuth Reactivation Window
When an OAuth login matches a soft-deleted account, reactivation is only
permitted if the account was deleted within `OAUTH_REACTIVATION_MAX_AGE_DAYS`
days (default: 30, set to `0` to disable the limit). All reactivations are
recorded in the persistent audit trail.

### OAuth Race Condition Mitigation
Concurrent OAuth callbacks for the same provider identity are handled using
`INSERT … ON CONFLICT DO NOTHING` followed by a fetch-on-race fallback, so
duplicate records are never created even under high concurrency.

---

## Persistent Audit Trail

### Database Table (`account_audit_events`)

```sql
CREATE TABLE account_audit_events (
    id               UUID        NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    user_id          UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_type       TEXT        NOT NULL,   -- e.g. 'account_reactivated'
    source           TEXT        NOT NULL,   -- e.g. 'oauth'
    provider         TEXT,                   -- e.g. 'github'
    provider_user_id TEXT,
    details          JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### Access
- **Per-user**: `GET /api/v1/auth/audit-events` — paginated, scoped to the
  caller's user ID.
- **Admin**: `GET /api/v1/admin/audit-events` — filterable by `user_id`,
  `event_type`, `source`, `provider`; offset-paginated; restricted to
  `ADMIN_USER_IDS` allowlist.

---

## Database Schema

Applied via SQLx migrations in `apps/server/migrations/`:

| Migration | Description |
|---|---|
| `001_initial_schema.sql` | Core tables: users, prekey_bundles, files, uploads, refresh_tokens |
| `002_oauth_accounts.sql` | OAuth tables: oauth_accounts, oauth_states |
| `003_account_audit_events.sql` | Audit trail: account_audit_events + indexes |

```sql
-- 001: Core tables (PostgreSQL 17)

CREATE TABLE users (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username      TEXT NOT NULL UNIQUE,
    email         TEXT NOT NULL UNIQUE,
    password_hash TEXT,                  -- Argon2id hash (NULL for OAuth-only users)
    argon2_salt   TEXT,                  -- Stored for client key re-derivation (NULL for OAuth-only)
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at    TIMESTAMPTZ           -- Soft delete
);

CREATE TABLE prekey_bundles (
    user_id    UUID PRIMARY KEY REFERENCES users(id),
    bundle     JSONB NOT NULL,          -- Public key data only (no secrets)
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE files (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id                UUID NOT NULL REFERENCES users(id),
    encrypted_name         TEXT NOT NULL,  -- AES-GCM encrypted filename
    size_bytes             BIGINT NOT NULL,
    total_chunks           INT NOT NULL,
    encrypted_key_envelope TEXT NOT NULL,  -- Signal-sealed file key
    content_hash           TEXT NOT NULL,  -- BLAKE3 of plaintext (for dedup)
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at             TIMESTAMPTZ      -- Soft delete (trash)
);

CREATE TABLE uploads (
    id                     UUID PRIMARY KEY,
    user_id                UUID NOT NULL REFERENCES users(id),
    total_chunks           INT NOT NULL,
    chunks_received        INT NOT NULL DEFAULT 0,
    size_bytes             BIGINT NOT NULL,
    encrypted_key_envelope TEXT NOT NULL,
    content_hash           TEXT NOT NULL,
    encrypted_name         TEXT NOT NULL,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW()
    -- Orphaned uploads cleaned up by a background job after 24h
);

CREATE TABLE refresh_tokens (
    token      TEXT PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users(id),
    username   TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 002: OAuth tables

CREATE TABLE oauth_accounts (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id           UUID NOT NULL REFERENCES users(id),
    provider          TEXT NOT NULL,         -- 'github', 'google', 'microsoft', 'apple', 'facebook'
    provider_user_id  TEXT NOT NULL,         -- user's ID at the provider
    provider_email    TEXT,
    provider_username TEXT,
    avatar_url        TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(provider, provider_user_id)      -- one FreeBox account per provider identity
);

CREATE TABLE oauth_states (
    state         TEXT PRIMARY KEY,         -- CSRF token (single-use, 5 min TTL)
    pkce_verifier TEXT NOT NULL,
    provider      TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at    TIMESTAMPTZ NOT NULL
);
```

---

## Storage

### Providers

| Provider | Plugin crate | Notes |
|---|---|---|
| Local filesystem | `plugins/storage-local` | Default for development |
| S3 / Cloudflare R2 | `plugins/storage-s3` | Set `STORAGE_PROVIDER=s3` or `r2` |

Selected via the `STORAGE_PROVIDER` environment variable. The active plugin is
loaded at startup by `AppState::new`.

### S3 / R2 Plugin (`plugins/storage-s3`)

- Uses **AWS SigV4** for request signing (compatible with Cloudflare R2, MinIO, etc.)
- **Auto-creates the bucket** on first use if a `NoSuchBucket` error is returned,
  so deployments do not require pre-provisioned buckets.
- Bucket-level operations (list, create) are handled via raw REST calls with
  SigV4 signing in `sigv4.rs` — not via the OpenDAL object interface.
- Credentials: `STORAGE_S3_ACCESS_KEY_ID` / `STORAGE_S3_SECRET_ACCESS_KEY`

### Storage Layout (Object Keys)

```
chunks/{file_id}/{chunk_index:08}
  e.g. chunks/550e8400-e29b-41d4-a716-446655440000/00000000
       chunks/550e8400-e29b-41d4-a716-446655440000/00000001

keys/{file_id}.envelope       -- Signal-sealed file key envelope
```

This layout enables parallel chunk download, range seeks, and prefix-based deletion.

---

## CLI (`fbx`)

A single statically-linked binary (`apps/cli`). Ships with no runtime dependencies.

### Commands

```
fbx auth register          Register a new FreeBox account
fbx auth login             Login and store credentials in the OS keychain
fbx auth logout            Remove stored credentials
fbx auth whoami            Display the currently logged-in user

fbx upload <file>          Upload file(s) with E2EE (progress bar, 8 parallel streams)
fbx download <id>          Download and decrypt a file
fbx ls [path]              List files (alias: fbx list)
fbx rm <file>              Delete a file — moves to trash (alias: fbx remove)
fbx sync <dir>             Two-way sync a local directory

fbx datasource list        List available storage data sources (alias: fbx provider)
fbx datasource add         Register a cloud or local storage data source
fbx datasource rm          Remove a configured data source

fbx bucket list            List all buckets for the configured storage credentials
fbx bucket create <name>   Create a new bucket

fbx msg send <user>        Send an encrypted message
fbx msg read               Read unread messages

fbx mail compose           Compose and send encrypted email
fbx mail read              Read inbox

fbx plugin install         Install a FreeBox plugin
fbx plugin list            List installed plugins
fbx plugin rm              Remove a plugin
```

### Token Refresh

The CLI's shared HTTP client (`apps/cli/src/client.rs`) transparently refreshes
the access token via the `send_with_refresh` helper whenever it receives an
HTTP 401. The new token pair is persisted to the session file automatically —
callers never need to handle token expiry manually.

### Configuration

Stored in `~/.config/freebox/config.toml` (TOML format). Includes the server
URL and the active `StorageSource`. The `--server` global flag or the
`FREEBOX_SERVER` environment variable override the config file value.

---

## Server Configuration

All values are read from environment variables (or a `.env` file in the working
directory, loaded automatically via `dotenvy`).

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | _(required)_ | PostgreSQL connection URL |
| `JWT_SECRET` | _(required)_ | HMAC secret for JWT signing |
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `8080` | Bind port |
| `DB_POOL_SIZE` | `5` | SQLx connection pool size |
| `JWT_ACCESS_TTL_SECS` | `900` | Access token TTL (15 min) |
| `JWT_REFRESH_TTL_SECS` | `2592000` | Refresh token TTL (30 days) |
| `RATE_LIMIT_REQUESTS` | `200` | Max requests per window per key |
| `RATE_LIMIT_WINDOW_SECS` | `60` | Rate limit window in seconds |
| `OAUTH_REACTIVATION_MAX_AGE_DAYS` | `30` | Max deletion age for OAuth reactivation (`0` = unlimited) |
| `ADMIN_USER_IDS` | _(empty)_ | Comma-separated UUIDs with admin access |
| `STORAGE_PROVIDER` | `local` | `local`, `s3`, or `r2` |
| `STORAGE_LOCAL_ROOT` | `./data` | Root path for local storage |
| `STORAGE_S3_BUCKET` | _(required for s3/r2)_ | S3/R2 bucket name |
| `STORAGE_S3_REGION` | `auto` | AWS region or `auto` for R2 |
| `STORAGE_S3_ENDPOINT` | _(optional)_ | Custom endpoint URL (required for R2/MinIO) |
| `STORAGE_S3_ACCESS_KEY_ID` | _(required for s3/r2)_ | S3 access key |
| `STORAGE_S3_SECRET_ACCESS_KEY` | _(required for s3/r2)_ | S3 secret key |
| `OAUTH_{PROVIDER}_CLIENT_ID` | _(optional)_ | OAuth2 client ID (enables provider) |
| `OAUTH_{PROVIDER}_CLIENT_SECRET` | _(optional)_ | OAuth2 client secret |
| `OAUTH_{PROVIDER}_REDIRECT_URI` | _(optional)_ | OAuth2 redirect URI |

OAuth providers (`PROVIDER`): `GITHUB`, `GOOGLE`, `MICROSOFT`, `APPLE`, `FACEBOOK`.
A provider is enabled only when all three of its vars are set.

---

## Deployment Architecture

### Minimal (single-server / self-hosted)

```
Internet
  │  HTTPS
  ▼
freebox-server (Axum + Tokio)
  ├── Rate limiter (in-process, per TCP peer)
  ├── PostgreSQL  (local or managed)
  └── Object storage (local filesystem / S3 / R2)
```

No reverse proxy or load balancer is required. The server is safe to expose
directly to the internet because:
- Rate limiting uses the real TCP peer IP (not spoofable headers).
- Upload body size is enforced at the framework layer.
- All secrets are server-side only; clients receive only encrypted blobs.

### Scaled (multi-replica)

```
                    ┌─────────────────────────────┐
                    │        Load Balancer         │
                    │    (nginx / AWS ALB)         │
                    └──────────────┬──────────────┘
                                   │
              ┌────────────────────┼────────────────────┐
              │                    │                    │
     ┌────────▼────────┐  ┌───────▼────────┐  ┌───────▼────────┐
     │  freebox-server │  │ freebox-server │  │ freebox-server │
     │   (Axum + Tokio)│  │  (replica 2)  │  │  (replica 3)  │
     └────────┬────────┘  └───────┬────────┘  └───────┬────────┘
              └────────────────────┼────────────────────┘
                                   │
              ┌────────────────────┼────────────────────┐
              │                    │                    │
     ┌────────▼──────┐   ┌────────▼──────┐   ┌────────▼──────┐
     │  PostgreSQL   │   │    Dragonfly  │   │  NATS JetStr. │
     │  (primary +   │   │  (cache +     │   │  (event bus)  │
     │   replicas)   │   │   sessions)   │   │               │
     └───────────────┘   └───────────────┘   └───────────────┘
                                   │
                    ┌──────────────▼──────────────┐
                    │       Object Storage         │
                    │  (S3 / R2 / MinIO / GCS)    │
                    └─────────────────────────────┘
```

> **Note:** In a multi-replica setup the in-process rate limiter is per-replica.
> For a shared rate limit across replicas, replace `RateLimiter` with a
> Redis/Dragonfly-backed implementation.

### Local Development

```sh
# Start PostgreSQL (data persists in the postgres-data Docker named volume)
docker compose -f infra/docker-compose.dev.yml up -d

# Apply migrations
cd apps/server
sqlx migrate run

# Run the server (reads from .env automatically)
# Background tasks start automatically: orphaned upload cleanup (hourly)
cargo run -p freebox-server

# Build the CLI
cargo build -p fbx
```

---

## ADR Index (Architecture Decision Records)

| ID | Decision | Status |
|----|----------|--------|
| ADR-001 | Use Rust for backend | Accepted |
| ADR-002 | Axum over Actix-web | Accepted |
| ADR-003 | Apache OpenDAL for storage abstraction | Accepted |
| ADR-004 | BLAKE3 over SHA-256 for content hashing | Accepted |
| ADR-005 | Dragonfly over Redis | Accepted |
| ADR-006 | NATS JetStream over Kafka | Accepted |
| ADR-007 | Signal Protocol for 1:1 E2EE | Accepted |
| ADR-008 | MLS RFC 9420 for group E2EE | Accepted |
| ADR-009 | Plugin system modelled after VS Code | Accepted |
| ADR-010 | Argon2id for password hashing | Accepted |
| ADR-011 | OAuth2 + PKCE for third-party auth (GitHub, Google, Microsoft, Apple, Facebook) | Accepted |
| ADR-012 | REST + WebSocket over GraphQL/gRPC for client-facing API | Accepted |
| ADR-013 | In-process token-bucket rate limiter using TCP peer IP (no proxy trust) | Accepted |
| ADR-014 | S3/R2 storage via SigV4 REST with auto-bucket creation | Accepted |
| ADR-015 | Persistent audit trail in PostgreSQL (account_audit_events) | Accepted |
| ADR-016 | Admin API gated on static UUID allowlist (ADMIN_USER_IDS) | Accepted |
