# Server Files Module

> Last synced: 2026-05-11

Data flow and types for `apps/server/src/api/files.rs` — chunked upload/download.

## Upload Protocol (Three-Phase)

```mermaid
stateDiagram-v2
    [*] --> Init: POST /files/upload/init
    Init --> Uploading: Server returns upload_id
    Uploading --> Uploading: PUT /files/upload/:id (chunk N)
    Uploading --> Completing: POST /files/upload/:id/complete
    Completing --> Complete: All chunks verified
    Completing --> Failed: Missing chunks
    Complete --> [*]: file_id returned

    note right of Init
        Client sends:
        - total_chunks
        - size_bytes
        - encrypted_key_envelope
        - content_hash (BLAKE3)
        - encrypted_name
    end note

    note right of Uploading
        Up to 8 parallel streams
        32 MiB body limit per chunk
        Server stores to StorageProvider
    end note
```

## File Lifecycle

```mermaid
flowchart LR
    UPLOAD[Upload Complete] --> ACTIVE[Active File]
    ACTIVE -->|DELETE /files/:id| TRASH[Soft-Deleted / Trash]
    TRASH -->|POST /files/:id/restore| ACTIVE
    TRASH -->|30 days| PURGE[Hard Delete]
    ACTIVE -->|PATCH /files/:id| ACTIVE
    PURGE --> GONE[Removed from DB + Storage]

    subgraph Orphan Cleanup
        STALE[Incomplete Upload > 24h] -->|Background task| GONE
    end
```

## Request/Response Types

```mermaid
classDiagram
    class UploadInitRequest {
        +total_chunks: u32
        +size_bytes: u64
        +encrypted_key_envelope: String
        +content_hash: String
        +encrypted_name: String
    }

    class UploadInitResponse {
        +upload_id: Uuid
        +chunk_size: usize
    }

    class UploadCompleteResponse {
        +file_id: Uuid
    }

    class FileMetaResponse {
        +file_id: Uuid
        +encrypted_name: String
        +size_bytes: i64
        +total_chunks: i32
        +encrypted_key_envelope: String
        +content_hash: String
        +created_at: DateTime
    }

    class FileListResponse {
        +files: Vec~FileMetaResponse~
        +total: i64
        +limit: i64
        +offset: i64
    }

    class TrashFileResponse {
        +file_id: Uuid
        +encrypted_name: String
        +size_bytes: i64
        +total_chunks: i32
        +content_hash: String
        +created_at: DateTime
        +deleted_at: DateTime
    }
```

## Handler → Storage Call Chain

```mermaid
sequenceDiagram
    participant H as Handler (files.rs)
    participant DB as PostgreSQL
    participant SP as StorageProvider
    participant S3 as S3 / Local FS

    Note over H: upload_chunk()
    H->>H: Extract Claims from request
    H->>DB: SELECT upload WHERE upload_id AND user_id
    H->>H: Validate chunk size ≤ 32 MiB
    H->>SP: put("chunks/{upload_id}/{chunk_index}", ciphertext)
    SP->>S3: PUT object
    H->>DB: UPDATE uploads SET chunks_received += 1

    Note over H: download_chunk()
    H->>DB: SELECT file WHERE file_id AND user_id
    H->>SP: get("chunks/{file_id}/{chunk_n}")
    SP->>S3: GET object
    S3-->>SP: Bytes
    SP-->>H: Bytes
    H-->>H: Return octet-stream response
```
