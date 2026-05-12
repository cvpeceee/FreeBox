# Server API

> Last synced: 2026-05-11

Class diagram for `apps/server` — state, config, error, and route assembly.

## AppState & Dependencies

```mermaid
classDiagram
    direction TB

    class AppState {
        +config: Arc~Config~
        +db: PgPool
        +storage: Arc~dyn StorageProvider~
        +http_client: reqwest::Client
        +oauth: OAuthConfig
        +rate_limiter: SharedRateLimiter
        +new(config) AppState$ 
    }

    class Config {
        +host: String
        +port: u16
        +database_url: String
        +db_pool_size: u32
        +redis_url: String
        +rate_limit_requests: u32
        +rate_limit_window_secs: u64
        +oauth_reactivation_max_age_days: u32
        +admin_user_ids: Vec~Uuid~
        +jwt_secret: String
        +jwt_access_ttl_secs: u64
        +jwt_refresh_ttl_secs: u64
        +storage_provider: String
        +storage_local_root: String
        +storage_s3_bucket: String
        +storage_s3_region: String
        +storage_s3_endpoint: String
        +storage_s3_access_key: String
        +storage_s3_secret_key: String
        +argon2_memory_kib: u32
        +argon2_iterations: u32
        +argon2_parallelism: u32
        +oauth: OAuthConfig
        +from_env() Result~Config~$
    }

    class OAuthConfig {
        +github: Option~OAuthProviderConfig~
        +google: Option~OAuthProviderConfig~
        +microsoft: Option~OAuthProviderConfig~
        +apple: Option~OAuthProviderConfig~
        +facebook: Option~OAuthProviderConfig~
        +from_env() OAuthConfig$
        +get() Option~OAuthProviderConfig~
    }

    class OAuthProviderConfig {
        +client_id: String
        +client_secret: String
        +redirect_uri: String
    }

    class AppError {
        <<enum>>
        NotFound(String)
        Unauthorized(String)
        Forbidden(String)
        BadRequest(String)
        Conflict(String)
        RateLimited(String)
        PayloadTooLarge(String)
        Internal(anyhow::Error)
    }

    class Claims {
        +sub: Uuid
        +username: String
        +exp: usize
        +iat: usize
        +new(user_id, ttl_secs) Claims$
    }

    class RateLimiter {
        -max_requests: usize
        -window: Duration
        -max_buckets: usize
        -buckets: Mutex~HashMap~String, VecDeque~Instant~~~
        +new(max_requests, window) RateLimiter$
        +allow(key) bool
    }

    AppState --> Config : holds Arc
    AppState --> OAuthConfig : holds
    AppState --> RateLimiter : holds Arc
    Config --> OAuthConfig : contains
    OAuthConfig --> OAuthProviderConfig : 0..5 providers
    AppError ..|> IntoResponse : Axum impl
    AppError ..> `freebox_core::Error` : From impl
```

## Route Assembly (api/mod.rs)

```mermaid
flowchart TB
    subgraph Middleware Stack
        RID[SetRequestIdLayer<br/>X-Request-Id]
        TRACE[TraceLayer<br/>structured access logs]
        COMP[CompressionLayer<br/>Gzip / Brotli]
        CORS[CorsLayer]
        RL[RateLimiter middleware]
    end

    subgraph Public Routes
        HEALTH["GET /health"]
        REG["POST /auth/register"]
        LOGIN["POST /auth/login"]
        REFRESH["POST /auth/refresh"]
        SALT["GET /auth/salt/:username"]
        OAUTH_CFG["GET /auth/oauth/configured"]
        OAUTH_INIT["GET /auth/oauth/:provider"]
        OAUTH_CB["GET /auth/oauth/:provider/callback"]
        DS["GET /storage/datasources"]
    end

    subgraph Auth Middleware
        AUTH["require_auth<br/>JWT validation → Claims"]
    end

    subgraph Authenticated Routes
        LOGOUT["POST /auth/logout"]
        PROVIDERS["GET /auth/providers"]
        AUDIT["GET /auth/audit-events"]
        ADMIN_AUDIT["GET /admin/audit-events"]
        LINK["POST /auth/oauth/:provider/link"]
        UNLINK["DELETE /auth/oauth/:provider/unlink"]
        KEYS_GET["GET /keys/:user_id"]
        KEYS_OTP["POST /keys/one-time"]
        UP_INIT["POST /files/upload/init"]
        UP_CHUNK["PUT /files/upload/:upload_id"]
        UP_DONE["POST /files/upload/:id/complete"]
        LS_FILES["GET /files"]
        LS_TRASH["GET /files/trash"]
        GET_FILE["GET /files/:file_id"]
        RENAME["PATCH /files/:file_id"]
        DEL_FILE["DELETE /files/:file_id"]
        DL_CHUNK["GET /files/:file_id/chunk/:n"]
        RESTORE["POST /files/:file_id/restore"]
        BUCKETS_LS["GET /storage/buckets"]
        BUCKETS_CR["POST /storage/buckets"]
    end

    RID --> TRACE --> COMP --> CORS --> RL

    RL --> Public Routes
    RL --> AUTH --> Authenticated Routes
```
