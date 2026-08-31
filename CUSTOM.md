# tokscale-core Custom

This private repository is the canonical tokscale-core dependency for `HCH725/TokenBar-custom`.

## Repository relationship

- `origin`: `HCH725/tokscale-core-custom` — private canonical downstream.
- `upstream`: `Nanako0129/tokscale-core` — official source.
- TokenBar pins an exact commit from this repository as its `vendor/tokscale-core` submodule.

## Current downstream contract

CatDesk exposes an append-only local ledger at `~/.catdesk/usage.jsonl` with one row per recorded MCP tool call:

```json
{"eventId": "<uuid>", "timestampMs": 1788210000123, "inputTokens": 12, "outputTokens": 8, "bucket": "through-gpt-5.6"}
```

The Hermes source lane must:

- discover that ledger only when the `hermes` client is enabled;
- ingest it alongside Hermes SQLite/profile databases without writing to either source;
- emit `client=hermes`, `model=catdesk-mcp`, `provider=catdesk`;
- include CatDesk input/output tokens in normal Hermes and global usage reports;
- keep CatDesk cost at `0`/unknown until authoritative model/provider pricing is available;
- use `eventId` as the stable deduplication identity so moving/reordering rows does not change an event's identity; legacy rows without `eventId` may use a content-derived fallback;
- ignore malformed/partial rows without discarding valid rows;
- include ledger mtime/path changes in normal source-cache invalidation;
- never invent timestamped history for CatDesk usage that predates the ledger.

The v1 ledger is intentionally append-only and unrotated. A linear scan is accepted for this small local source; do not add a database, rotation service, or background index until measured ledger size/latency justifies it.

## Update workflow

When the upstream core revision used by TokenBar changes, start from that new upstream revision, re-evaluate whether this custom parser is still needed, port only the minimum surviving delta, run `git diff --check` and the complete `cargo test` suite, then update TokenBar's submodule pointer only after review/audit.
