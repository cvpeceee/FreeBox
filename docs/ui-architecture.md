# FreeBox Web UI Architecture

## Overview

The web UI is a **standalone single-page application (SPA)** that communicates
with the FreeBox backend exclusively through its documented REST API. There is
no server-side rendering, no shared runtime, and no build-time coupling to the
Rust codebase. This makes it trivially replaceable — a React Native mobile app,
a Flutter app, or any other client can be pointed at the same API and work
identically.

```
┌─────────────────────────┐          ┌─────────────────────────┐
│   Web UI (React SPA)    │  REST    │   FreeBox Server (Rust) │
│   apps/web/             │ ◄──────► │   apps/server/          │
│   localhost:5173 (dev)  │  JSON    │   localhost:8080        │
│   CDN / nginx (prod)    │          │   (Axum + PostgreSQL)   │
└─────────────────────────┘          └─────────────────────────┘
         ▲                                        ▲
         │                                        │
 Future: React Native              Future: gRPC / WebSocket
 Future: Flutter                   (same REST API, extended)
 Future: Electron
```

The backend does not know or care what client is talking to it — it only sees
HTTP requests with Bearer tokens. Any UI client that implements the auth flow
and speaks JSON can use FreeBox.

---

## Starting the Servers

### Prerequisites

| Tool | Version | Install |
|---|---|---|
| Rust + Cargo | stable | https://rustup.rs |
| Node.js | ≥ 20 | https://nodejs.org |
| Docker Desktop | any | https://docker.com |
| sqlx-cli | latest | `cargo install sqlx-cli --no-default-features --features rustls,postgres` |

---

### Step 1 — Start PostgreSQL

```powershell
# From the repo root
docker compose -f infra/docker-compose.dev.yml up -d
```

This starts PostgreSQL on port **5432** with a named Docker volume
(`postgres-data`). Data persists across restarts. Only `docker compose down -v`
destroys the volume.

---

### Step 2 — Apply Database Migrations

```powershell
cd apps/server
$env:DATABASE_URL = "postgres://freebox:freebox_dev@127.0.0.1:5432/freebox"
sqlx migrate run
```

This applies all three migrations:
- `001_initial_schema.sql` — users, files, uploads, refresh_tokens
- `002_oauth_accounts.sql` — oauth_accounts, oauth_states
- `003_account_audit_events.sql` — account_audit_events

Migrations are idempotent — re-running them is safe.

---

### Step 3 — Start the Backend Server

```powershell
# From the repo root (or apps/server/)
$env:DATABASE_URL = "postgres://freebox:freebox_dev@127.0.0.1:5432/freebox"
$env:JWT_SECRET   = "dev-secret-change-in-production-must-be-long"
cargo run -p freebox-server
```

Or create a `.env` file in `apps/server/` (loaded automatically at startup):

```env
DATABASE_URL=postgres://freebox:freebox_dev@127.0.0.1:5432/freebox
JWT_SECRET=dev-secret-change-in-production-must-be-long
STORAGE_PROVIDER=local
STORAGE_LOCAL_ROOT=./data
```

Then simply:

```powershell
cd apps/server
cargo run -p freebox-server
```

The server starts on **http://127.0.0.1:8080**. Verify with:

```powershell
Invoke-WebRequest http://127.0.0.1:8080/health
# Expected: StatusCode 200
```

Background tasks that start automatically:
- **Orphaned upload cleanup** — runs every hour, hard-deletes incomplete uploads older than 24 h

---

### Step 4 — Start the Web UI

```powershell
# In a separate terminal
cd apps/web
npm run dev
```

The UI starts on **http://localhost:5173**.

During development, Vite proxies all `/api/**` requests to
`http://127.0.0.1:8080` automatically — you never need to configure CORS or
touch the backend for UI development.

---

### Quick Start (all-in-one summary)

```powershell
# Terminal 1 — infrastructure
docker compose -f infra/docker-compose.dev.yml up -d

# Terminal 2 — backend (first run: apply migrations first)
cd apps/server
sqlx migrate run   # only needed once per fresh DB
cargo run -p freebox-server

# Terminal 3 — web UI
cd apps/web
npm run dev
```

Open **http://localhost:5173** in your browser.

---

## UI Technology Stack

| Layer | Library | Version | Purpose |
|---|---|---|---|
| Framework | React | 19 | Component model, JSX |
| Build tool | Vite | 8 | Dev server, HMR, bundling |
| Language | TypeScript | 5 | Type safety across all API boundaries |
| Routing | React Router | 7 | Client-side navigation, nested layouts |
| Server state | TanStack Query | 5 | API caching, background refetch, mutations |
| Client state | Zustand | 5 | Auth session (isAuthenticated) |
| HTTP client | Axios | 1 | Interceptors for auto token refresh |
| Styling | Tailwind CSS | 4 | Utility-first, zero runtime |
| Icons | Lucide React | latest | SVG icon set |
| Utilities | clsx | 2 | Conditional class merging |

---

## Project Structure

