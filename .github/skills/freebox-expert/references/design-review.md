# FreeBox Design Review Checklist

A structured audit of FreeBox's architecture, security model, and implementation. Run this checklist when you want to find design flaws, security gaps, or improvement opportunities.

---

## How to Run This Review

For each section, use `grep_search` and `read_file` to verify each item against the actual code. Mark items as:
- ✅ **OK** — Correctly implemented
- ⚠️ **Partial** — Partially implemented, needs attention
- ❌ **Gap** — Not implemented or incorrectly implemented
- 🔍 **Verify** — Needs closer inspection of specific code path

---

## Section 1: Zero-Knowledge Invariant

The server must NEVER see plaintext data or private key material.

| Check | How to Verify | Status |
|-------|--------------|--------|
| File content encrypted before leaving client | `packages/crypto/src/encryption.rs` — `encrypt_chunk()` called in CLI before upload | 🔍 |
| FileKey sealed with Signal session key before upload | Look for envelope encryption in upload flow | 🔍 |
| BLAKE3 content hash computed client-side only | Grep for `blake3` in server code — should NOT appear in upload handlers | 🔍 |
| Server never logs plaintext filenames | Grep for `file_name` in server `tracing` calls | 🔍 |
| Private keys never transmitted | Grep for `secret_key`, `signing_key`, `master_secret` in `auth.rs` POST bodies | 🔍 |
| Salt endpoint open without auth | `/api/v1/auth/salt/:username` — intentionally public (needed for Argon2id client-side derivation) | ✅ |

**Known Gap**: Client-side file name encryption. The web app's `FilesPage.tsx` displays `file_name` labelled "(encrypted)" — but the upload path needs to verify the name is actually encrypted before being stored, and the download path must decrypt it client-side.

---

## Section 2: Cryptographic Correctness

| Check | File | Risk if Wrong |
|-------|------|--------------|
| AES-GCM nonce never reused for same key | `encryption.rs` — nonce derived from chunk index | Nonce reuse = full plaintext recovery |
| FileKey is per-file unique | `FileKey::generate()` called per file, not per user | Key reuse = cross-file XOR attack |
| Argon2id params meet OWASP 2023 | `keys.rs` — 64 MiB, 3 iter, 4 lanes | Weak params = brute-force feasible |
| Signed prekey signature verified in X3DH | `signal.rs` — `verify_signed_prekey()` called before any DH | MITM on key server substitutes prekey |
| BLAKE3 domain separation used | `signal::domains` module — separate constants per derivation | Domain confusion = key reuse across protocols |
| Secrets zeroized on drop | All secret types have `ZeroizeOnDrop` | Secrets linger in freed memory |
| One-time prekeys are single-use | Server deletes OPK after handing it to a peer | Replay attack if OPK reused |

**Known Gap — Chunk Count in Envelope**: The AES-GCM encrypted chunks don't include a total chunk count in authenticated metadata. An attacker (or a compromised server) could truncate a file by removing trailing chunks and the client would accept a shorter-than-expected file without detecting the attack.

**Fix**: Include `{ total_chunks: u32, file_id: uuid }` in the sealed FileKey envelope, and verify the count matches the chunk manifest on download.

**Known Gap — OPK Exhaustion Fallback**: When one-time prekeys are exhausted, the X3DH spec allows falling back to omitting DH4. This weakens per-session forward secrecy. FreeBox should refuse new session initiation when OPKs are exhausted and prompt the client to replenish.

---

## Section 3: Server Security (OWASP Top 10)

### A01 — Broken Access Control
| Check | Location | Status |
|-------|---------|--------|
| File operations require JWT auth | Axum middleware validates token before handlers | ✅ |
| User can only access own files | File handlers check `owner_id = claims.sub` | 🔍 Verify in upload/download handlers |
| Admin routes require admin claim | `ADMIN_USER_IDS` env-var check in admin handlers | ✅ |
| Chunk download validates ownership | `GET /files/:file_id/chunk/:n` — checks file ownership | 🔍 |

### A02 — Cryptographic Failures
| Check | Status |
|-------|--------|
| TLS 1.3 minimum enforced | Configured at deployment level (not in Axum code itself) — needs infra verification | 🔍 |
| JWT signed with strong secret | Check `JWT_SECRET` env var length requirement | 🔍 |
| Passwords not stored in plaintext | Auth uses Argon2id hash — raw password never persisted | ✅ |
| Sensitive fields not logged | `AppError::Internal` logs internally but returns generic message | ✅ |

### A03 — Injection
| Check | Status |
|-------|--------|
| SQL via parameterized queries (sqlx) | sqlx `query!` macro uses `$1` placeholders — no string interpolation | ✅ |
| File IDs validated as UUIDs | Route params parsed as `Uuid` type by Axum — rejects non-UUID strings | ✅ |
| File names not used in SQL concatenation | 🔍 Verify `file_name` handling in upload handler |

### A05 — Security Misconfiguration
| Check | Status |
|-------|--------|
| CORS configured, not wildcard `*` | `CorsLayer` in `main.rs` — verify origins are restricted | 🔍 |
| Default body size limit | `DefaultBodyLimit::max(32 * 1024 * 1024)` on upload route | ✅ |
| Rate limiting enabled | `RateLimiter` middleware applied globally | ✅ |
| Rate limiter uses peer IP, not `X-Forwarded-For` | `rate_limit.rs` — `ConnectInfo<SocketAddr>` used | ✅ |

