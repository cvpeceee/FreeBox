# Plugin Lifecycle

> Last synced: 2026-05-11

Plugin discovery, loading, event flow, and storage call patterns.

## Plugin Lifecycle State Machine

```mermaid
stateDiagram-v2
    [*] --> Discovered: plugin.toml found in plugins/
    Discovered --> Loading: Kernel reads PluginManifest
    Loading --> Validating: Parse config_schema
    Validating --> Loaded: Schema valid
    Validating --> Disabled: Schema invalid

    Loaded --> Running: on_load(ctx) returns Ok
    Loaded --> Disabled: on_load(ctx) returns Err

    Running --> Running: Handles events via EventBus
    Running --> Stopping: Graceful shutdown signal
    Stopping --> Stopped: on_unload() completes
    Stopping --> ForceKilled: 30s timeout exceeded

    Stopped --> [*]
    ForceKilled --> [*]
    Disabled --> [*]
```

## Plugin Discovery & Bootstrap

```mermaid
sequenceDiagram
    participant K as Kernel (main.rs)
    participant FS as File System
    participant PM as PluginManifest
    participant P as Plugin impl
    participant EB as EventBus
    participant SP as StorageProvider

    K->>FS: Scan plugins/ directory
    FS-->>K: [storage-s3/, storage-local/]

    loop For each plugin directory
        K->>FS: Read plugin.toml
        FS-->>K: TOML content
        K->>PM: Parse PluginManifest
        PM-->>K: {id, capabilities, config_schema}

        K->>K: Build PluginContext {event_bus, config, instance_id}
        K->>P: on_load(ctx)
        P->>P: Validate config
        P->>P: Initialize backend (e.g., OpenDAL Operator)
        P-->>K: Ok(())

        alt provides "storage.provider"
            K->>P: as_storage_provider()
            P-->>K: Some(Arc<dyn StorageProvider>)
            K->>K: Register as active StorageProvider
        end
    end
```

## Event Flow Between Plugins

```mermaid
flowchart LR
    subgraph FileHandler["files.rs Handler"]
        UL[Upload Complete]
    end

    subgraph EB["EventBus (type-erased)"]
        PUB["publish_typed(FileUploaded)"]
        SUB1["Subscriber: Sync Plugin"]
        SUB2["Subscriber: Messaging Plugin"]
        SUB3["Subscriber: Audit Logger"]
    end

    subgraph Plugins
        SYNC["Sync Engine\n(notify connected clients)"]
        MSG["Messaging Plugin\n(notify file share recipients)"]
        AUDIT["Audit Plugin\n(log to DB)"]
    end

    UL --> PUB
    PUB --> SUB1 --> SYNC
    PUB --> SUB2 --> MSG
    PUB --> SUB3 --> AUDIT
```

## Storage Provider Call Pattern

```mermaid
sequenceDiagram
    participant H as Handler
    participant K as Kernel
    participant SP as StorageProvider (trait)
    participant OP as OpenDAL Operator
    participant BE as Backend (S3/FS)

    H->>K: state.storage (Arc<dyn StorageProvider>)
    K-->>H: &S3Plugin or &LocalPlugin

    alt Small file (< 5 MiB)
        H->>SP: put(key, data)
        SP->>OP: write(key, data)
        OP->>BE: PUT object
    else Large file (multipart)
        H->>SP: create_multipart(key)
        SP->>OP: create_multipart_writer
        SP-->>H: MultipartUpload{upload_id, key}

        loop For each part
            H->>SP: upload_part(upload, part_num, data)
            SP->>OP: write_part
            SP-->>H: CompletedPart{part_number, etag}
        end

        H->>SP: complete_multipart(upload, parts)
        SP->>OP: complete_multipart_writer
    end
```

## Plugin Manifest Example

```toml
# plugins/storage-s3/plugin.toml
[plugin]
id          = "storage-s3"
name        = "Amazon S3 Storage"
version     = "1.0.0"
api_version = "^1.0"
author      = "FreeBox Team"
license     = "Apache-2.0"

[plugin.capabilities]
provides = ["storage.provider"]
requires = []
```
