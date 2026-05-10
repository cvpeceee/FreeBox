-- Migration: 003_account_audit_events.sql
-- Persistent account-level audit events.

CREATE TABLE account_audit_events (
    id               UUID        NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    user_id          UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_type       TEXT        NOT NULL,
    source           TEXT        NOT NULL,
    provider         TEXT,
    provider_user_id TEXT,
    details          JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_account_audit_events_user_created_at
    ON account_audit_events (user_id, created_at DESC);

CREATE INDEX idx_account_audit_events_event_type_created_at
    ON account_audit_events (event_type, created_at DESC);
