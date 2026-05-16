#!/usr/bin/env pwsh
<#
.SYNOPSIS
    FreeBox developer CLI — build and run the Rust server and React web UI.

.DESCRIPTION
    fbx build   Build both the Rust server and the React web app. Shows errors
                from both without starting anything.

    fbx run     Build and start both servers, then open http://localhost:5173
                in your browser. Streams colour-coded output from each process
                in a single terminal window. Press Ctrl+C to stop everything.

    fbx check   Run Rust and TypeScript type-checkers only (faster than a full
                build — no binary produced, no dist folder written).

.EXAMPLE
    .\fbx.ps1 build
    .\fbx.ps1 run
    .\fbx.ps1 check
#>

param(
    [Parameter(Position = 0)]
    [ValidateSet('build', 'run', 'stop', 'check', 'help')]
    [string]$Command = 'help'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------

$RepoRoot       = $PSScriptRoot
$WebDir         = Join-Path $RepoRoot 'apps\web'
$EnvFile        = Join-Path $RepoRoot '.env'
$ComposeFile    = Join-Path $RepoRoot 'infra\docker-compose.dev.yml'

$WebPort    = 5173
$ServerPort = 8080
$PostgresPort = 5432

# ---------------------------------------------------------------------------
# Colour helpers
# ---------------------------------------------------------------------------

function Write-Header([string]$Text) {
    Write-Host ""
    Write-Host "  $Text" -ForegroundColor White -BackgroundColor DarkBlue
    Write-Host ""
}

function Write-Step([string]$Text) {
    Write-Host "  >> $Text" -ForegroundColor DarkCyan
}

function Write-Ok([string]$Text) {
    Write-Host "  OK  $Text" -ForegroundColor Green
}

function Write-Fail([string]$Text) {
    Write-Host "  !!  $Text" -ForegroundColor Red
}

function Write-Warn([string]$Text) {
    Write-Host "  **  $Text" -ForegroundColor Yellow
}

function Write-ServerLine([string]$Line) {
    Write-Host "[server] $Line" -ForegroundColor Cyan
}

function Write-WebLine([string]$Line) {
    Write-Host "   [web] $Line" -ForegroundColor Green
}

function Write-InfraLine([string]$Line) {
    Write-Host " [infra] $Line" -ForegroundColor DarkYellow
}

# Convert a Windows absolute path to the WSL /mnt/<drive>/... equivalent.
# Avoids calling 'wsl wslpath' which loses backslashes via argument expansion.
function ConvertTo-WslPath([string]$WinPath) {
    $drive = $WinPath.Substring(0, 1).ToLower()
    $rest  = $WinPath.Substring(2).Replace('\', '/')
    return "/mnt/$drive$rest"
}

# ---------------------------------------------------------------------------
# .env loader — sets env vars in the current process so child processes
# (and Start-Job sub-shells) inherit them.
# ---------------------------------------------------------------------------

function Import-DotEnv {
    if (-not (Test-Path $EnvFile)) {
        $EnvExample = Join-Path $RepoRoot '.env.example'
        if (Test-Path $EnvExample) {
            Write-Warn ".env not found — auto-creating from .env.example (safe for local dev)."
            # Strip inline comments (e.g. VALUE=123  # note) line-by-line.
            $lines = Get-Content $EnvExample
            $lines = $lines | ForEach-Object {
                if ($_ -match '^(\s*[A-Za-z_][A-Za-z0-9_]*\s*=\s*\S+)\s+#') {
                    $Matches[1]
                } else {
                    $_
                }
            }

            # /tmp/... is not ideal on Windows; use a local data folder instead.
            $localData = (Join-Path $RepoRoot 'data\freebox-storage').Replace('\', '/')
            $lines = $lines | ForEach-Object { $_ -replace 'STORAGE_LOCAL_ROOT=.*', "STORAGE_LOCAL_ROOT=$localData" }

            # 'localhost' resolves to ::1 (IPv6) on Windows; WSL2 Docker port
            # forwarding only reliably works on 127.0.0.1 (IPv4).
            $lines = $lines | ForEach-Object { $_ -replace '@localhost:', '@127.0.0.1:' }
            $lines = $lines | ForEach-Object { $_ -replace '://localhost:', '://127.0.0.1:' }

            Set-Content -Path $EnvFile -Value $lines -Encoding UTF8
            Write-Ok "Created .env (review and customise JWT_SECRET before going to production)"
        } else {
            Write-Fail ".env and .env.example both missing. Cannot start server."
            exit 1
        }
    }

    foreach ($line in Get-Content $EnvFile) {
        # Skip blank lines and comments
        if ($line -match '^\s*$' -or $line -match '^\s*#') { continue }

        # KEY=value  (value may contain = signs)
        if ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$') {
            $key   = $Matches[1]
            $value = $Matches[2].Trim()

            # Strip surrounding quotes if present
            if ($value -match '^"(.*)"$' -or $value -match "^'(.*)'$") {
                $value = $Matches[1]
            }

            [System.Environment]::SetEnvironmentVariable($key, $value, 'Process')
        }
    }

    Write-Ok "Loaded $EnvFile"
}

# ---------------------------------------------------------------------------
# Docker infra — start all services and wait for Postgres to be healthy
# ---------------------------------------------------------------------------

function Start-Infra {
    # Verify WSL2 is available (we run docker compose inside WSL to avoid
    # Docker Desktop corporate sign-in enforcement via Registry policy).
    if (-not (Get-Command wsl -ErrorAction SilentlyContinue)) {
        Write-Fail "wsl not found. Enable WSL2 and install Docker inside it."
        exit 1
    }

    # Convert Windows compose-file path to a WSL Linux path.
    $wslCompose = ConvertTo-WslPath $ComposeFile

    Write-Step "Starting Docker services (postgres, dragonfly, minio)..."
    wsl docker compose -f $wslCompose up -d 2>&1 | ForEach-Object { Write-InfraLine $_ }

    if ($LASTEXITCODE -ne 0) {
        Write-Fail "docker compose up failed (exit $LASTEXITCODE)"
        exit 1
    }

    $pgReady = Wait-ForPort -Port $PostgresPort -TimeoutSeconds 30 -Label "PostgreSQL (:$PostgresPort)"
    if (-not $pgReady) {
        Write-Fail "PostgreSQL did not start in time. Check: docker compose -f infra/docker-compose.dev.yml logs postgres"
        exit 1
    }

    # TCP port open != PostgreSQL ready.  The postgres process binds the port
    # early during startup, but it can't accept protocol connections until
    # initialisation finishes (WAL replay, shared-memory setup, etc.).
    # Docker's health-check runs `pg_isready` and waits for the full startup.
    Write-Step "Waiting for PostgreSQL to be healthy (pg_isready)..."
    $healthDeadline = (Get-Date).AddSeconds(30)
    $pgHealthy = $false
    while ((Get-Date) -lt $healthDeadline) {
        $status = wsl docker inspect --format '{{.State.Health.Status}}' freebox-postgres 2>&1
        if ($status -eq 'healthy') { $pgHealthy = $true; break }
        Start-Sleep -Milliseconds 500
    }
    if (-not $pgHealthy) {
        Write-Fail "PostgreSQL container is not healthy. Check: wsl docker logs freebox-postgres"
        exit 1
    }

    Write-Ok "Docker services running (postgres:$PostgresPort, dragonfly:6379, minio:19000)"
}

function Stop-Infra {
    if (-not (Get-Command wsl -ErrorAction SilentlyContinue)) {
        Write-Fail "wsl not found. Enable WSL2 and install Docker inside it."
        exit 1
    }

    $wslCompose = ConvertTo-WslPath $ComposeFile

    Write-Step "Stopping Docker services..."
    wsl docker compose -f $wslCompose down 2>&1 | ForEach-Object { Write-InfraLine $_ }

    if ($LASTEXITCODE -ne 0) {
        Write-Fail "docker compose down failed (exit $LASTEXITCODE)"
        exit 1
    }

    Write-Ok "Docker services stopped."
}

# ---------------------------------------------------------------------------
# Port readiness check (TCP connect, no HTTP required)
# ---------------------------------------------------------------------------

function Wait-ForPort {
    param(
        [int]$Port,
        [int]$TimeoutSeconds = 60,
        [string]$Label = "port $Port"
    )

    Write-Step "Waiting for $Label to be ready..."
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)

    while ((Get-Date) -lt $deadline) {
        foreach ($addr in @('127.0.0.1', '::1')) {
            try {
                $client = [System.Net.Sockets.TcpClient]::new()
                $ar = $client.BeginConnect($addr, $Port, $null, $null)
                if ($ar.AsyncWaitHandle.WaitOne(500)) {
                    $client.EndConnect($ar)
                    $client.Close()
                    return $true
                }
                $client.Close()
            } catch {
                # port not ready yet
            }
        }
        Start-Sleep -Milliseconds 200
    }

    Write-Fail "$Label did not become ready within $TimeoutSeconds seconds."
    return $false
}

# ---------------------------------------------------------------------------
# fbx build
# ---------------------------------------------------------------------------

function Invoke-Build {
    Write-Header "fbx build"

    # --- Rust ---
    Write-Step "cargo build -p freebox-server"
    Push-Location $RepoRoot
    try {
        cargo build -p freebox-server
        if ($LASTEXITCODE -ne 0) {
            Write-Fail "Rust build failed (exit $LASTEXITCODE)"
            exit 1
        }
        Write-Ok "Rust server built successfully"
    } finally {
        Pop-Location
    }

    # --- TypeScript / React ---
    Write-Step "pnpm --filter @freebox/web build"
    Push-Location $WebDir
    try {
        pnpm run build
        if ($LASTEXITCODE -ne 0) {
            Write-Fail "Web build failed (exit $LASTEXITCODE)"
            exit 1
        }
        Write-Ok "Web app built successfully"
    } finally {
        Pop-Location
    }

    Write-Host ""
    Write-Ok "All builds passed. No errors."
}

# ---------------------------------------------------------------------------
# fbx check  (type-check only, fastest feedback loop)
# ---------------------------------------------------------------------------

function Invoke-Check {
    Write-Header "fbx check"

    # --- Rust clippy (warnings-as-errors for a strict check) ---
    Write-Step "cargo clippy -p freebox-server"
    Push-Location $RepoRoot
    try {
        cargo clippy -p freebox-server -- -D warnings
        if ($LASTEXITCODE -ne 0) {
            Write-Fail "Rust clippy found errors (exit $LASTEXITCODE)"
            exit 1
        }
        Write-Ok "Rust clippy clean"
    } finally {
        Pop-Location
    }

    # --- TypeScript type-check only (no emit) ---
    Write-Step "tsc --noEmit (web)"
    Push-Location $WebDir
    try {
        pnpm exec tsc --noEmit
        if ($LASTEXITCODE -ne 0) {
            Write-Fail "TypeScript type errors found (exit $LASTEXITCODE)"
            exit 1
        }
        Write-Ok "TypeScript types clean"
    } finally {
        Pop-Location
    }

    Write-Host ""
    Write-Ok "All checks passed."
}

# ---------------------------------------------------------------------------
# fbx run
# ---------------------------------------------------------------------------

function Invoke-Run {
    Write-Header "fbx run"

    # Kill any processes already holding the ports we need. This prevents
    # `cargo run` failing with 'address in use' after a previous fbx run.
    foreach ($port in @($ServerPort, $WebPort)) {
        $owners = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue |
                  Select-Object -ExpandProperty OwningProcess -Unique
        foreach ($pid in $owners) {
            if ($pid -and $pid -ne 0) {
                $proc = Get-Process -Id $pid -ErrorAction SilentlyContinue
                if ($proc) {
                    Write-Step "Stopping $($proc.ProcessName) (PID $pid) on :$port ..."
                    Stop-Process -Id $pid -Force -ErrorAction SilentlyContinue
                }
            }
        }
    }

    # Build the server binary upfront in the main process so compilation
    # output is visible and errors stop the script before anything starts.
    # Running `cargo run` inside a background job causes the job to finish
    # (State = Completed) when cargo exits after compilation even if the
    # binary itself ran fine — the startup crash-detector misreads this.
    Write-Step "Building Rust server..."
    $buildOutput = cargo build -p freebox-server 2>&1
    $buildOutput | ForEach-Object { Write-ServerLine $_ }
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "Rust server build failed — fix errors above and retry."
        exit 1
    }
    Write-Ok "Rust server binary ready."

    Start-Infra
    Import-DotEnv

    # Capture the current env snapshot so both jobs inherit it.
    $envSnapshot = @{}
    [System.Environment]::GetEnvironmentVariables('Process').GetEnumerator() | ForEach-Object {
        $envSnapshot[$_.Key] = $_.Value
    }

    # Pre-built binary path — avoids cargo overhead and the compile-in-job
    # false-positive crash detection.
    $serverBin = Join-Path $RepoRoot 'target\debug\freebox-server.exe'

    # --- Start Rust server job ---
    Write-Step "Starting Rust server on :$ServerPort ..."

    $serverJob = Start-Job -Name 'freebox-server' -ScriptBlock {
        param($bin, $root, $envMap)
        foreach ($kv in $envMap.GetEnumerator()) {
            [System.Environment]::SetEnvironmentVariable($kv.Key, $kv.Value, 'Process')
        }
        Set-Location $root
        & $bin 2>&1
    } -ArgumentList $serverBin, $RepoRoot, $envSnapshot

    # --- Start Vite dev server job ---
    Write-Step "Starting React dev server on :$WebPort ..."

    $webJob = Start-Job -Name 'freebox-web' -ScriptBlock {
        param($webDir, $envMap)
        foreach ($kv in $envMap.GetEnumerator()) {
            [System.Environment]::SetEnvironmentVariable($kv.Key, $kv.Value, 'Process')
        }
        Set-Location $webDir
        pnpm run dev 2>&1
    } -ArgumentList $WebDir, $envSnapshot

    # -----------------------------------------------------------------------
    # Unified startup loop: stream output from both jobs in real time while
    # waiting for both ports. This makes compilation progress visible and lets
    # us bail out immediately if the server crashes instead of waiting 120 s.
    # -----------------------------------------------------------------------

    function Test-Port([int]$Port) {
        foreach ($addr in @('127.0.0.1', '::1')) {
            try {
                $c = [System.Net.Sockets.TcpClient]::new()
                $ar = $c.BeginConnect($addr, $Port, $null, $null)
                if ($ar.AsyncWaitHandle.WaitOne(100)) { $c.EndConnect($ar); $c.Close(); return $true }
                $c.Close()
            } catch {}
        }
        return $false
    }

    Write-Step "Waiting for Vite (:$WebPort) and Rust server (:$ServerPort) — streaming output below..."
    $viteReady   = $false
    $serverReady = $false
    $deadline    = (Get-Date).AddSeconds(180)

    while ((Get-Date) -lt $deadline) {
        # Stream whatever each job has printed since the last poll.
        Receive-Job -Job $serverJob -ErrorAction SilentlyContinue | ForEach-Object { Write-ServerLine $_ }
        Receive-Job -Job $webJob    -ErrorAction SilentlyContinue | ForEach-Object { Write-WebLine    $_ }

        # Bail out fast if the server process died during startup.
        if ($serverJob.State -in 'Failed','Completed') {
            Write-Fail "Rust server crashed during startup — last output shown above."
            Write-Fail "Check the [server] lines for the error (config missing, DB refused, etc.)"
            Receive-Job -Job $serverJob -ErrorAction SilentlyContinue | ForEach-Object { Write-ServerLine $_ }
            Stop-Job  -Job $webJob -ErrorAction SilentlyContinue
            Remove-Job -Job $serverJob, $webJob -Force -ErrorAction SilentlyContinue
            exit 1
        }

        if (-not $viteReady   -and (Test-Port $WebPort))    { $viteReady   = $true; Write-Ok "Vite ready   (:$WebPort)" }
        if (-not $serverReady -and (Test-Port $ServerPort)) { $serverReady = $true; Write-Ok "Server ready (:$ServerPort)" }
        if ($viteReady -and $serverReady) { break }

        Start-Sleep -Milliseconds 250
    }

    if ($viteReady -and $serverReady) {
        Write-Ok "Both servers ready — opening http://localhost:$WebPort"
        Start-Process "http://localhost:$WebPort"
    } elseif ($viteReady) {
        Write-Warn "Rust server did not become ready within 3 minutes."
        Write-Warn "Opening UI anyway — check [server] output above for errors."
        Start-Process "http://localhost:$WebPort"
    } else {
        Write-Fail "Neither server became ready. Check output above."
        Stop-Job  -Job $serverJob, $webJob -ErrorAction SilentlyContinue
        Remove-Job -Job $serverJob, $webJob -Force -ErrorAction SilentlyContinue
        exit 1
    }

    Write-Host ""
    Write-Host "  Press Ctrl+C to stop both servers." -ForegroundColor DarkGray
    Write-Host ""

    # --- Stream output from both jobs until Ctrl+C ---
    try {
        while ($true) {
            $serverLines = Receive-Job -Job $serverJob -ErrorAction SilentlyContinue
            $webLines    = Receive-Job -Job $webJob    -ErrorAction SilentlyContinue

            foreach ($line in $serverLines) { Write-ServerLine $line }
            foreach ($line in $webLines)    { Write-WebLine    $line }

            # Exit if either process died unexpectedly
            if ($serverJob.State -in 'Failed','Completed') {
                Receive-Job -Job $serverJob -ErrorAction SilentlyContinue | ForEach-Object { Write-ServerLine $_ }
                Write-Fail "Rust server exited unexpectedly — see [server] output above."
                break
            }
            if ($webJob.State -in 'Failed','Completed') {
                Receive-Job -Job $webJob -ErrorAction SilentlyContinue | ForEach-Object { Write-WebLine $_ }
                Write-Fail "Vite dev server exited unexpectedly."
                break
            }

            Start-Sleep -Milliseconds 200
        }
    } finally {
        Write-Host ""
        Write-Step "Stopping servers..."
        Stop-Job  -Job $serverJob, $webJob -ErrorAction SilentlyContinue
        Remove-Job -Job $serverJob, $webJob -Force -ErrorAction SilentlyContinue
        Write-Ok "Stopped."
    }
}

