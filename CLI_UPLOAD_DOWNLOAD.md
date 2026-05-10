# FreeBox CLI — Upload, Download & Storage Sources

Everything runs through the `fbx` binary. Files are **encrypted on your device
before they leave** — your storage provider (Cloudflare R2, AWS S3, MinIO, …)
only ever stores opaque ciphertext it cannot read.

---

## Prerequisites

### Build the binary

```powershell
cd "C:\Personal projects\FreeBox"
cargo build -p fbx
# Binary is at: target\debug\fbx.exe
```

### Make `fbx` available in your terminal (one-time setup)

**Option A — add the build output folder to your PATH (recommended for development)**

Run this once in PowerShell (persists across restarts):

```powershell
$fbxDir = "C:\Personal projects\FreeBox\target\debug"
[Environment]::SetEnvironmentVariable(
    "PATH",
    [Environment]::GetEnvironmentVariable("PATH","User") + ";$fbxDir",
    "User"
)
```

Then **open a new terminal** and verify:

```powershell
fbx --version
# fbx 0.1.0
```

**Option B — copy the binary to a folder already on your PATH**

```powershell
# Example: copy to C:\Users\<you>\bin  (create it first if needed)
Copy-Item "C:\Personal projects\FreeBox\target\debug\fbx.exe" "$env:USERPROFILE\bin\fbx.exe"
```

**Option C — run with full path (no setup needed)**

```powershell
& "C:\Personal projects\FreeBox\target\debug\fbx.exe" auth register
```

> **Note:** After running `cargo build` again, Option A and B automatically pick up the latest binary. For a production build use `cargo build --release -p fbx` and the binary will be at `target\release\fbx.exe`.

---

## 1. Connect to your FreeBox server

The CLI talks to a FreeBox server, which in turn stores encrypted chunks in
whichever storage backend is configured. You need to register once, then log in
on each machine.

### Register a new account

```bash
fbx auth register
# Prompts: Username, Email, Password
# Generates your Signal Protocol key pair automatically
```

### Log in

```bash
fbx auth login
# Prompts: Username, Password
# Stores a session token at ~/.config/freebox/session.json
```

### Check who you are logged in as

```bash
fbx auth whoami
# alice (alice@example.com) on https://freebox.io
```

### Log out

```bash
fbx auth logout
```

---

## 2. Manage storage sources

Storage sources are the cloud backends that hold your encrypted files. You
register them in the CLI config so you can track, test, and reference them
easily. The server reads its active backend from environment variables (see
**Server configuration** below).

### List configured sources

```bash
fbx provider list
```

Example output:

```
NAME                 TYPE       ENDPOINT / BUCKET                             REGION
------------------------------------------------------------------------------------------
cloudflare-r2        cloudflare-r2  https://abc123.r2.cloudflarestorage.com/my-bucket  auto
```

### Add a new source

```bash
fbx provider add cloudflare-r2
```

Interactive prompts:

```
Source name (e.g. cloudflare-r2): cloudflare-r2
Endpoint URL: https://<YOUR_ACCOUNT_ID>.r2.cloudflarestorage.com
Bucket name: freebox-storage
Region [auto]:
Access Key ID: <your-r2-access-key-id>
Secret Access Key: ********
```

After adding, the command prints the exact environment variables to set on your
server (see **Server configuration** below).

Other supported provider types you can pass directly:

| Type | Example |
|---|---|
| `cloudflare-r2` | `fbx provider add cloudflare-r2` |
| `aws-s3` | `fbx provider add aws-s3` |
| `minio` | `fbx provider add minio` |
| `backblaze-b2` | `fbx provider add backblaze-b2` |
| `local` | `fbx provider add local` |

### Test source connectivity

```bash
fbx provider test cloudflare-r2
# ✓ Endpoint reachable (HTTP 403). Source 'cloudflare-r2' looks good.
# (403 = auth required but endpoint is live — that is correct)
```

### Remove a source

```bash
fbx provider remove cloudflare-r2
```

---

## 3. Server configuration for Cloudflare R2

Set these environment variables before starting `freebox-server`.
You can put them in a `.env` file in the server directory.

```env
# Required
DATABASE_URL=postgres://freebox:freebox_dev@localhost:5432/freebox
JWT_SECRET=change-me-to-a-long-random-string

# Storage — Cloudflare R2
STORAGE_PROVIDER=s3
STORAGE_S3_BUCKET=freebox-storage
STORAGE_S3_REGION=auto
STORAGE_S3_ENDPOINT=https://<YOUR_ACCOUNT_ID>.r2.cloudflarestorage.com
STORAGE_S3_ACCESS_KEY=<your-r2-access-key-id>
STORAGE_S3_SECRET_KEY=<your-r2-secret-access-key>
```

