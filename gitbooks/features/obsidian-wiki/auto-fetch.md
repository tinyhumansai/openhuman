---
description: >-
  Every twenty minutes, OpenHuman walks every active integration and folds new
  data into your memory tree. No prompts, no polling loops you have to write.
icon: arrows-rotate
---

# Auto-fetch from Integrations

Most "AI assistants" are reactive: you ask, they think, they answer. OpenHuman is the opposite. It pulls from your stack continuously, so by the time you ask "what landed in my inbox overnight?" the answer is already in the [Memory Tree](memory-tree.md).

## How it works

A single periodic scheduler ticks every twenty minutes. On each tick it walks every active [integration](../integrations/README.md), looks up the matching native provider, and, if enough time has elapsed since that connection's last sync, calls `provider.sync(ctx, SyncReason::Periodic)`.

```
every 20 min
    |
    v
for each active connection (Gmail, Notion, GitHub, ...)
    |
    +--> check sync_state (toolkit, connection_id)
    |       - last sync timestamp
    |       - daily budget
    |       - dedup set
    |       - cursor
    |
    +--> if interval elapsed -> provider.sync()
            |
            +--> on success -> record_sync_success(ts)
```

A few things matter here:

- **One global tick, not one task per connection.** The number of connections per user is small; a single 20-minute tick is enough and keeps bookkeeping trivial.
- **State is per `(toolkit, connection_id)`.** Each connection has its own cursor, its own last-sync timestamp, its own dedup set, its own daily budget. Restarts rebuild this from local KV, a missed periodic sync is harmless because the next tick after restart picks it back up.
- **Native syncs are shared with event-driven paths.** When a webhook or `on_connection_created` event fires a non-periodic sync, it stamps the same sync_state, so the scheduler doesn't redundantly re-fire.
- **Errors are logged and swallowed.** The scheduler must never panic out of its loop, or periodic sync stops silently for the rest of the process lifetime.

## What lands in the memory tree

Each provider is responsible for shaping its own ingest. The Gmail provider, for example, fetches a page of new messages, runs the email canonicalizer, and pipes the result through the same `ingest` path the manual UI uses, chunks land in SQLite, summary bucket fills, topic tree gets dirtied for any entities touched.

Other providers (GitHub, Slack, Notion, …) follow the same shape: fetch new items since cursor → canonicalize → ingest into the [Memory Tree](memory-tree.md).

## Why a 20-minute tick

The original design ran at 60 seconds. With several connected providers, that meant a steady drumbeat of HTTP fetches and DB writes, visibly busy on a laptop. Twenty minutes trades a little staleness for noticeably less foreground load. The per-provider `sync_interval_secs` still caps the _minimum_ delay between actual syncs; the global tick only loosens the upper bound.

## Tuning and visibility

- **Per-provider interval**. each native provider declares its own `sync_interval_secs`, so high-traffic toolkits (Gmail) can sync more often than low-traffic ones (Stripe).
- **Daily budget**. every connection has a daily request budget to keep API costs and rate limits sane.
- **Logs**. sync activity is logged in the core logs at debug level.

## See also

- [Third-party Integrations](../integrations/README.md). the connector layer auto-fetch runs on top of.
- [Memory Tree](memory-tree.md). where everything ends up.
- [Smart Token Compression](../token-compression.md). what keeps "fetch everything" cheap.