# ---------------------------------------------------------------------------
# Help
# ---------------------------------------------------------------------------

function Show-Help {
    Write-Host @"

  FreeBox developer CLI

  USAGE
      .\fbx.ps1 <command>

  COMMANDS
      build   Compile the Rust server and build the React app.
              Reports all errors without starting anything.

      run     Start Docker infra, build and run both servers, open the browser.
              Streams colour-coded output in this terminal.
              Press Ctrl+C to stop the servers (Docker keeps running).

      stop    Stop all Docker services (postgres, dragonfly, minio).
              Run this when you are done for the day.

      check   Run Rust clippy and TypeScript type-checker only.
              Fastest way to catch errors during development.

  PORTS
      Rust API server   http://localhost:$ServerPort
      React UI (Vite)   http://localhost:$WebPort
      PostgreSQL        localhost:$PostgresPort
      MinIO console     http://localhost:19001

  PREREQUISITES
      - Docker Desktop installed and running
      - .env file at the repo root (copy from .env.example)
      - Rust toolchain (cargo) installed
      - Node.js >= 20 and pnpm >= 9 installed

"@
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------

switch ($Command) {
    'build' { Invoke-Build }
    'run'   { Invoke-Run   }
    'stop'  { Stop-Infra   }
    'check' { Invoke-Check }
    'help'  { Show-Help    }
    default { Show-Help    }
}