### A07 — Authentication Failures
| Check | Status |
|-------|--------|
| JWT expiry enforced | Claims include `exp` field — verify validation | 🔍 |
| Refresh token rotation on use | Check `POST /auth/refresh` for token revocation before issuing new one | 🔍 |
| Logout revokes refresh token | `POST /auth/logout` must invalidate token in DB | 🔍 |
| OAuth PKCE used | `GET /auth/oauth/:provider` — `code_verifier` / `code_challenge` flow | ✅ |
| OAuth race condition handled | `INSERT … ON CONFLICT DO NOTHING` + fetch fallback | ✅ |

### A09 — Logging & Monitoring
| Check | Status |
|-------|--------|
| Structured access logs | `TraceLayer` with `tracing` | ✅ |
| Request ID for correlation | `SetRequestIdLayer` adds `X-Request-Id` | ✅ |
| Audit trail for auth events | `account_audit_events` table — login, register, reactivate | ✅ |
| Per-user audit log endpoint | `GET /api/v1/auth/audit-events` | ✅ |

---

## Section 4: Rate Limiter Design

**File**: `apps/server/src/rate_limit.rs`

Current implementation uses a sliding-window token bucket with `Mutex<HashMap<String, VecDeque<Instant>>>`.

| Issue | Severity | Detail |
|-------|----------|--------|
| **Global Mutex contention** | Medium | All requests contend on a single `tokio::sync::Mutex`. Under high concurrency, this becomes a bottleneck. Consider `DashMap` (concurrent HashMap) with per-bucket `Mutex`. |
| **In-process only** | Medium | Rate limits reset on server restart and don't apply across multiple instances. For a horizontally scaled deployment, this needs Redis/Dragonfly-backed rate limiting. |
| **LRU eviction is random** | Low | When `max_buckets` is reached, `evict_oldest_bucket` removes the bucket with the oldest single timestamp — not necessarily the least recently used. True LRU requires an ordered data structure (e.g. `linked-hash-map`). |
| **Bearer token hash exposes timing** | Low | The limiter keys authenticated requests by hashing the Bearer token. Ensure the hash function is consistent and the key comparison doesn't leak timing (use constant-time comparison if keys are compared). |

---

## Section 5: Plugin System

| Check | Status |
|-------|--------|
| Plugin manifest validated at load time | `PluginManifest` deserialized with serde — schema validation via `config_schema` | 🔍 Schema validation code not yet visible |
| Plugins can't call each other directly | Plugin communication only via `EventBus` — verify no direct plugin imports | 🔍 |
| WASM sandboxing for untrusted plugins | Documented as planned in `architecture.md` — not yet implemented | ⚠️ |
| Plugin can't access other plugins' config | `PluginContext` scoped to one plugin's config | ✅ |
| Plugin version compatibility checked | `api_version` SemVer range in manifest | 🔍 Verify kernel checks this |

---

## Section 6: Frontend Security

| Check | Status |
|-------|--------|
| Tokens in `localStorage` | ⚠️ XSS risk. Consider `HttpOnly` cookie for production. |
| Client-side decryption missing | ❌ Download flow delivers ciphertext. Crypto Web Worker needed. |
| File names displayed as ciphertext | ⚠️ UX gap. Decryption must be client-side. |
| No CSP header | 🔍 Check if `index.html` or server sets `Content-Security-Policy` |
| React Query `staleTime: 30_000` | ✅ Prevents excessive re-fetching |
| Error boundaries missing | ⚠️ Unhandled render errors crash the full app |

---

## Section 7: Architecture Strengths

These design decisions are sound and should be preserved:

1. **Single crypto crate**: All primitives in `packages/crypto` — one audit surface, no primitive scatter
2. **Trait-based plugin system**: Zero direct coupling between kernel and plugins — easily testable
3. **Peer IP for rate limiting**: `X-Forwarded-For` ignored — prevents spoofing when server is directly internet-facing
4. **`zeroize` on all secrets**: Memory safety for key material — prevents heap dump attacks
5. **Audit trail table**: `account_audit_events` provides accountability without compromising E2EE
6. **Chunked upload with chunk index in nonce**: Prevents chunk reordering attacks at the crypto layer
7. **BLAKE3 for deduplication hash**: Never sent to server — prevents hash-based content probing

---

## Prioritized Improvement Roadmap

### Critical (fix before production)
1. **Client-side decryption in web app** — current download delivers ciphertext
2. **Chunk count in file envelope** — prevents truncation attacks
3. **OPK exhaustion handling** — refuse session init, don't silently downgrade

### High
4. **Token storage hardening** — evaluate `HttpOnly` cookie vs `localStorage`
5. **WASM sandboxing for plugins** — required for untrusted third-party plugins
6. **Rate limiter horizontal scaling** — Dragonfly/Redis backend for multi-instance deployments

### Medium
7. **Signed prekey rotation schedule** — automated SPK rotation every 30-90 days
8. **Rate limiter concurrency** — replace global `Mutex<HashMap>` with `DashMap`
9. **React error boundaries** — prevent full-app crashes from component errors

### Low
10. **Upload progress in UI** — chunk-level progress reporting
11. **Next-page prefetch** — React Query prefetchQuery for smoother pagination
12. **CSP headers** — add `Content-Security-Policy` to harden against XSS
