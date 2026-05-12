# Guardian Report — 2026-05-11

## Diagrams Created

- [x] [system-overview.md](diagrams/system-overview.md) — crate dependency graph + file upload/download sequence diagrams
- [x] [core-traits.md](diagrams/core-traits.md) — `Plugin`, `StorageProvider`, `EventBus` class diagram + implementors
- [x] [crypto-types.md](diagrams/crypto-types.md) — key hierarchy, encryption pipeline, X3DH flow, domain constants
- [x] [server-api.md](diagrams/server-api.md) — `AppState`, `Config`, `AppError`, `Claims`, route assembly flowchart
- [x] [server-auth.md](diagrams/server-auth.md) — password auth, OAuth2, JWT claims
- [x] [server-files.md](diagrams/server-files.md) — upload protocol state machine, file lifecycle, handler→storage chain
- [x] [server-middleware.md](diagrams/server-middleware.md) — middleware pipeline, rate limiter design
- [x] [cli-commands.md](diagrams/cli-commands.md) — command tree, session/config types, upload flow
- [x] [cli-auth-flow.md](diagrams/cli-auth-flow.md) — login, auto-refresh, registration sequence
- [x] [plugin-lifecycle.md](diagrams/plugin-lifecycle.md) — lifecycle state machine, discovery, event flow
- [x] [web-components.md](diagrams/web-components.md) — component tree, Zustand store, API client, TS types

---

## Documentation Sync

| Document | Status | Drift Found |
|----------|--------|-------------|
| `README.md` | :warning: **Drift** | States web app is "Next.js 15 (App Router, React 19)" but actual code is a **React + Vite SPA** (no Next.js). Repository structure shows `apps/web/` as `Next.js 15` — inaccurate. Tech stack table lists "Next.js 15" — should say "React 18/19 + Vite". |
| `README.md` | :warning: **Drift** | States `STORAGE_S3_ACCESS_KEY_ID` / `STORAGE_S3_SECRET_ACCESS_KEY` but code uses `STORAGE_S3_ACCESS_KEY` / `STORAGE_S3_SECRET_KEY` (no `_ID` / `_ACCESS_` suffixes). |
| `docs/architecture.md` | :white_check_mark: In sync | API routes, DB schema, security hardening all match code. |
| `docs/testing.md` | :white_check_mark: In sync | Coverage map matches actual test file locations. |
| `docs/ui-architecture.md` | :white_check_mark: In sync | Correctly describes the React SPA + Vite setup. |

---

## Missing Tests

| Priority | Module | What's Missing |
|----------|--------|---------------|
| **CRITICAL** | `packages/crypto/src/signal.rs` | No test for `x3dh_respond()` — only `x3dh_initiate()` is tested. The responder function is the other half of the handshake. |
| **HIGH** | `plugins/storage-s3/src/sigv4.rs` | No tests for SigV4 signing. This is security-critical (AWS authentication). Test against known test vectors from AWS docs. |
| **HIGH** | `plugins/storage-s3/src/bucket.rs` | No unit tests for `create_bucket()` / `list_buckets()` / XML parsing. |
| **HIGH** | `plugins/storage-local/src/lib.rs` | No tests at all for `LocalPlugin`. Missing: `put`/`get`/`delete`/`list` round-trip, multipart, `on_load` config validation. |
| **HIGH** | `apps/server/src/api/oauth.rs` | Large module (~1500 lines) with tests, but no test for the OAuth **link** / **unlink** / **reactivation** flows. |
| **MEDIUM** | `apps/server/src/rate_limit.rs` | Has tests for basic allow/deny, but no test for bucket eviction (`max_buckets` exceeded) or concurrent access. |
| **MEDIUM** | `apps/server/src/api/storage.rs` | Has tests, but no test for `create_bucket()` handler (only `list_datasources` and `list_buckets`). |
| **MEDIUM** | `apps/cli/src/commands/sync.rs` | No tests. Sync is a complex feature (direction, watch mode). |
| **MEDIUM** | `apps/cli/src/commands/msg.rs` | No tests for messaging commands. |
| **MEDIUM** | `apps/cli/src/commands/mail.rs` | No tests for mail commands. |
| **MEDIUM** | `apps/cli/src/commands/list.rs` | No tests for `fbx ls` output formatting. |
| **LOW** | `apps/cli/src/commands/remove.rs` | No tests for `fbx rm`. |
| **LOW** | `apps/cli/src/commands/plugin.rs` | No tests for plugin management commands. |
| **LOW** | `packages/core/src/storage.rs` | No tests for `StorageCapabilities` defaults or `ObjectMeta` serialization. |