```
apps/web/
├── index.html
├── vite.config.ts          # Vite + Tailwind + /@ alias + /api proxy
├── tsconfig.app.json       # Strict TS, paths alias
├── package.json
└── src/
    ├── main.tsx            # React root mount
    ├── App.tsx             # Router tree + QueryClientProvider
    ├── index.css           # Tailwind import only
    │
    ├── lib/
    │   ├── api/
    │   │   ├── client.ts   # Axios instance, token attach/refresh, all API calls
    │   │   ├── types.ts    # TypeScript types mirroring server DTOs exactly
    │   │   └── index.ts    # Re-exports
    │   └── utils.ts        # formatBytes, formatDate
    │
    ├── stores/
    │   └── authStore.ts    # Zustand: isAuthenticated, logout()
    │
    ├── components/
    │   ├── ui/
    │   │   ├── Button.tsx  # Variants: primary / secondary / danger / ghost
    │   │   ├── Input.tsx   # Labelled input with error display
    │   │   ├── Card.tsx    # Card / CardHeader / CardBody
    │   │   └── Spinner.tsx # Loading indicator
    │   ├── AppLayout.tsx   # Sidebar shell (Files, Trash, Storage, Settings, Logout)
    │   └── ProtectedRoute.tsx  # Redirects unauthenticated users to /login
    │
    └── pages/
        ├── LoginPage.tsx         # Password login + OAuth provider buttons
        ├── RegisterPage.tsx      # Account creation form
        ├── OAuthCallbackPage.tsx # Reads tokens from URL params after OAuth redirect
        ├── FilesPage.tsx         # Paginated file list, delete, download
        ├── UploadModal.tsx       # Chunked upload with progress bars (8 parallel streams)
        ├── TrashPage.tsx         # Soft-deleted files, restore button
        ├── StoragePage.tsx       # Data sources + bucket list/create
        └── SettingsPage.tsx      # Linked OAuth providers + account audit log
```

---

## API Integration Model

The UI and backend are **integrated only through the REST API contract**. This
is the same contract used by the CLI (`apps/cli/`) and any future mobile client.

```
src/lib/api/types.ts    ←→    apps/server/src/api/files.rs
                         ←→    apps/server/src/api/auth.rs
                         ←→    apps/server/src/api/oauth.rs
                         ←→    apps/server/src/api/storage.rs
```

If a server DTO changes, update `types.ts` — TypeScript will immediately
surface every call site that needs updating. No other coupling exists.

### Authentication Flow

```
1. User submits login form
     ↓
2. POST /api/v1/auth/login → { access_token, refresh_token, expires_in }
     ↓
3. Tokens stored in localStorage (client.ts: saveTokens)
     ↓
4. Zustand authStore.isAuthenticated = true → ProtectedRoute allows access
     ↓
5. Every subsequent request: Axios interceptor attaches Bearer token
     ↓
6. On 401: Axios interceptor calls POST /api/v1/auth/refresh, retries once
     ↓
7. Logout: DELETE tokens from localStorage, isAuthenticated = false → /login
```

### OAuth Flow

```
1. User clicks "GitHub" button
     ↓
2. Browser navigates to GET /api/v1/auth/oauth/github (server-side)
     ↓
3. Server redirects to GitHub, user authorises
     ↓
4. GitHub redirects to GET /api/v1/auth/oauth/github/callback (server)
     ↓
5. Server exchanges code for tokens, creates/finds user, issues JWT pair
     ↓
6. Server redirects to /oauth/callback?access_token=...&refresh_token=...
     ↓
7. OAuthCallbackPage.tsx reads params, calls saveTokens(), navigates to /
```

---

## Development vs Production

### Development

- Vite dev server on `:5173` with HMR (instant hot reload on save)
- All `/api/**` requests proxied to `:8080` by Vite — **no CORS needed**
- TypeScript errors shown inline in browser console via Vite overlay
- React strict mode enabled (double-renders in dev to catch side effects)

### Production Build

```powershell
cd apps/web
npm run build
# Output: dist/  (static HTML + JS + CSS)
```

Serve `dist/` with any static file host — nginx, Cloudflare Pages, AWS S3 +
CloudFront, Vercel, etc. Point your reverse proxy so that `/api/**` routes to
the Rust server and everything else serves `index.html` (for client-side routing).

Example nginx config:

```nginx
server {
    listen 443 ssl;
    root /var/www/freebox;
    index index.html;

    # Serve the React SPA — all unknown paths return index.html
    location / {
        try_files $uri $uri/ /index.html;
    }

    # Proxy API requests to the Rust backend
    location /api/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

---

## Future Mobile / Desktop Clients

Because the UI is decoupled from the backend, adding additional clients requires
**zero backend changes** for features already supported by the API:

| Future client | What to build |
|---|---|
| React Native (iOS/Android) | Reuse `lib/api/client.ts` (works in RN with AsyncStorage for tokens) |
| Electron (desktop) | Wrap the web build — zero changes needed |
| Flutter | Implement the same API calls with `dio` HTTP client |
| CLI (already exists) | `apps/cli/` — Rust, using the same server endpoints |

The only backend extension needed for mobile is push notifications
(`/api/v1/push/register`) — everything else already works.

---

## Known Limitations (Next Steps)

| Item | Status |
|---|---|
| Client-side Argon2 password hashing | Planned — WASM crypto module |
| Signal Protocol prekey generation in browser | Planned — WASM crypto module |
| File decryption on download | Planned — requires the WASM crypto module |
| File rename UI (PATCH endpoint exists) | Not yet wired into FilesPage |
| Admin audit log page | Not yet built (endpoint exists: `/api/v1/admin/audit-events`) |
| Dark mode | Tailwind `dark:` classes — straightforward to add |
