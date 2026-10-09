# Security and Privacy

## Data accessed

Singularity Ingest reads Steam's local `appcache/httpcache` directory. It
extracts replay URLs and sends match IDs, cluster IDs, salts, and optionally a
Steam3 account attribution.

It does not read Steam passwords, saved refresh tokens, game memory, or replay
contents in cache-only mode.

## Deployment requirements

- Use a per-client ingestion token.
- Send requests over Tailscale, a private LAN, or HTTPS.
- Never commit tokens to this repository.
- Use the cache-only baseline; the Game Coordinator module is not compiled into
  the Singularity client.
- Obtain consent from every player whose match data is collected.

## Reporting

Do not open public issues containing match salts or bearer tokens. Rotate a
compromised ingestion token immediately and remove affected data from the
platform.
