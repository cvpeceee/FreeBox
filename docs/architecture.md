# FreeBox Architecture

## Overview

FreeBox is a **plugin-based platform** with a small, auditable Rust kernel.
Every capability — storage, messaging, email — is a plugin that implements
well-defined trait interfaces. The kernel provides auth, routing, and an event
bus; plugins provide all domain-specific functionality.

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
Auth Middleware
  │  Validates JWT Bearer token
  │  Injects Claims into request extensions
  ▼
Route Handler (e.g. files::upload_chunk)
  │  Pulls Claims, validates ownership
  │  Calls StorageProvider::put()
  ▼
Plugin Registry → StorageProvider (S3 / Local / GCS)
  │
  │  Publishes FileUploaded event
  ▼
Event Bus (NATS JetStream)
  │  Fan-out to subscribers
  ▼
Other Plugins (messaging, mail, sync notifications...)
```

---

## Database Schema

```sql
-- Core tables (PostgreSQL 17)

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

## Storage Layout (Object Keys)

Encrypted chunks are stored using deterministic, opaque keys:

```
chunks/{file_id}/{chunk_index:08}
  e.g. chunks/550e8400-e29b-41d4-a716-446655440000/00000000
       chunks/550e8400-e29b-41d4-a716-446655440000/00000001
```

The key envelope (sealed file key) is stored separately:

```
keys/{file_id}.envelope
```

This layout enables:
- Parallel chunk download (fetch all keys, stream in parallel)
- Range downloads (seek to chunk N directly)
- Efficient deletion (delete all keys matching prefix)

---

## Deployment Architecture

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
                    │  (S3 / GCS / MinIO / IPFS)  │
                    └─────────────────────────────┘
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
