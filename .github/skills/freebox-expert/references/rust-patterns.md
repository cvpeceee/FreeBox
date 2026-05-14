# FreeBox Rust Patterns Deep-Dive

This reference teaches Rust concepts using actual FreeBox code as examples.

---

## 1. Trait Objects — `dyn StorageProvider`

**File**: `packages/core/src/storage.rs`

### The Pattern
```rust
// The trait — defines the contract any storage backend must fulfil
#[async_trait]
pub trait StorageProvider: Send + Sync {
    async fn put(&self, key: &str, data: Bytes) -> Result<ObjectMeta>;
    async fn get(&self, key: &str) -> Result<Bytes>;
    async fn delete(&self, key: &str) -> Result<()>;
    fn capabilities(&self) -> StorageCapabilities;
    // ...
}
```

The kernel stores backends as `Arc<dyn StorageProvider>` — a **trait object** (fat pointer: vtable + data pointer). This lets the kernel swap S3 for local disk without changing any calling code.

### Why `Send + Sync`?
- `Send`: The trait object can be moved between threads (required by Tokio's async runtime)
- `Sync`: The trait object can be shared by reference across threads simultaneously (`Arc<T>` requires `T: Sync`)

Without these bounds, the compiler rejects `Arc<dyn StorageProvider>`.

### Why `async_trait`?
Rust traits cannot have `async fn` methods natively (they require `impl Future` in the return type, which can't be object-safe). The `async_trait` macro desugars them to `-> Box<dyn Future<...> + Send + '_>`, making them compatible with trait objects.

**Teaching point**: Object safety rules — a trait method is NOT object-safe if:
- It returns `Self`
- It has generic type parameters (`fn foo<T>(&self, t: T)`)
- It's `async fn` without `async_trait`

---

## 2. Plugin System — Dependency Inversion

**File**: `packages/core/src/plugin.rs`

### The Pattern
```rust
pub struct PluginContext {
    pub event_bus: Arc<dyn EventBus>,        // kernel service
    pub config: HashMap<String, Value>,      // plugin-specific config
    pub instance_id: Uuid,
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> &PluginManifest;
    async fn on_load(&mut self, ctx: PluginContext) -> Result<()>;
    async fn on_unload(&mut self) -> Result<()>;
}
```

The `PluginContext` is injected at `on_load` time — this is classic **dependency injection** (DI). The plugin never imports the kernel crate; it only imports `freebox-core` (the trait contracts). This means:
1. Plugins are independently compilable and testable
2. The kernel can be replaced without touching plugins
3. Circular dependencies are impossible

### Manifest Validation
```toml
# plugins/storage-s3/plugin.toml
[plugin]
id = "storage-s3"
api_version = "^1.0"
[plugin.capabilities]
provides = ["storage.provider"]
```

At load time, the kernel checks `api_version` (semver range), then validates plugin config against `config_schema` (JSON Schema). This replicates VS Code's extension manifest validation pattern.

---

## 3. Shared State — `Arc<AppState>`

**File**: `apps/server/src/state.rs`

### The Pattern
```rust
pub struct AppState {
    pub db: PgPool,
    pub storage: Arc<dyn StorageProvider>,
    pub config: Config,
    // ...
}
```

Axum handlers receive `State(state): State<Arc<AppState>>`. The `Arc` allows many concurrent handlers to hold a reference to the same state without copying it.

### Why `Arc` not `Mutex<AppState>`?
`AppState` itself is immutable after construction (`db`, `config` are read-only at runtime). `PgPool` and `Arc<dyn StorageProvider>` manage their own internal concurrency. So `Arc` alone suffices — no outer `Mutex` needed.

**Teaching point**: `Arc<Mutex<T>>` is the Rust pattern for shared mutable state. But prefer making state immutable where possible — it's cheaper (no lock contention) and simpler.

---

## 4. Error Handling — `thiserror`

**File**: `apps/server/src/error.rs`

### The Pattern
```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}
```

`#[from]` generates `impl From<sqlx::Error> for AppError`, so `?` operator works seamlessly:
```rust
let user = db.find_user(id).await?;  // sqlx::Error auto-converted to AppError
```

### Axum Integration
`AppError` implements `IntoResponse` to map each variant to the right HTTP status code:
```rust
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg).into_response(),
            Self::Unauthorized    => StatusCode::UNAUTHORIZED.into_response(),
            Self::Database(_)     => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}
```

**Teaching point**: Never expose internal error details (e.g., SQL error messages) to the client. The `Database(_)` arm returns only a 500, not the actual `sqlx::Error` message which might contain table names or query structure.

---

## 5. `zeroize` — Memory Safety for Secrets

**File**: `packages/crypto/src/keys.rs`

### The Problem
When a Rust value is dropped, its memory is marked as "free" but the bytes aren't overwritten. A heap dump or memory scanner could find your secret key bytes sitting in deallocated memory.

### The Solution
```rust
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct FileKey([u8; 32]);
```

`ZeroizeOnDrop` generates:
```rust
impl Drop for FileKey {
    fn drop(&mut self) {
        self.0.zeroize(); // writes 0x00 to all 32 bytes before deallocation
    }
}
```

### Critical: Compiler Optimization
The compiler is allowed to optimize away "dead writes" — including zeroing bytes that are never read again. `zeroize` uses platform-specific memory barriers (`std::sync::atomic::compiler_fence`) to prevent this optimization.

**Teaching point**: `ptr::write_volatile` is another approach. `zeroize` is preferred because it's audited, cross-platform, and handles edge cases (e.g., compiler intrinsics that could skip the write).

---

## 6. Axum Middleware Composition — Tower Layers

**File**: `apps/server/src/main.rs`

### The Pattern
```rust
let app = Router::new()
    .route("/api/v1/files", get(files::list))
    .layer(SetRequestIdLayer::new(...))  // outermost — runs first on request
    .layer(TraceLayer::new_for_http())
    .layer(CompressionLayer::new())
    .layer(CorsLayer::permissive())
    .layer(middleware::from_fn_with_state(state.clone(), rate_limit));
    // innermost — runs last on request, first on response
```

Layers wrap in reverse order — the last `.layer()` call is **innermost** (closest to the handler). Request flows **downward** through layers; response flows **upward** (like a stack of middleware in Express.js but type-safe).

### `middleware::from_fn_with_state`
Lets you write an async function as middleware:
```rust
async fn rate_limit(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    if state.limiter.check(&req).is_err() {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    next.run(req).await
}
```

**Teaching point**: Tower's `Service` trait is the abstraction underneath all Axum middleware. Each `layer` wraps the router in another `Service`. This is the **Decorator pattern** expressed as a type system.

---

## 7. Async Rust — Key Concepts

### Why Async?
Network I/O (DB queries, HTTP upstream calls) spends most time waiting. Async lets one OS thread handle thousands of concurrent waits without blocking.

### Tokio Runtime
FreeBox uses `#[tokio::main]` which starts a multi-threaded scheduler. Each `.await` point is a suspension point — the task yields the thread to other tasks.

### `Arc` vs `Rc` in Async Code
- `Rc` is NOT `Send` — cannot be moved across threads
- `Arc` IS `Send + Sync` — safe for multi-threaded async
- Rule: Always use `Arc` in Axum handlers (they run on any Tokio worker thread)

### Lifetime Errors with Async
Common error: `future cannot be sent between threads safely` — usually means a non-`Send` type (like `Rc` or `MutexGuard`) is held across an `.await` point.

Fix: Drop the guard before `.await`:
```rust
// BAD
let guard = mutex.lock().unwrap();
some_async_fn().await;  // guard still alive here — not Send
drop(guard);

// GOOD
let value = {
    let guard = mutex.lock().unwrap();
    guard.clone()  // copy the value out, guard drops here
};
some_async_fn().await;  // guard is already dropped
```

---

## 8. Rust Ownership in the Plugin API

### `PluginContext` is Moved, Not Cloned
```rust
async fn on_load(&mut self, ctx: PluginContext) -> Result<()>;
```

`ctx` is **moved** into `on_load` — after the call, the kernel no longer owns it. The plugin owns its context for the rest of its lifetime. This transfer of ownership is enforced at compile time — no runtime reference counting needed.

### `&mut self` on Plugin Lifecycle Methods
`on_load` and `on_unload` take `&mut self` — exclusive mutable access. This prevents two lifecycle methods from running concurrently on the same plugin instance, which is the correct behavior (you can't be loading and unloading simultaneously).

---

## Common Rust Mistakes to Avoid (FreeBox Context)

| Mistake | Where it would hurt | Fix |
|---------|--------------------|----|
| Cloning `FileKey` | `crypto/encryption.rs` | `FileKey` intentionally not `Clone` — each key should encrypt exactly one file |
| Holding `MutexGuard` across `.await` | Any async handler | Extract value before `.await` |
| Using `unwrap()` in production paths | Server handlers | Use `?` with `AppError` |
| Reusing a nonce | AES-GCM encryption | FreeBox derives nonce from chunk index — never reuse |
| `Arc<Mutex<Vec<...>>>` when `DashMap` suffices | Rate limiter | Use lock-free concurrent collections when appropriate |