---

## Tech Recommendations

| Area | Current | Suggestion | Rationale |
|------|---------|------------|-----------|
| Web client hashing | PBKDF2 via Web Crypto API | Argon2id via WASM (`argon2-browser` or custom WASM build) | The code comments already note PBKDF2 is a temporary workaround. PBKDF2 is weaker than Argon2id for password hashing (not memory-hard). |
| S3 bucket ops | Custom SigV4 + reqwest | `aws-sdk-s3` crate for bucket management | The custom SigV4 implementation covers a minimal subset. `aws-sdk-s3` is officially maintained, handles edge cases (chunked signing, retries, regional endpoints), and reduces audit surface. |
| CORS config | `CorsLayer::new().allow_origin(Any)` | Restrict origins in production | `allow_origin(Any)` is fine for dev, but production should whitelist the frontend domain. Add `CORS_ALLOWED_ORIGINS` env var. |
| Event Bus | `NoopEventBus` used at runtime | Implement a real `EventBus` | The architecture docs describe NATS JetStream, but the actual code only uses `NoopEventBus`. Events (e.g., `FileUploaded`) are never published or consumed. |
| Cache | `redis_url` in Config | Not used anywhere | `Config` has a `redis_url` field, but no code reads from or writes to Redis/Dragonfly. Either implement caching or remove the field. |

---

## Architectural Gaps

- **`NoopEventBus` everywhere**: The `EventBus` trait and its typed extensions are well-designed, but the only implementation is `NoopEventBus` (a no-op). No events are published or subscribed to at runtime. The plugin communication backbone described in the architecture docs does not exist yet.

- **No plugin discovery from filesystem**: The architecture describes plugins being "discovered from the `plugins/` directory on startup," but `main.rs` hard-codes `S3Plugin::new()` and `LocalPlugin::new()` based on the `STORAGE_PROVIDER` env var. There is no dynamic plugin discovery, no `plugin.toml` loading, no `PluginRegistry`.

- **`PluginManifest` constructed inline**: Both `S3Plugin` and `LocalPlugin` hard-code their manifest in `new()` rather than loading from `plugin.toml`. The `plugin.toml` files exist in the plugin directories but are never read.

- **`as_storage_provider()` never called**: The `Plugin` trait has `is_storage_provider()` and `as_storage_provider()` methods, but the server constructs the `StorageProvider` directly by casting the plugin `Arc` rather than going through the trait method.

- **No WebSocket routes**: The architecture and README mention `WS /ws/sync` and `WS /ws/messaging` routes for real-time features. These do not exist in the code.

- **Missing `ensure_bucket_exists`**: The `bucket.rs` module exports `ensure_bucket_exists` but the S3 plugin's `on_load` does not call it. The architecture doc says "auto-creates the bucket on first use" — this is not implemented.

---

## Code Quality

- **Production `.unwrap()` calls** (7 total):
  - `apps/server/src/main.rs:277,283` — `.expect()` on signal handler install. Acceptable (process would be broken anyway).
  - `apps/server/src/api/oauth.rs:389` — `.unwrap()` on `link_user_id`. Should use `ok_or(AppError::BadRequest(...))`.
  - `apps/cli/src/commands/download.rs:107` — `.unwrap()` on `ProgressStyle`. Low risk (static template).
  - `apps/cli/src/commands/upload.rs:113` — `.unwrap()` on `ProgressStyle`. Low risk (static template).
  - `apps/cli/src/commands/provider.rs:264` — `.unwrap()` on `sources.last()` after `push()`. Safe but brittle.
  - `plugins/storage-s3/src/sigv4.rs:17` — `.expect("HMAC accepts any key length")`. Safe (documented invariant).

- **README says "Next.js 15"** for the web frontend, but the actual implementation is **React + Vite** (no Next.js). The `apps/web/package.json`, `vite.config.ts`, and `App.tsx` all confirm this is a plain SPA.

- **`DerivedKeys` struct** is exported from `packages/crypto/src/keys.rs` and re-exported from `lib.rs`, but the struct definition and `derive_keys_from_password` function are only used in tests within the crypto crate itself. The CLI auth module has its own separate key derivation flow.

- **`MasterSecret` in keys.rs** — the `SignedPrekey::generate()` method uses `.unwrap_or_default()` on `SystemTime::now().duration_since(UNIX_EPOCH)`, which silently returns 0 if the clock is before epoch. This is unlikely but could mask issues on embedded/misconfigured systems.
