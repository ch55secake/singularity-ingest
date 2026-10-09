# Singularity Ingest Specification

Status: baseline v0.1.0

## Objective

Collect Deadlock replay salts from a consenting player's local Steam cache and
submit them to the private Singularity platform.

## Scope

The baseline supports:

- Windows and Linux Steam installations.
- Recursive startup cache scanning.
- Filesystem notification-based incremental scanning.
- `.meta.bz2` and `.dem.bz2` replay URL parsing.
- Configurable HTTPS ingestion endpoint.
- Optional bearer-token authentication.
- Idempotent local suppression of already-ingested salts during a process run.

## Non-goals

- Steam Game Coordinator access in the default mode.
- Steam credential collection.
- Game-memory access or process injection.
- Replay downloading or demo parsing.
- Match discovery for players whose cache is not being watched.

## Input format

The collector searches cache files for URLs shaped like:

```text
http://replay<cluster>.valve.net/1422450/<match_id>_<salt>.meta.bz2
http://replay<cluster>.valve.net/1422450/<match_id>_<salt>.dem.bz2
```

Only Deadlock app ID `1422450` is accepted.

## Output contract

The client sends a JSON array to `SINGULARITY_INGEST_URL`:

```json
[
  {
    "match_id": 42476710,
    "cluster_id": 183,
    "metadata_salt": 428480166,
    "replay_salt": null,
    "username": "ingest-tool:123456"
  }
]
```

The `username` field is an optional Steam3 account attribution. It is not a
password or authentication credential.

## Environment variables

| Variable | Required | Default |
|---|---:|---|
| `SINGULARITY_INGEST_URL` | No | Public Deadlock API salts endpoint |
| `SINGULARITY_INGEST_TOKEN` | No | No authorization header |

## Acceptance criteria

- A real cached replay URL produces one valid payload.
- Invalid hosts, app IDs, extensions, and malformed salts are ignored.
- `--no-gc` performs no Steam GC connection.
- A configured endpoint receives metadata and replay salts.
- Repeated filesystem events do not repeatedly submit the same salt in one run.
- The process remains usable with Steam running and while Deadlock is closed.
