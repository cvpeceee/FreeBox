# CLI Commands

> Last synced: 2026-05-11

Class diagram for `apps/cli` — command structure, session, and config types.

## Command Tree

```mermaid
classDiagram
    class Cli {
        +server: String
        +quiet: bool
        +verbose: bool
        +command: Commands
    }

    class Commands {
        <<enum>>
        Auth(AuthCommands)
        Upload(files, destination, parallelism)
        Download(remote, output)
        List(path, long)
        Remove(remote, permanent)
        Sync(local, remote, watch, direction)
        Msg(MsgCommands)
        Mail(MailCommands)
        Datasource(ProviderCommands)
        Bucket(BucketCommand)
        Plugin(PluginCommands)
    }

    class AuthCommands {
        <<enum>>
        Register(username, email)
        Login(username)
        Logout
        Whoami
    }

    class MsgCommands {
        <<enum>>
        Send(to, message)
        Read
    }

    class MailCommands {
        <<enum>>
        Compose(to, subject, body)
        Read
    }

    class ProviderCommands {
        <<enum>>
        List
        Add(name, provider, endpoint, bucket, region)
        Remove(name)
    }

    class BucketCommand {
        <<enum>>
        List
        Create(name)
    }

    class PluginCommands {
        <<enum>>
        Install(name)
        List
        Remove(name)
    }

    class SyncDirection {
        <<enum>>
        Both
        Push
        Pull
    }

    Cli --> Commands
    Commands --> AuthCommands
    Commands --> MsgCommands
    Commands --> MailCommands
    Commands --> ProviderCommands
    Commands --> BucketCommand
    Commands --> PluginCommands
    Commands --> SyncDirection
```

## Session & Config

```mermaid
classDiagram
    class Session {
        +access_token: String
        +refresh_token: String
        +server: String
        +username: String
        +user_id: Uuid
        +save() Result~()~
        +load() Result~Session~$
        +clear() Result~()~$
    }

    class CliConfig {
        +server: String
        +sources: Vec~StorageSource~
        +load() CliConfig$
        +save() Result~()~
    }

    class StorageSource {
        +name: String
        +provider: String
        +endpoint: String
        +bucket: String
        +region: String
    }

    CliConfig --> StorageSource : 0..N

    note for Session "Storage: ~/.config/freebox/session.json\nWindows: %APPDATA%/FreeBox/session.json"
    note for CliConfig "Storage: ~/.config/freebox/config.toml\nWindows: %APPDATA%/FreeBox/config.toml"
```

## CLI Upload Flow

```mermaid
sequenceDiagram
    participant U as User
    participant CLI as fbx
    participant SESS as Session
    participant CFG as CliConfig
    participant CRYPTO as freebox-crypto
    participant SRV as Server

    U->>CLI: fbx upload file.pdf --parallelism 8
    CLI->>SESS: Session::load()
    SESS-->>CLI: {access_token, server, ...}

    CLI->>CRYPTO: FileKey::generate()
    CLI->>CLI: Split file into 4 MiB chunks
    CLI->>CRYPTO: encrypt_chunk(key, 0, chunk0)
    CLI->>CRYPTO: encrypt_chunk(key, 1, chunk1)

    CLI->>SRV: POST /files/upload/init
    SRV-->>CLI: {upload_id}

    par 8 parallel streams
        CLI->>SRV: PUT /files/upload/:id (chunk 0)
        CLI->>SRV: PUT /files/upload/:id (chunk 1)
        CLI->>SRV: PUT /files/upload/:id (chunk N)
    end

    CLI->>SRV: POST /files/upload/:id/complete
    SRV-->>CLI: {file_id}
    CLI-->>U: ✓ Uploaded file.pdf (file_id)
```

## CLI Module Map

| Module | File | Responsibility |
|--------|------|---------------|
| `main` | `main.rs` | Clap CLI struct, command dispatch |
| `auth` | `auth.rs` | register, login, logout, whoami handlers |
| `client` | `client.rs` | HTTP utils, `send_with_refresh()` |
| `session` | `session.rs` | Session struct, JSON persistence |
| `config` | `config.rs` | CliConfig, TOML persistence |
| `commands/upload` | `commands/upload.rs` | File encryption + chunked upload |
| `commands/download` | `commands/download.rs` | Chunked download + decryption |
| `commands/list` | `commands/list.rs` | `fbx ls` implementation |
| `commands/remove` | `commands/remove.rs` | `fbx rm` implementation |
| `commands/sync` | `commands/sync.rs` | Two-way directory sync |
| `commands/msg` | `commands/msg.rs` | Encrypted messaging |
| `commands/mail` | `commands/mail.rs` | Encrypted email |
| `commands/provider` | `commands/provider.rs` | Datasource management |
| `commands/bucket` | `commands/bucket.rs` | Bucket list/create |
| `commands/plugin` | `commands/plugin.rs` | Plugin install/list/remove |
