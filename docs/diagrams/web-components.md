# Web Frontend Components

> Last synced: 2026-05-11

Component tree, stores, and API client for `apps/web` (React + TypeScript + Vite).

## Component Tree

```mermaid
flowchart TB
    APP["App.tsx<br/>BrowserRouter + QueryClientProvider"]

    subgraph Public Routes
        LOGIN["LoginPage"]
        REGISTER["RegisterPage"]
        OAUTH_CB["OAuthCallbackPage"]
    end

    subgraph Protected["ProtectedRoute (checks authStore)"]
        LAYOUT["AppLayout<br/>(nav, sidebar, outlet)"]
        FILES["FilesPage"]
        TRASH["TrashPage"]
        STORAGE["StoragePage"]
        SETTINGS["SettingsPage"]
        UPLOAD["UploadModal"]
    end

    subgraph UI Components
        BTN["Button"]
        CARD["Card"]
        INPUT["Input"]
        SPIN["Spinner"]
    end

    APP --> LOGIN
    APP --> REGISTER
    APP --> OAUTH_CB
    APP --> Protected

    LAYOUT --> FILES
    LAYOUT --> TRASH
    LAYOUT --> STORAGE
    LAYOUT --> SETTINGS
    FILES --> UPLOAD

    FILES --> BTN
    FILES --> CARD
    LOGIN --> INPUT
    LOGIN --> BTN
    REGISTER --> INPUT
    REGISTER --> BTN
    UPLOAD --> INPUT
    UPLOAD --> BTN
    LAYOUT --> SPIN
```

## State Management

```mermaid
classDiagram
    class useAuthStore {
        <<Zustand Store>>
        +isAuthenticated: boolean
        +setAuthenticated(value: boolean) void
        +logout() void
    }

    class localStorage {
        <<Browser Storage>>
        fbx_access_token: string
        fbx_refresh_token: string
    }

    class QueryClient {
        <<TanStack Query>>
        +retry: 1
        +staleTime: 30s
    }

    useAuthStore --> localStorage : reads token existence
    useAuthStore --> localStorage : clearTokens() on logout
```

## API Client Architecture

```mermaid
classDiagram
    class apiClient {
        <<module: lib/api/client.ts>>
        +hashPassword(password, salt) Promise~string~
        +getAccessToken() string|null
        +getRefreshToken() string|null
        +saveTokens(tokens) void
        +clearTokens() void
        +register(username, email, password) Promise~AuthTokens~
        +login(username, password) Promise~AuthTokens~
        +logout() Promise~void~
        +listFiles(limit, offset) Promise~FileListResponse~
        +getFileMeta(fileId) Promise~FileMeta~
        +uploadInit(req) Promise~UploadInitResponse~
        +uploadChunk(uploadId, chunk) Promise~void~
        +uploadComplete(uploadId) Promise~UploadCompleteResponse~
        +downloadChunk(fileId, n) Promise~Blob~
        +deleteFile(fileId) Promise~void~
        +restoreFile(fileId) Promise~void~
        +listTrash(limit, offset) Promise~TrashListResponse~
        +listOAuthProviders() Promise~OAuthProvider[]~
        +listDatasources() Promise~Datasource[]~
        +listBuckets() Promise~BucketListResponse~
    }

    class AxiosInterceptors {
        <<request>>
        Attach Bearer token
        <<response>>
        Auto-refresh on 401
        Retry original request
    }

    apiClient --> AxiosInterceptors : configured via interceptors
```

## TypeScript Types (mirror server DTOs)

```mermaid
classDiagram
    class AuthTokens {
        +access_token: string
        +refresh_token: string
        +expires_in: number
    }

    class FileMeta {
        +file_id: string
        +encrypted_name: string
        +size_bytes: number
        +total_chunks: number
        +encrypted_key_envelope: string
        +content_hash: string
        +created_at: string
    }

    class FileListResponse {
        +files: FileMeta[]
        +total: number
        +limit: number
        +offset: number
    }

    class TrashFile {
        +file_id: string
        +encrypted_name: string
        +size_bytes: number
        +total_chunks: number
        +content_hash: string
        +created_at: string
        +deleted_at: string
    }

    class UploadInitRequest {
        +total_chunks: number
        +size_bytes: number
        +encrypted_key_envelope: string
        +content_hash: string
        +encrypted_name: string
    }

    class OAuthProvider {
        +provider: string
        +provider_email: string?
        +provider_username: string?
        +avatar_url: string?
        +created_at: string
    }

    class AuditEvent {
        +id: string
        +user_id: string
        +event_type: string
        +source: string
        +provider: string?
        +provider_user_id: string?
        +details: Record
        +created_at: string
    }

    FileListResponse --> FileMeta : contains
```

## File Structure

| File | Role |
|------|------|
| `App.tsx` | Root component, routing, QueryClient setup |
| `components/ProtectedRoute.tsx` | Auth guard (redirects to /login) |
| `components/AppLayout.tsx` | Shell layout (nav + content outlet) |
| `components/ui/Button.tsx` | Reusable button component |
| `components/ui/Card.tsx` | Card container |
| `components/ui/Input.tsx` | Form input |
| `components/ui/Spinner.tsx` | Loading spinner |
| `lib/api/client.ts` | Axios-based API client with auto-refresh |
| `lib/api/types.ts` | TypeScript interfaces matching server DTOs |
| `lib/api/index.ts` | Re-exports |
| `stores/authStore.ts` | Zustand auth state |
| `pages/FilesPage.tsx` | File list + upload |
| `pages/TrashPage.tsx` | Trash (soft-deleted files) |
| `pages/StoragePage.tsx` | Storage/datasource management |
| `pages/SettingsPage.tsx` | User settings, OAuth links |
| `pages/LoginPage.tsx` | Login form |
| `pages/RegisterPage.tsx` | Registration form |
| `pages/OAuthCallbackPage.tsx` | OAuth redirect handler |
| `pages/UploadModal.tsx` | Upload dialog |
