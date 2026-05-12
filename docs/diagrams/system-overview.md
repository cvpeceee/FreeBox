# System Overview

> Last synced: 2026-05-11

High-level crate dependency graph and data flow for the FreeBox platform.

## Crate Dependency Graph

```mermaid
graph TD
    subgraph Clients
        WEB["apps/web<br/>(React SPA)"]
        CLI["apps/cli<br/>(fbx)"]
    end

    subgraph Server
        SRV["apps/server<br/>(Axum REST API)"]
    end

    subgraph Packages
        CORE["packages/core<br/>Plugin, StorageProvider,<br/>EventBus traits"]
        CRYPTO["packages/crypto<br/>AES-256-GCM, X3DH,<br/>Double Ratchet"]
    end

    subgraph Plugins
        S3["plugins/storage-s3<br/>S3Plugin"]
        LOCAL["plugins/storage-local<br/>LocalPlugin"]
    end

    subgraph External
        PG[(PostgreSQL)]
        STORE[(S3 / R2 / Local FS)]
    end

    WEB -->|REST / JSON| SRV
    CLI -->|REST / JSON| SRV
    CLI --> CRYPTO

    SRV --> CORE
    SRV --> CRYPTO
    SRV --> S3
    SRV --> LOCAL

    S3 --> CORE
    LOCAL --> CORE

    S3 -->|OpenDAL| STORE
    LOCAL -->|OpenDAL| STORE

    SRV --> PG
```

## Data Flow: File Upload (End-to-End)

```mermaid
sequenceDiagram
    participant U as User
    participant CLI as fbx CLI
    participant CRYPTO as freebox-crypto
    participant SRV as Server (Axum)
    participant AUTH as Auth Middleware
    participant FILES as files.rs Handler
    participant SP as StorageProvider
    participant PG as PostgreSQL
    participant S3 as S3 / Local FS

    U->>CLI: fbx upload file.pdf
    CLI->>CRYPTO: FileKey::generate()
    CLI->>CRYPTO: encrypt_chunk(key, idx, data)
    CRYPTO-->>CLI: ChunkCiphertext[]

    CLI->>SRV: POST /files/upload/init
    SRV->>AUTH: Validate JWT
    AUTH-->>SRV: Claims{sub, username}
    SRV->>FILES: upload_init()
    FILES->>PG: INSERT INTO uploads
    PG-->>FILES: upload_id
    FILES-->>CLI: {upload_id, chunk_size}

    loop For each chunk (8 parallel)
        CLI->>SRV: PUT /files/upload/:id (chunk N)
        SRV->>AUTH: Validate JWT
        SRV->>FILES: upload_chunk()
        FILES->>SP: put(key, ciphertext)
        SP->>S3: PUT object
        FILES->>PG: UPDATE uploads SET chunks_received
    end

    CLI->>SRV: POST /files/upload/:id/complete
    SRV->>FILES: upload_complete()
    FILES->>PG: INSERT INTO files
    FILES-->>CLI: {file_id}
    CLI-->>U: ✓ Uploaded file.pdf
```

## Data Flow: File Download

```mermaid
sequenceDiagram
    participant U as User
    participant CLI as fbx CLI
    participant CRYPTO as freebox-crypto
    participant SRV as Server (Axum)
    participant FILES as files.rs Handler
    participant SP as StorageProvider
    participant S3 as S3 / Local FS

    U->>CLI: fbx download file-id
    CLI->>SRV: GET /files/:file_id (metadata)
    SRV->>FILES: get_file_meta()
    FILES-->>CLI: {total_chunks, encrypted_key_envelope}

    loop For each chunk
        CLI->>SRV: GET /files/:id/chunk/:n
        SRV->>FILES: download_chunk()
        FILES->>SP: get(chunk_key)
        SP->>S3: GET object
        S3-->>SP: ciphertext bytes
        SP-->>FILES: Bytes
        FILES-->>CLI: encrypted chunk
    end

    CLI->>CRYPTO: decrypt_chunk(key, idx, chunk)
    CRYPTO-->>CLI: plaintext bytes
    CLI->>U: Write decrypted file to disk
```

## Module Map

| Crate | Key Exports | Depends On |
|-------|-------------|------------|
| `freebox-core` | `Plugin`, `StorageProvider`, `EventBus`, `Error` | (none — leaf crate) |
| `freebox-crypto` | `FileKey`, `encrypt_chunk`, `decrypt_chunk`, `x3dh_initiate`, `RatchetSession` | (none — leaf crate) |
| `freebox-server` | `AppState`, `Config`, `AppError`, `RateLimiter`, `Claims` | `freebox-core`, `freebox-crypto`, `storage-s3`, `storage-local` |
| `freebox-cli` | `Cli`, `Commands`, `Session`, `CliConfig` | `freebox-crypto` |
| `storage-s3` | `S3Plugin`, `BucketInfo`, `create_bucket`, `list_buckets` | `freebox-core` |
| `storage-local` | `LocalPlugin` | `freebox-core` |
| `apps/web` | React SPA (pages, stores, API client) | (REST API only) |
