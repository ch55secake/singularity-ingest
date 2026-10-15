# Singularity Ingest

Singularity Ingest is a lightweight Windows/Linux client that watches Steam's
local HTTP cache for Deadlock replay URLs and submits only the resulting match
IDs and salts to a configured ingestion endpoint. It also watches the game's
console log and tells the platform when a match starts and ends, so the Discord
bot can post both teams and the platform can fetch the match afterwards.

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

The collector reads at most the first 200 bytes of cache files. It scans
recursively at startup, watches filesystem notifications, and repeats the
recursive scan every five minutes to catch missed notifications.

### Match start and end

The collector finds the Deadlock install from Steam's library folders and
follows `game/citadel/console.log`, which the game writes when it is launched
with `-condebug`. When the game logs `Lobby … for Match <match id> created` (a
match has started) or `… destroyed` (the player left it), the collector sends:

```json
{"match_id": 117420947, "event": "started", "account_id": 1003816066}
```

to `/ingest/live` next to the configured `/ingest/salts` URL (override it with
`SINGULARITY_LIVE_URL`). `account_id` is the Steam account that last logged in
on the PC. Reports are off when the ingest URL is not a Singularity URL.

To add `-condebug -conclearlog` to Deadlock's launch options (`-conclearlog`
starts the log afresh on each launch), close Steam and run:

```powershell
& "$env:LOCALAPPDATA\deadlock-api-ingest\deadlock-api-ingest.exe" --enable-console-log
```

`--check-console-log` exits 0 when the flag is already there and 3 when not.
Steam rewrites its config when it exits, so the change is lost if Steam is
running. The installer closes and restarts Steam for this step. A backup of the
config is written next to it as `localconfig.vdf.singularity-backup`.

## Configuration

The collector has no public Deadlock API fallback. Set the private Singularity
salts endpoint and collector token before starting it:

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

Install in one step from PowerShell, signed in as the Windows user who plays
Deadlock (administrator rights are not needed):

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/ch55secake/singularity-ingest/master/install-windows.ps1))) -Token <collector token>
```

The installer:

1. stores the ingest URL (default `http://192.168.1.110:8080/ingest/salts`,
   change it with `-IngestUrl`) and the collector token in the Windows user
   environment; without `-Token` it keeps a saved token or asks for one;
2. downloads the latest Windows release from this fork;
3. if Deadlock doesn't already launch with `-condebug`, closes Steam, adds
   `-condebug -conclearlog` to the launch options and starts Steam again (skip
   this with `-SkipConsoleLog`; it is skipped while Deadlock runs);
4. creates and starts a hidden logon task (`conhost.exe --headless`) for the
   current user (use `-NoAutoStart` for desktop shortcuts instead).

A release must exist in the fork before running the installer.

Cache-only mode is enabled by default:

```powershell
& "$env:LOCALAPPDATA\deadlock-api-ingest\deadlock-api-ingest.exe" --no-gc
```

Scan the existing cache once and exit:

```powershell
& "$env:LOCALAPPDATA\deadlock-api-ingest\deadlock-api-ingest.exe" --no-gc --once
```

The installer can create a scheduled task. Configure the task to pass
`--no-gc`; do not enable automatic GC recovery for Singularity's cache-only
deployment.

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
