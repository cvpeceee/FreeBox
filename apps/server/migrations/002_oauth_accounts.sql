-- Migration: 002_oauth_accounts.sql
-- Adds OAuth2 third-party authentication support (GitHub, Google, Microsoft, Apple, Facebook).

-- ---------------------------------------------------------------------------
-- Allow OAuth-only users who have no password
-- ---------------------------------------------------------------------------
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;
ALTER TABLE users ALTER COLUMN argon2_salt   DROP NOT NULL;

-- ---------------------------------------------------------------------------
-- OAuth linked accounts
-- ---------------------------------------------------------------------------
-- Each row represents one OAuth provider linked to a FreeBox user account.
-- A user can link multiple providers, and each provider identity can only
-- be linked to one FreeBox account.
CREATE TABLE oauth_accounts (
    id                UUID        NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    user_id           UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Provider name: 'github', 'google', 'microsoft', 'apple', 'facebook'
    provider          TEXT        NOT NULL,
    -- The user's unique ID at the OAuth provider (e.g., GitHub user ID, Google sub).
    provider_user_id  TEXT        NOT NULL,
    -- Email address from the provider's profile (may differ from users.email).
    provider_email    TEXT,
    -- Display name from the provider's profile.
    provider_username TEXT,
    -- Avatar URL from the provider's profile.
    avatar_url        TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- One FreeBox account per provider identity.
    CONSTRAINT oauth_accounts_provider_unique UNIQUE (provider, provider_user_id)
);

-- Fast lookup by user_id (e.g., "list all linked providers for this user").
CREATE INDEX idx_oauth_accounts_user_id ON oauth_accounts (user_id);

-- ---------------------------------------------------------------------------
-- OAuth CSRF + PKCE state (short-lived, cleaned up by background job)
-- ---------------------------------------------------------------------------
-- Stores the PKCE verifier and CSRF state token between the initiate and
-- callback phases of the OAuth flow. Rows expire after 5 minutes.
CREATE TABLE oauth_states (
    state         TEXT        NOT NULL PRIMARY KEY,
    pkce_verifier TEXT        NOT NULL,
    provider      TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at    TIMESTAMPTZ NOT NULL
);

-- Background job deletes expired states periodically.
CREATE INDEX idx_oauth_states_expires_at ON oauth_states (expires_at);
