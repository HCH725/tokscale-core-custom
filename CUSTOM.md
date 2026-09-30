# tokscale-core Custom

This private repository is the canonical tokscale-core dependency for the private Syrtis consumer in `HCH725/TokenBar-custom`.

## Repository relationship

- `origin`: `HCH725/tokscale-core-custom` — private canonical downstream.
- `upstream`: `Nanako0129/tokscale-core` — official source; the current Syrtis v2.2 rebase baseline is `319ffa8ca75f6cd2bfaf96ae0d295a8fa618ec2c`.
- The private Syrtis consumer pins an exact reviewed commit from this repository as its `vendor/tokscale-core` submodule.

## Current downstream contract

CatDesk exposes an append-only local ledger at `~/.catdesk/usage.jsonl` with one row per recorded MCP tool call:

```json
{"eventId": "<uuid>", "timestampMs": 1788210000123, "inputTokens": 12, "outputTokens": 8, "bucket": "through-gpt-5.6", "pricingModel": "gpt-5.6-sol"}
```

The Hermes source lane must:

- discover that ledger only when the `hermes` client is enabled;
- ingest it alongside Hermes SQLite/profile databases without writing to either source;
- emit `client=hermes`, `model=catdesk-mcp`, `provider=catdesk`;
- include CatDesk tokens in normal Hermes and global usage reports, translating MCP direction correctly: ledger `outputTokens` are model input and ledger `inputTokens` are model output;
- keep the displayed identity `model=catdesk-mcp`, `provider=catdesk`, but use `pricingModel` only as the pricing lookup identity so CatDesk remains separately attributable;
- for legacy rows written before `pricingModel` existed, use the explicitly confirmed historical fallback `gpt-5.6-sol`; never infer a model from the broad `through-gpt-5.6` accounting bucket;
- run CatDesk cost through tokscale's normal pricing service and mark it estimated/partially estimated according to the same coverage contract as other sources;
- use `eventId` as the stable deduplication identity so moving/reordering rows does not change an event's identity; legacy rows without `eventId` may use a content-derived fallback;
- ignore malformed/partial rows without discarding valid rows;
- include ledger mtime/path changes in normal source-cache invalidation;
- never invent timestamped history for CatDesk usage that predates the ledger.

The v1 ledger is intentionally append-only and unrotated. A linear scan is accepted for this small local source; do not add a database, rotation service, or background index until measured ledger size/latency justifies it.

### OpenCode Go pricing

Hermes usage attributed to `provider=opencode-go` (or normalized `opencode_go`) uses OpenCode Go's official usage-value pricing before the generic pricing catalog. The adapter is provider-scoped, preserves provider-reported cost as authoritative, and falls back to the existing pricing service for unknown Go models.

Current covered Go models are `mimo-v2.5`, `mimo-v2.5-pro`, `muse-spark-1.2-contributor`, `deepseek-v4-pro`, `deepseek-v4-flash`, and `deepseek-v4-flash-vision-exp`. DeepSeek V4 applies the official weekday UTC peak windows `[01:00,04:00)` and `[06:00,10:00)`; weekends and all other times are off-peak. Hermes `output_tokens` already represents billable completion output, so the separate `reasoning_tokens` field must not be charged again on this path. Pricing source: `https://opencode.ai/docs/go/`.

### Codex subscription equivalents

Hermes `openai-codex` rows explicitly marked `cost_status=included` with `billing_mode=subscription_included` or `codex_responses` keep authoritative raw incremental cost `$0`. A separate recorded-model ChatGPT Work/Codex rate-card equivalent is exposed for attribution/reporting only; it must never overwrite provider-reported cost or be interpreted as 5-hour/weekly quota depletion.

### Temporary Syrtis v2.2 baseline remediations

Public baseline `319ffa8` carries two Droid reply-count defects found while reviewing the private rebase: multi-day reply counts ride on the first fragment, and local client counts use fragment count instead of the fragments' coalesced reply totals. This fork keeps each fragment's own `message_count`, sums those counts for the Droid local-client total, and bumps only Droid `parser_version` from 2 to 3 so unchanged cached parser output is rebuilt; `CACHE_FORMAT_VERSION` is unchanged. Drop these hunks when upstream incorporates an equivalent fix.

The same review found account-only CodeRabbit billing/trial details embedded in configuration comments. Those details are intentionally generalized here without changing CodeRabbit behavior; repository configuration must not carry account billing metadata.

## Update workflow

When the public core revision used by Syrtis changes, start from that new upstream revision, re-evaluate every private delta, port only the minimum surviving CatDesk/OpenCode Go/Codex contract, run `git diff --check`, targeted rustfmt on touched Rust files, the complete `cargo test` suite, and clippy, then update Syrtis's submodule pointer only after independent review/audit.
