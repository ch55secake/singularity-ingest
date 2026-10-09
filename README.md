# Singularity Ingest

Singularity Ingest is a lightweight Windows/Linux client that watches Steam's
local HTTP cache for Deadlock replay URLs and submits only the resulting match
IDs and salts to a configured ingestion endpoint.

The default mode is **cache-only**. It does not connect to the Steam Game
Coordinator, read game memory, download replay files, or send Steam passwords.

## Data flow

```text
Steam HTTP cache
        ↓
replay URL extraction
        ↓
match ID + cluster + metadata/replay salt
        ↓ HTTPS POST
Singularity platform
```

The collector reads at most the first 200 bytes of changed cache files. It
performs one recursive scan at startup and then uses filesystem notifications.

## Configuration

The public Deadlock API remains the default destination for upstream
compatibility. A Singularity deployment should set:

```powershell
$env:SINGULARITY_INGEST_URL = "https://singularity.example/ingest/salts"
$env:SINGULARITY_INGEST_TOKEN = "replace-with-a-client-token"
```

For a Windows scheduled task, persist the values at user scope before logging
back in:

```powershell
[Environment]::SetEnvironmentVariable("SINGULARITY_INGEST_URL", "https://singularity.example/ingest/salts", "User")
[Environment]::SetEnvironmentVariable("SINGULARITY_INGEST_TOKEN", "replace-with-a-client-token", "User")
```

The token is sent as:

```text
Authorization: Bearer <token>
```

## Windows usage

Cache-only mode is enabled by default:

```powershell
& "$env:LOCALAPPDATA\deadlock-api-ingest\deadlock-api-ingest.exe" --no-gc
```

Scan the existing cache once and exit:

```powershell
& "$env:LOCALAPPDATA\deadlock-api-ingest\deadlock-api-ingest.exe" --no-gc --once
```

The existing upstream installer can create a scheduled task. Configure the
task to pass `--no-gc`; do not enable automatic GC recovery for Singularity's
cache-only deployment.

## Game Coordinator code

The upstream `src/gc/` implementation is retained for reference but is not
compiled into the Singularity baseline. The CLI has no Game Coordinator mode.
This keeps the Windows client cache-only and avoids Steam session access.

## Resource usage

The client is designed to remain idle while watching the cache. Expected usage
is a short startup disk-read burst, then negligible CPU, low memory usage, and
small HTTPS requests only when new replay URLs are discovered.

## Project

Singularity is a private friends-only match analytics project. See:

- [`SPEC.md`](SPEC.md)
- [`SECURITY.md`](SECURITY.md)
- [`UPSTREAM.md`](UPSTREAM.md)

This repository is derived from the MIT-licensed
[`deadlock-api-ingest`](https://github.com/deadlock-api/deadlock-api-ingest)
project.
