# Upstream Maintenance

The `upstream` remote points to:

```text
git@github.com:deadlock-api/deadlock-api-ingest.git
```

Singularity-specific changes are intentionally limited to:

- Cache-only behavior with the upstream GC module disabled.
- Configurable ingestion URL.
- Optional bearer authentication.
- Project documentation.

When syncing upstream changes:

1. Review changes to `src/scan_cache.rs`, `src/utils.rs`, and `src/gc/`.
2. Preserve the cache-only default.
3. Re-run unit tests and `cargo test --all-targets`.
4. Confirm no Steam credentials or GC behavior were added to the baseline.
