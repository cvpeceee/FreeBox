# Server Middleware Stack

> Last synced: 2026-05-11

Middleware layers in `apps/server` — applied outermost-first on every request.

## Middleware Pipeline

```mermaid
flowchart TB
    REQ([Incoming HTTP Request])
    REQ --> RID

    subgraph "Layer 1: Observability"
        RID["SetRequestIdLayer\n→ X-Request-Id: UUID"]
        TRACE["TraceLayer\n→ Structured JSON access logs"]
    end

    subgraph "Layer 2: Transport"
        COMP["CompressionLayer\n→ Gzip / Brotli responses"]
        CORS["CorsLayer\n→ allow_origin: Any (dev)\n→ restrict in prod"]
    end

    subgraph "Layer 3: Protection"
        RL["RateLimiter middleware\n→ Token-bucket per bearer hash or peer IP\n→ 200 req / 60s default\n→ 100K max buckets"]
    end

    subgraph "Layer 4: Auth (authenticated routes only)"
        AUTH["require_auth middleware\n→ Extract Bearer JWT\n→ Validate HS256 signature + expiry\n→ Inject Claims into request extensions"]
    end

    RID --> TRACE --> COMP --> CORS --> RL
    RL --> PUBLIC[Public Route Handler]
    RL --> AUTH --> AUTHED[Authenticated Route Handler]

    PUBLIC --> RES([HTTP Response])
    AUTHED --> RES
```

## Rate Limiter Design

```mermaid
classDiagram
    class RateLimiter {
        -max_requests: usize
        -window: Duration
        -max_buckets: usize
        -buckets: Mutex~HashMap~
        +new(max_requests, window) Self
        +with_max_buckets(max, window, cap) Self
        +allow(key: str) bool
        -prune_expired(buckets, now)
        -evict_oldest_bucket(buckets)
    }

    note for RateLimiter "Algorithm: Sliding window token bucket\nKey selection priority:\n1. SHA hash of Bearer token (authenticated)\n2. TCP peer IP via ConnectInfo (unauthenticated)\n3. Anonymous fallback bucket\n\nSecurity: X-Forwarded-For is IGNORED"
```

## Security Properties

| Layer | What it does | Why |
|-------|-------------|-----|
| `SetRequestIdLayer` | Adds `X-Request-Id: UUID` | Log correlation across services |
| `TraceLayer` | JSON-formatted access logs | Observability (Loki, Datadog) |
| `CompressionLayer` | Gzip/Brotli response body | Bandwidth reduction |
| `CorsLayer` | Cross-origin access control | Browser security boundary |
| `RateLimiter` | Per-token/IP request throttling | DoS protection |
| `DefaultBodyLimit` | 32 MiB max on upload route | Memory exhaustion prevention |
| `require_auth` | JWT validation + Claims injection | Authentication |
| `ConnectInfo<SocketAddr>` | TCP peer IP (not headers) | Prevent IP spoofing |
