# Rust for FreeBox — A Beginner's Guide

> This guide assumes zero Rust experience. It explains every Rust concept used
> in FreeBox with simple analogies, real examples from the codebase, and
> exercises to build understanding.

---

## Table of Contents

1. [Why Rust?](#1-why-rust)
2. [Setting Up](#2-setting-up)
3. [Variables and Types](#3-variables-and-types)
4. [Ownership — Rust's Superpower](#4-ownership--rusts-superpower)
5. [Borrowing and References](#5-borrowing-and-references)
6. [Structs — Building Blocks](#6-structs--building-blocks)
7. [Enums — Multiple Choices](#7-enums--multiple-choices)
8. [Traits — Shared Behavior](#8-traits--shared-behavior)
9. [Error Handling — No Exceptions](#9-error-handling--no-exceptions)
10. [Option — Nullable Done Right](#10-option--nullable-done-right)
11. [Generics — Write Once, Use for Any Type](#11-generics--write-once-use-for-any-type)
12. [Async/Await — Doing Many Things at Once](#12-asyncawait--doing-many-things-at-once)
13. [Modules and Crates — Organizing Code](#13-modules-and-crates--organizing-code)
14. [Smart Pointers — Arc, Box, Mutex](#14-smart-pointers--arc-box-mutex)
15. [Closures — Functions as Values](#15-closures--functions-as-values)
16. [Derive Macros — Auto-Generated Code](#16-derive-macros--auto-generated-code)
17. [Testing](#17-testing)
18. [Cargo — The Build Tool](#18-cargo--the-build-tool)
19. [FreeBox Concept Map](#19-freebox-concept-map)
20. [Recommended Learning Path](#20-recommended-learning-path)

---

## 1. Why Rust?

Rust gives you three things that usually require picking two:

| Property | Java/Go | Python/JS | C/C++ | **Rust** |
|---|---|---|---|---|
| Fast (no garbage collector) | No | No | Yes | **Yes** |
| Memory safe (no crashes) | Yes | Yes | No | **Yes** |
| Concurrent (no data races) | Partially | No | No | **Yes** |

For FreeBox, this means:
- **Performance**: File encryption at 3+ GB/s (hardware AES-NI)
- **Security**: The compiler catches memory bugs before they become vulnerabilities
- **Reliability**: If it compiles, it almost certainly won't crash at runtime

---

## 2. Setting Up

```bash
# Install Rust (includes `cargo`, the build tool)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Verify installation
rustc --version    # the compiler
cargo --version    # the build tool (like npm for Rust)

# Build FreeBox
cd freebox
cargo build        # compiles everything
cargo test         # runs all tests
cargo run -p freebox-server  # runs the server
```

---

## 3. Variables and Types

### Immutable by default

In Rust, variables cannot be changed after creation unless you say `mut`:

```rust
let name = "FreeBox";         // immutable — cannot be changed
let mut counter = 0;          // mutable — can be changed
counter += 1;                 // OK
// name = "Other";            // ERROR: `name` is not mutable
```

**Why?** Immutable data is easier to reason about, especially across threads.
If 10 threads read the same value and none can change it, there is no data race.

### Common types

```rust
// Integers
let chunk_size: usize = 4 * 1024 * 1024;   // 4 MiB (unsigned, pointer-sized)
let port: u16 = 8080;                       // unsigned 16-bit (0 to 65535)
let count: i32 = -1;                        // signed 32-bit

// Booleans
let is_encrypted: bool = true;

// Strings
let username: String = String::from("alice");  // owned, heap-allocated
let greeting: &str = "hello";                 // borrowed string slice (a view)

// Arrays and Vectors
let key: [u8; 32] = [0u8; 32];              // fixed-size array: 32 bytes
let chunks: Vec<u8> = Vec::new();            // growable list (like ArrayList)

// Byte slices
let data: &[u8] = b"hello";                 // borrowed view into bytes
```

### Where this appears in FreeBox

```rust
// packages/crypto/src/encryption.rs
pub const CHUNK_SIZE: usize = 4 * 1024 * 1024;  // constant: 4 MiB

// packages/crypto/src/keys.rs
pub struct MasterSecret([u8; 32]);  // a struct wrapping a 32-byte array
```

---

## 4. Ownership — Rust's Superpower

This is the #1 concept that makes Rust different from every other language.

### The Rules

1. Every value has exactly **one owner** (a variable)
2. When the owner goes out of scope, the value is **dropped** (freed)
3. You can **move** ownership to another variable, but then the original is gone

```rust
fn main() {
    let key = String::from("secret_key_123");

    let key2 = key;        // ownership MOVES to key2
    // println!("{}", key); // ERROR: `key` was moved, it no longer exists
    println!("{}", key2);   // OK: key2 is the owner now
}
```

### Analogy: Ownership is like a physical key

Imagine a house key. Only one person can hold it at a time. You can:
- **Hand it to someone** (move) — now they have it, you do not
- **Let someone look at it** (borrow) — they can see it but you keep it
- **Give someone a copy** (clone) — expensive, now two keys exist

### Why FreeBox cares about ownership

```rust
// packages/crypto/src/encryption.rs
// FileKey does NOT implement Clone — you cannot accidentally copy a secret key
#[derive(Zeroize, ZeroizeOnDrop)]   // key bytes are wiped when dropped
pub struct FileKey([u8; 32]);

// If FileKey had Clone, you could write:
//   let key2 = key.clone();  // BAD: now the key exists in two places in memory
// Without Clone, the compiler prevents this entirely.
```

When a `FileKey` goes out of scope, `ZeroizeOnDrop` overwrites its 32 bytes
with zeros. Because ownership guarantees **exactly one owner**, we know the
key is wiped exactly once, from exactly one location. No leaks possible.

---

## 5. Borrowing and References

Instead of moving ownership, you can **borrow** a value:

```rust
fn print_length(s: &str) {    // `&str` means "I'm borrowing a string"
    println!("Length: {}", s.len());
}   // the borrow ends here — the original owner still has the value

fn main() {
    let name = String::from("alice");
    print_length(&name);       // lend `name` to the function
    println!("{}", name);      // still works — we still own `name`
}
```

### Two kinds of borrows

```rust
// Shared borrow (&T) — read-only, many allowed at the same time
fn read_key(key: &FileKey) { /* can read but not modify */ }

// Mutable borrow (&mut T) — read+write, only ONE at a time
fn advance_ratchet(session: &mut RatchetSession) { /* can modify */ }
```

**The Rule**: You can have EITHER:
- Many `&T` (shared read) at the same time, OR
- Exactly one `&mut T` (exclusive write)
- Never both

This is how Rust prevents data races **at compile time**.

### Where this appears in FreeBox

```rust
// packages/crypto/src/encryption.rs
pub fn encrypt_chunk(
    key: &FileKey,      // borrows the key (read-only)
    index: u64,         // copies the integer (cheap)
    data: &[u8],        // borrows the byte slice (read-only)
) -> anyhow::Result<ChunkCiphertext> {
    // key is borrowed — the caller still owns it and can use it for the next chunk
}
```

---

## 6. Structs — Building Blocks

Structs group related data together (like a class in Java/Python, but with no
inheritance).

```rust
// Simple struct
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_url: String,
}

// Tuple struct (wraps a single value — used for type safety)
pub struct FileKey([u8; 32]);   // a FileKey IS a [u8; 32], but the type system
                                 // treats them differently, so you can't
                                 // accidentally pass a random byte array
                                 // where a FileKey is expected.

// Methods on a struct (like class methods)
impl FileKey {
    // Associated function (like a static method) — no `self` parameter
    pub fn generate() -> Self {
        // ...
    }

    // Method — takes `&self` (borrows the struct)
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0   // .0 accesses the first field of a tuple struct
    }
}
```

### Where this appears in FreeBox

```rust
// apps/server/src/state.rs
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,              // shared config
    pub db: PgPool,                       // database connection pool
    pub storage: Arc<dyn StorageProvider>, // storage backend (any provider)
}

// Using it:
impl AppState {
    pub fn new(config: Config, db: PgPool, storage: Arc<dyn StorageProvider>) -> Self {
        Self {
            config: Arc::new(config),
            db,
            storage,
        }
    }
}
```

---

## 7. Enums — Multiple Choices

Enums in Rust are far more powerful than in other languages. Each variant can
carry different data:

```rust
// packages/core/src/error.rs
pub enum Error {
    // Simple variant — no data
    Unauthenticated,

    // Variant with named fields (like a struct inside the enum)
    NotFound { key: String },

    // Variant wrapping another error type
    Io(std::io::Error),
}
```

### Pattern matching (switch on steroids)

```rust
match error {
    Error::NotFound { key } => {
        println!("Could not find: {}", key);
    }
    Error::Unauthenticated => {
        println!("Please log in");
    }
    Error::Io(io_err) => {
        println!("I/O problem: {}", io_err);
    }
    _ => {
        println!("Something else went wrong");
    }
}
```

**The compiler forces you to handle every case.** If you add a new variant to
the enum, every `match` statement that doesn't handle it becomes a compile error.
This prevents "forgot to handle the new error type" bugs.

### Where this appears in FreeBox

```rust
// apps/server/src/error.rs — converting our errors to HTTP responses
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_title) = match &self {
            AppError::NotFound(_)     => (StatusCode::NOT_FOUND, "Not Found"),
            AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "Unauthorized"),
            AppError::BadRequest(_)   => (StatusCode::BAD_REQUEST, "Bad Request"),
            AppError::Internal(e) => {
                tracing::error!(error = %e, "Internal server error");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
            }
            // ... every variant must be handled
        };
    }
}
```

---

## 8. Traits — Shared Behavior

Traits define **behavior** that types can implement (like interfaces in Java/Go,
but more powerful). This is the most important concept for FreeBox's plugin system.

```rust
// packages/core/src/storage.rs
// ANY storage backend (S3, local, GCS) must implement these methods:
#[async_trait]
pub trait StorageProvider: Send + Sync {
    fn id(&self) -> &str;
    async fn put(&self, key: &str, data: Bytes) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Bytes>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>>;
    // ...
}
```

Now any struct that implements this trait can be used as a storage backend:

```rust
// plugins/storage-s3/src/lib.rs
#[async_trait]
impl StorageProvider for S3Plugin {
    fn id(&self) -> &str { "storage-s3" }

    async fn put(&self, key: &str, data: Bytes) -> Result<()> {
        self.op()?.write(key, data).await  // uses OpenDAL to write to S3
    }
    // ... each method delegates to S3 via OpenDAL
}

// plugins/storage-local/src/lib.rs
#[async_trait]
impl StorageProvider for LocalPlugin {
    fn id(&self) -> &str { "storage-local" }

    async fn put(&self, key: &str, data: Bytes) -> Result<()> {
        self.op()?.write(key, data).await  // uses OpenDAL to write to disk
    }
    // ... each method delegates to local filesystem via OpenDAL
}
```

The server does not care which one is being used:

```rust
// The server just calls methods on `dyn StorageProvider`:
state.storage.put("chunks/abc/00000001", encrypted_bytes).await?;
// This might go to S3, local disk, GCS — the server does not know or care.
```

### `dyn Trait` — dynamic dispatch (trait objects)

```rust
// `Arc<dyn StorageProvider>` means:
// "A shared pointer to SOME type that implements StorageProvider,
//  but we do not know which specific type at compile time."
pub storage: Arc<dyn StorageProvider>,
```

This is how FreeBox achieves its "bring your own cloud" architecture.
The server is written once; storage plugins are swappable at runtime.

### `Send + Sync` — thread safety markers

```rust
pub trait StorageProvider: Send + Sync { ... }
//                         ^^^^   ^^^^
// Send = can be transferred to another thread
// Sync = can be shared between threads via &reference
```

These are automatic traits — the compiler checks them for you. If your struct
contains something that is not thread-safe (like a raw pointer), it will not
compile as `Send + Sync`.

---

## 9. Error Handling — No Exceptions

Rust has no try/catch. Instead, functions that can fail return `Result<T, E>`:

```rust
enum Result<T, E> {
    Ok(T),    // success — contains the value
    Err(E),   // failure — contains the error
}
```

### The `?` operator — propagate errors concisely

```rust
// Without `?`:
fn read_file(path: &str) -> Result<String, io::Error> {
    let content = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => return Err(e),  // early return with the error
    };
    Ok(content)
}

// With `?` — does the same thing in one line:
fn read_file(path: &str) -> Result<String, io::Error> {
    let content = std::fs::read_to_string(path)?;  // returns Err early if it fails
    Ok(content)
}
```

### Where this appears in FreeBox

```rust
// apps/server/src/api/auth.rs
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<impl IntoResponse> {        // handler returns Result
    validate_username(&req.username)?;   // ? = if validation fails, return error
    validate_email(&req.email)?;         // ? = if email is bad, return error

    let server_hash = hash_password_server(&req.password_hash)?;

    let mut tx = state.db.begin().await  // start DB transaction
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    //   ^^^^^^^^ converts one error type to another, then ? propagates it

    // ... if everything succeeds, return the response
    Ok((StatusCode::CREATED, Json(tokens)))
}
```

### `anyhow` vs `thiserror`

FreeBox uses two error crates:

```rust
// `thiserror` — for LIBRARY code (packages/core, packages/crypto)
// Defines specific, matchable error types
#[derive(Debug, Error)]
pub enum Error {
    #[error("Object not found: {key}")]
    NotFound { key: String },
    #[error("Storage backend error: {message}")]
    Storage { message: String },
}

// `anyhow` — for APPLICATION code (apps/server, apps/cli)
// Wraps any error into a generic box (good for "just tell me what went wrong")
fn do_stuff() -> anyhow::Result<()> {
    let data = std::fs::read("file.txt")?;   // io::Error auto-converted
    let json: Value = serde_json::from_slice(&data)?;  // serde error auto-converted
    Ok(())
}
```

---

## 10. Option — Nullable Done Right

Rust has no `null`. Instead, values that might be absent use `Option<T>`:

```rust
enum Option<T> {
    Some(T),   // the value exists
    None,      // no value
}

// Example from FreeBox:
pub struct PluginManifest {
    pub id: String,                    // always present
    pub author: Option<String>,        // might be absent
    pub license: Option<String>,       // might be absent
}

// Using it:
match manifest.author {
    Some(name) => println!("Author: {}", name),
    None       => println!("No author specified"),
}

// Shorthand:
let author = manifest.author.unwrap_or("Unknown".to_string());
```

The compiler forces you to check for `None` before using the value.
`NullPointerException` literally cannot happen in Rust.

---

## 11. Generics — Write Once, Use for Any Type

```rust
// A function that works with ANY event type:
async fn publish_typed<E: Event>(&self, event: E) -> Result<()> {
//                     ^^^^^^^^^
// "E can be any type, as long as it implements the Event trait"
    let payload = serde_json::to_value(&event)?;
    self.publish_raw(E::event_type(), payload).await
}

// Called with different types:
bus.publish_typed(FileUploaded { ... }).await?;     // E = FileUploaded
bus.publish_typed(UserRegistered { ... }).await?;   // E = UserRegistered
```

The compiler generates a specialized version for each type used. Zero runtime cost.

---

## 12. Async/Await — Doing Many Things at Once

Network I/O (database queries, HTTP requests, file uploads) takes time. Instead
of blocking a thread while waiting, async code **yields** and lets other work
proceed.

```rust
// `async fn` returns a Future — a value that will produce a result later
async fn upload_file(data: &[u8]) -> Result<()> {
    let encrypted = encrypt(data).await?;        // wait for encryption
    storage.put("key", encrypted).await?;         // wait for upload
    db.record_upload().await?;                    // wait for DB write
    Ok(())
}
// While one upload is waiting for S3 to respond, another upload can
// be encrypting its data. Thousands of concurrent uploads, one thread.
```

### Tokio — the async runtime

Rust's async needs a **runtime** to actually execute futures. FreeBox uses Tokio:

```rust
// apps/server/src/main.rs
#[tokio::main]                     // starts the Tokio runtime
async fn main() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

### Where this appears in FreeBox

Every API handler, every database query, every storage operation is `async`:

```rust
// apps/server/src/api/files.rs
pub async fn upload_chunk(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse> {
    // All of these are async — they do not block the thread
    let upload = sqlx::query!(...)
        .fetch_optional(&state.db)
        .await?;                          // DB query (async)

    state.storage
        .put(&storage_key, body)
        .await?;                          // Storage write (async)

    Ok(StatusCode::NO_CONTENT)
}
```

---

## 13. Modules and Crates — Organizing Code

### Crate = a package (like an npm package)

```
freebox/
  packages/
    core/       <-- one crate (freebox-core)
    crypto/     <-- one crate (freebox-crypto)
  apps/
    server/     <-- one crate (freebox-server)
    cli/        <-- one crate (fbx)
  plugins/
    storage-s3/ <-- one crate (freebox-storage-s3)
```

Each crate has a `Cargo.toml` (like `package.json`) and a `src/lib.rs` or
`src/main.rs` entry point.

### Module = a file or folder within a crate

```rust
// packages/core/src/lib.rs
pub mod error;     // loads src/error.rs
pub mod event;     // loads src/event.rs
pub mod plugin;    // loads src/plugin.rs
pub mod storage;   // loads src/storage.rs

// Re-export so users write `freebox_core::Error` not `freebox_core::error::Error`
pub use error::{Error, Result};
```

### Visibility

```rust
pub struct Config { ... }       // public — anyone can use
pub(crate) fn helper() { ... }  // only within this crate
fn private_fn() { ... }         // only within this file/module
```

### Workspace = a group of crates that share dependencies

```toml
# Cargo.toml (root)
[workspace]
members = [
    "packages/core",
    "packages/crypto",
    "apps/server",
    "apps/cli",
    "plugins/storage-s3",
    "plugins/storage-local",
]

# Pin dependency versions once, use everywhere:
[workspace.dependencies]
tokio = { version = "1.37", features = ["full"] }
```

---

## 14. Smart Pointers — Arc, Box, Mutex

### `Box<T>` — heap allocation

```rust
// Puts data on the heap instead of the stack.
// Used when the size is not known at compile time (trait objects).
let plugin: Box<dyn Plugin> = Box::new(S3Plugin::new());
```

### `Arc<T>` — shared ownership across threads

```rust
// Arc = Atomic Reference Counted
// Multiple owners can share the same data safely across threads.
let storage: Arc<dyn StorageProvider> = Arc::new(S3Plugin::new());

let storage2 = storage.clone();  // does NOT copy the plugin —
                                  // just increments a counter.
                                  // Both point to the same data.

// When the last Arc is dropped, the data is freed.
```

### Where this appears in FreeBox

```rust
// apps/server/src/state.rs
pub struct AppState {
    pub config: Arc<Config>,              // config shared across all handlers
    pub db: PgPool,                       // PgPool is already Arc inside
    pub storage: Arc<dyn StorageProvider>, // storage shared across all handlers
}
// AppState is cloned for every request, but the clones share the same
// Config and StorageProvider via Arc — cheap (just incrementing counters).
```

### `Mutex<T>` and `RwLock<T>` — interior mutability

```rust
use std::sync::Mutex;

let counter = Mutex::new(0);

// Only one thread can lock at a time:
{
    let mut value = counter.lock().unwrap();
    *value += 1;
}  // lock is released when `value` goes out of scope
```

---

## 15. Closures — Functions as Values

Closures are anonymous functions that capture variables from their environment:

```rust
// A closure that doubles a number:
let double = |x: i32| x * 2;
println!("{}", double(5));  // 10

// Closures can capture variables:
let prefix = "freebox";
let make_subject = |event_type: &str| format!("{}.{}", prefix, event_type);
println!("{}", make_subject("file.uploaded"));  // "freebox.file.uploaded"
```

### Where this appears in FreeBox

```rust
// Event bus handlers are closures:
bus.subscribe_typed(|event: FileUploaded| async move {
    tracing::info!("File {} uploaded", event.file_id);
    Ok(())
}).await?;

// Error mapping uses closures:
sqlx::query!(...)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
//   ^^^^^^^^ this closure converts one error type to another
```

---

## 16. Derive Macros — Auto-Generated Code

`#[derive(...)]` tells the compiler to automatically generate common trait
implementations:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileUploaded {
    pub file_id: Uuid,
    pub user_id: Uuid,
    pub size_bytes: u64,
    pub content_hash: String,
}

// What each derive gives you:
// Debug        -> println!("{:?}", event)  — human-readable debug output
// Clone        -> event.clone()            — create a copy
// PartialEq    -> event1 == event2         — equality comparison
// Serialize    -> serde_json::to_string()  — convert to JSON
// Deserialize  -> serde_json::from_str()   — parse from JSON
```

### FreeBox-specific derives

```rust
// packages/crypto/src/keys.rs
#[derive(Zeroize, ZeroizeOnDrop)]     // from the `zeroize` crate
pub struct MasterSecret([u8; 32]);

// Zeroize       -> secret.zeroize()  — manually wipe bytes to zero
// ZeroizeOnDrop -> when the value is dropped (goes out of scope),
//                  the bytes are automatically overwritten with zeros,
//                  preventing secrets from lingering in memory.
```

---

## 17. Testing

Tests live in the same file as the code they test:

```rust
// packages/crypto/src/encryption.rs

// ... production code above ...

#[cfg(test)]                        // only compiled during `cargo test`
mod tests {
    use super::*;                   // import everything from the parent module

    #[test]                         // marks a test function
    fn round_trip_single_chunk() {
        let key = FileKey::generate();
        let plaintext = b"Hello, FreeBox!";
        let chunk = encrypt_chunk(&key, 0, plaintext).unwrap();
        let decrypted = decrypt_chunk(&key, 0, &chunk).unwrap();
        assert_eq!(decrypted.as_ref(), plaintext);
    }

    #[test]
    fn tampered_ciphertext_rejected() {
        let key = FileKey::generate();
        let mut chunk = encrypt_chunk(&key, 0, b"sensitive").unwrap();
        chunk.ciphertext[5] ^= 0xFF;   // flip one bit
        let result = decrypt_chunk(&key, 0, &chunk);
        assert!(result.is_err(), "tampered data must be rejected");
    }

    #[tokio::test]                  // for async test functions
    async fn noop_event_bus_works() {
        let bus = NoopEventBus;
        let result = bus.publish_typed(FileUploaded { ... }).await;
        assert!(result.is_ok());
    }
}
```

```bash
cargo test                          # run all tests
cargo test -p freebox-crypto        # test one crate
cargo test encryption               # test functions matching "encryption"
cargo test -- --nocapture            # show println output
```

---

## 18. Cargo — The Build Tool

Cargo is Rust's npm/pip/maven. It handles building, testing, dependencies,
and publishing.

```bash
# Commands you will use daily:
cargo build                # compile (debug mode, fast compile)
cargo build --release      # compile (release mode, optimized, slow compile)
cargo test                 # run all tests
cargo run -p freebox-server  # build and run a specific crate
cargo check                # type-check without building (fastest feedback)
cargo clippy               # linter (catches common mistakes)
cargo fmt                  # auto-format all code
cargo audit                # check dependencies for known vulnerabilities
cargo doc --open           # generate and open API documentation
```

### Cargo.toml — the manifest

```toml
# packages/crypto/Cargo.toml
[package]
name = "freebox-crypto"
version = "0.1.0"
edition = "2021"

[dependencies]
aes-gcm = { workspace = true }       # from workspace root
argon2  = { workspace = true }
serde   = { version = "1", features = ["derive"] }  # specific version

[dev-dependencies]               # only for tests
proptest = "1.4"
```

---

## 19. FreeBox Concept Map

Here is every Rust concept and where it appears in FreeBox:

```
Concept              Where in FreeBox                          Why
---------            ----------------                          ---
Ownership            FileKey (non-Clone, ZeroizeOnDrop)        Prevent key duplication
Borrowing            encrypt_chunk(key: &FileKey, data: &[u8]) Avoid copying large data
Structs              AppState, Config, Claims                  Group related data
Enums                Error, AppError                           Exhaustive error handling
Traits               StorageProvider, Plugin, EventBus         Plugin system
async/await          Every handler, every DB query             Non-blocking I/O
Result<T, E>         Every function that can fail              No exceptions
Option<T>            PluginManifest.author                     No null
Arc<T>               AppState.config, AppState.storage         Shared across threads
Box<dyn Trait>       PluginRegistry stores Box<dyn Plugin>     Heap-allocated trait objects
Generics             EventBusExt::publish_typed<E: Event>      Type-safe event bus
Closures             .map_err(|e| ...), event handlers         Inline functions
Derive macros        #[derive(Serialize, Deserialize, Clone)]  Auto-implement traits
Modules              pub mod api; pub mod crypto;               Code organization
Workspace            Cargo.toml [workspace]                    Shared deps across crates
#[cfg(test)]         mod tests { ... }                         Test alongside code
ZeroizeOnDrop        MasterSecret, FileKey, RatchetSession     Wipe secrets from memory
```

---

## 20. Recommended Learning Path

### Week 1: Basics
1. Read chapters 1-6 of [The Rust Book](https://doc.rust-lang.org/book/)
   (free, official)
2. Do [Rustlings](https://github.com/rust-lang/rustlings) exercises 1-40
3. Read `packages/core/src/error.rs` — the simplest file in FreeBox

### Week 2: Ownership & Traits
1. Read chapters 7-10 of The Rust Book (traits, generics, lifetimes)
2. Read `packages/core/src/storage.rs` — understand the trait design
3. Read `plugins/storage-local/src/lib.rs` — see a trait implementation

### Week 3: Async & Error Handling
1. Read [Tokio tutorial](https://tokio.rs/tokio/tutorial)
2. Read `apps/server/src/api/health.rs` — simplest handler
3. Read `apps/server/src/api/auth.rs` — real handler with error handling

### Week 4: Crypto & Testing
1. Read `packages/crypto/src/encryption.rs` — understand the crypto code
2. Run `cargo test -p freebox-crypto` and read the test output
3. Write a new test for `RatchetSession` (send 10 messages in sequence)

### Ongoing Reference
- [Rust by Example](https://doc.rust-lang.org/rust-by-example/) — learn by doing
- [Rust Cheat Sheet](https://cheats.rs/) — quick reference card
- `cargo doc --open` — FreeBox's own generated API documentation