Get your R2 credentials from:
**Cloudflare Dashboard → R2 → Manage R2 API Tokens → Create Token**
(permission: Object Read & Write on your bucket)

Start the server:

```bash
cargo run -p freebox-server
# or in production:
./freebox-server
```

The server log confirms the backend is live:

```
INFO freebox_server: Storage provider loaded bucket=freebox-storage endpoint=https://...r2.cloudflarestorage.com
```

---

## 4. Upload files

```bash
# Upload a single file
fbx upload ./contract.pdf

# Upload to a named remote folder
fbx upload ./contract.pdf --destination remote://documents/

# Upload multiple files at once
fbx upload ./doc1.pdf ./doc2.pdf ./report.xlsx

# Control parallel chunk streams (default 8, max 16)
fbx upload ./large-video.mp4 --parallelism 16
```

What happens:
1. The file is split into 4 MB chunks
2. Each chunk is encrypted with AES-256-GCM using a unique per-file key
3. Chunks are uploaded in parallel to the server
4. The server streams ciphertext directly to Cloudflare R2
5. The encrypted key envelope is stored — only your device can unwrap it

Output:

```
[=================>    ] 3/4 chunks  contract.pdf
Uploaded contract.pdf as 3f7a1b2c-4d5e-6f7a-8b9c-0d1e2f3a4b5c
```

---

## 5. View your files

```bash
# Simple list (file names only)
fbx ls

# Full details: UUID, size, upload date, content hash
fbx ls --long

# Filter by remote path prefix
fbx ls remote://documents/
```

Example output (`--long`):

```
3f7a1b2c-4d5e-6f7a-8b9c-0d1e2f3a4b5c    245.3 KB  2026-05-08 14:32  a3f9c1d2e4b5  contract.pdf
8a9b0c1d-2e3f-4a5b-6c7d-8e9f0a1b2c3d    1.2 MB    2026-05-08 14:35  f1e2d3c4b5a6  documents/report.pdf
```

---

## 6. Download and decrypt files

The file is downloaded as ciphertext from R2 and **decrypted locally on your
device**. The decrypted plaintext is written to disk — R2 never sees it.

```bash
# By file name (exact or partial match)
fbx download contract.pdf

# By remote path
fbx download documents/report.pdf

# By UUID (from fbx ls --long)
fbx download 3f7a1b2c-4d5e-6f7a-8b9c-0d1e2f3a4b5c

# Save to a specific local path
fbx download contract.pdf --output ~/Downloads/contract.pdf

# Save into an existing folder (preserves original file name)
fbx download contract.pdf --output ~/Downloads/
```

What happens:
1. CLI fetches the encrypted metadata from the server
2. Encrypted chunks are downloaded from Cloudflare R2 (via server)
3. Chunks are decrypted locally with AES-256-GCM
4. Decrypted file is written to the output path
5. Session token is automatically refreshed if it expires mid-download

Output:

```
[=================>    ] 4/4 chunks
Downloaded to /home/alice/Downloads/contract.pdf
```

---

## 7. Delete files

```bash
# Soft delete — file moves to trash (recoverable for 30 days)
fbx rm 3f7a1b2c-4d5e-6f7a-8b9c-0d1e2f3a4b5c

# Also works by file name
fbx rm contract.pdf
```

---

## Quick reference

| Command | What it does |
|---|---|
| `fbx auth register` | Create account + generate encryption keys |
| `fbx auth login` | Log in and save session |
| `fbx auth whoami` | Show logged-in user |
| `fbx auth logout` | Clear session |
| `fbx provider list` | List configured storage sources |
| `fbx provider add <type>` | Register a new source interactively |
| `fbx provider test <name>` | Check endpoint is reachable |
| `fbx provider remove <name>` | Remove a source from config |
| `fbx upload <file(s)>` | Encrypt and upload to active backend |
| `fbx ls` | List files (names only) |
| `fbx ls --long` | List files with UUID, size, date, hash |
| `fbx download <name or UUID>` | Download and decrypt a file |
| `fbx download <name> --output <path>` | Download to a specific location |
| `fbx rm <name or UUID>` | Move file to trash |

---

## Adding future storage sources

When a new backend is added to FreeBox (GCS, Backblaze B2, Azure, IPFS, …),
the workflow is the same:

1. `fbx provider add <new-type>` — register it in your local config
2. Set the corresponding `STORAGE_*` environment variables on the server
3. Restart the server — all uploads/downloads now go through the new backend

Your encrypted files are backend-agnostic: the same ciphertext can be moved
between providers without re-encrypting.
