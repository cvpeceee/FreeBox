# CLI Auth Flow

> Last synced: 2026-05-11

Login, token refresh, and session persistence in `apps/cli`.

## Login & Session Persistence

```mermaid
sequenceDiagram
    participant U as User
    participant CLI as fbx
    participant SESS as Session Store
    participant CRYPTO as freebox-crypto
    participant SRV as Server

    U->>CLI: fbx auth login
    CLI->>CLI: Prompt for username & password

    CLI->>SRV: GET /auth/salt/:username
    SRV-->>CLI: {salt}
    CLI->>CRYPTO: MasterSecret::derive(password, salt)
    CRYPTO-->>CLI: MasterSecret
    CLI->>CLI: Derive client_hash from MasterSecret

    CLI->>SRV: POST /auth/login {username, client_hash}
    SRV-->>CLI: {access_token, refresh_token, expires_in, user_id}

    CLI->>SESS: Session::save({access_token, refresh_token, server, username, user_id})
    CLI-->>U: ✓ Logged in as alice
```

## Automatic Token Refresh

```mermaid
sequenceDiagram
    participant CLI as fbx (any command)
    participant CLIENT as client.rs
    participant SESS as Session Store
    participant SRV as Server

    CLI->>CLIENT: send_with_refresh(request)
    CLIENT->>SRV: GET /api/... (with access_token)

    alt 200 OK
        SRV-->>CLIENT: Response
        CLIENT-->>CLI: Success
    else 401 Unauthorized (token expired)
        SRV-->>CLIENT: 401
        CLIENT->>SESS: Session::load() → refresh_token
        CLIENT->>SRV: POST /auth/refresh {refresh_token}
        SRV-->>CLIENT: {new_access_token, new_refresh_token}
        CLIENT->>SESS: Session::save(updated tokens)
        CLIENT->>SRV: Retry original request (with new access_token)
        SRV-->>CLIENT: Response
        CLIENT-->>CLI: Success
    end
```

## Session File Layout

```mermaid
flowchart LR
    subgraph Config Directory
        direction TB
        DIR["~/.config/freebox/<br/>or %APPDATA%/FreeBox/"]
        SESS["session.json"]
        CONF["config.toml"]
    end

    DIR --> SESS
    DIR --> CONF

    subgraph session.json
        S_JSON["{\n  access_token: '...',\n  refresh_token: '...',\n  server: 'https://freebox.io',\n  username: 'alice',\n  user_id: 'uuid'\n}"]
    end

    subgraph config.toml
        C_TOML["server = 'https://freebox.io'\n\n[[sources]]\nname = 'r2-prod'\nprovider = 's3'\nendpoint = 'https://...'\nbucket = 'freebox'\nregion = 'auto'"]
    end

    SESS -.-> S_JSON
    CONF -.-> C_TOML
```

## Registration Flow

```mermaid
sequenceDiagram
    participant U as User
    participant CLI as fbx
    participant CRYPTO as freebox-crypto
    participant SRV as Server

    U->>CLI: fbx auth register
    CLI->>CLI: Prompt for username, email, password

    CLI->>CRYPTO: MasterSecret::generate_salt()
    CLI->>CRYPTO: MasterSecret::derive(password, salt)
    CLI->>CRYPTO: IdentityKeyPair::generate()
    CLI->>CRYPTO: SignedPrekey::generate(identity)
    CLI->>CRYPTO: Generate OneTimePrekeys × 100

    CLI->>SRV: POST /auth/register {username, email, client_hash, salt, prekey_bundle}
    SRV-->>CLI: 201 {user_id}

    CLI->>SRV: POST /auth/login {username, client_hash}
    SRV-->>CLI: {access_token, refresh_token}

    CLI->>CLI: Session::save(...)
    CLI-->>U: ✓ Registered and logged in as alice
```
