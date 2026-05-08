-- Migration: 001_initial_schema.sql
-- FreeBox initial database schema
-- Run with: sqlx migrate run

-- Enable UUID generation
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- ---------------------------------------------------------------------------
-- Users
-- ---------------------------------------------------------------------------
CREATE TABLE users (
    id            UUID        NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    username      TEXT        NOT NULL,
    email         TEXT        NOT NULL,
    -- Argon2id hash of the password. NEVER stores the raw password.
    password_hash TEXT        NOT NULL,
    -- Argon2id salt — stored so the client can re-derive its master key.
    -- Not secret, but unique per user.
    argon2_salt   TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Soft delete — retains audit trail. NULL = active account.
    deleted_at    TIMESTAMPTZ,

    CONSTRAINT users_username_unique UNIQUE (username),
    CONSTRAINT users_email_unique    UNIQUE (email)
);

CREATE INDEX idx_users_username ON users (username) WHERE deleted_at IS NULL;
CREATE INDEX idx_users_email    ON users (email)    WHERE deleted_at IS NULL;

-- ---------------------------------------------------------------------------
-- Signal Protocol prekey bundles (public key material only)
-- ---------------------------------------------------------------------------
CREATE TABLE prekey_bundles (
    user_id    UUID        NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    -- JSONB payload: { identity_key, signed_prekey, one_time_prekeys[] }
    -- All public key material. No secret keys are stored server-side.
    bundle     JSONB       NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- JWT refresh tokens (opaque random strings, NOT JWTs)
-- ---------------------------------------------------------------------------
CREATE TABLE refresh_tokens (
    token      TEXT        NOT NULL PRIMARY KEY,
    user_id    UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    username   TEXT        NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Background job deletes expired tokens daily.
CREATE INDEX idx_refresh_tokens_user_id    ON refresh_tokens (user_id);
CREATE INDEX idx_refresh_tokens_expires_at ON refresh_tokens (expires_at);

-- ---------------------------------------------------------------------------
-- In-progress chunked uploads (staging area)
-- ---------------------------------------------------------------------------
CREATE TABLE uploads (
    id                     UUID        NOT NULL PRIMARY KEY,
    user_id                UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    total_chunks           INT         NOT NULL CHECK (total_chunks > 0),
    chunks_received        INT         NOT NULL DEFAULT 0 CHECK (chunks_received >= 0),
    size_bytes             BIGINT      NOT NULL CHECK (size_bytes >= 0),
    -- Sealed file encryption key — only the owner can unwrap with Signal key.
    encrypted_key_envelope TEXT        NOT NULL,
    -- BLAKE3 of the plaintext file — used for deduplication (hash is safe to store).
    content_hash           TEXT        NOT NULL,
    -- AES-GCM encrypted file name — server cannot read file names.
    encrypted_name         TEXT        NOT NULL,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Orphaned uploads (no activity for 24h) are cleaned up by a background job.
CREATE INDEX idx_uploads_user_id    ON uploads (user_id);
CREATE INDEX idx_uploads_created_at ON uploads (created_at);

-- ---------------------------------------------------------------------------
-- Completed files
-- ---------------------------------------------------------------------------
CREATE TABLE files (
    id                     UUID        NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    user_id                UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    total_chunks           INT         NOT NULL CHECK (total_chunks > 0),
    size_bytes             BIGINT      NOT NULL CHECK (size_bytes >= 0),
    encrypted_key_envelope TEXT        NOT NULL,
    content_hash           TEXT        NOT NULL,
    encrypted_name         TEXT        NOT NULL,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Moved to trash. NULL = active. Background job hard-deletes after 30 days.
    deleted_at             TIMESTAMPTZ
);

CREATE INDEX idx_files_user_id    ON files (user_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_files_created_at ON files (created_at);
CREATE INDEX idx_files_hash       ON files (content_hash, user_id);
