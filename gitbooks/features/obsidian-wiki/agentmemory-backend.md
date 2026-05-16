---
description: >-
  Optional `Memory` trait backend that delegates to a locally-running
  agentmemory REST server, for users who self-host agentmemory across
  Claude Code, Cursor, Codex, OpenCode, and OpenHuman.
icon: database
---

# agentmemory backend

OpenHuman's default `Memory` trait backend is `sqlite` — the unified store
documented in [Memory Trees](memory-tree.md). For users who already
self-host [agentmemory](https://github.com/rohitg00/agentmemory) — typically
because they want a single durable memory shared across Claude Code,
Cursor, Codex, OpenCode, and OpenHuman — OpenHuman exposes an opt-in
backend that proxies every trait call through agentmemory's REST surface.

Selecting `backend = "agentmemory"` skips OpenHuman's SQLite + embedder
path entirely. agentmemory owns the storage, embedding, and retrieval
layers. OpenHuman becomes a thin REST client.

## When to use this

Use the agentmemory backend if:

- You already run `npx -y @agentmemory/agentmemory` for one or more
  coding agents and want OpenHuman to share the same durable store.
- You want hybrid BM25 + vector + graph retrieval without provisioning a
  separate embedder on the OpenHuman side.
- You prefer agentmemory's lifecycle (consolidation, retention scoring,
  auto-forget, graph extraction) over OpenHuman's unified store.

Keep the default `sqlite` backend if:

- You want self-contained, single-process operation with no external
  daemon dependency.
- You rely on OpenHuman-specific Memory Tree features (chunking,
  sealing, summary trees) that operate on top of the SQLite store. The
  Memory Tree pipeline is unaffected by the trait backend — it operates
  on the host's document store, orthogonally — but the agentmemory
  backend is most valuable when you've already standardised on
  agentmemory across other agents.

## Quick start

1. **Install + start agentmemory** (one terminal):

   ```bash
   npx -y @agentmemory/agentmemory
   ```

   Defaults to `http://localhost:3111` (REST) + `ws://localhost:49134`
   (engine). First boot generates an HMAC secret at `~/.agentmemory/.hmac`
   and prints it once.

2. **Point OpenHuman at it** in your `config.toml`:

   ```toml
   [memory]
   backend = "agentmemory"
   # Defaults below — set only when overriding.
   # agentmemory_url        = "http://localhost:3111"
   # agentmemory_secret     = ""           # HMAC bearer token, optional
   # agentmemory_timeout_ms = 5000
   ```

3. **Restart OpenHuman**. The factory short-circuits the SQLite path
   and logs `[memory::factory] using agentmemory backend at <url>`.

That's it. Existing OpenHuman call sites (`store`, `recall`, `get`,
`list`, `forget`, `namespace_summaries`, `count`, `health_check`) work
unchanged.

## Config keys

| Field | Default | Purpose |
|---|---|---|
| `agentmemory_url` | `http://localhost:3111` | Base URL for the agentmemory REST server |
| `agentmemory_secret` | _none_ | Optional HMAC bearer token. Sent as `Authorization: Bearer <secret>` |
| `agentmemory_timeout_ms` | `5000` | Per-request reqwest timeout |

When `backend == "agentmemory"`, the following existing `MemoryConfig`
fields are **ignored** — agentmemory owns its own embedding stack via
`~/.agentmemory/.env`:

- `embedding_provider`
- `embedding_model`
- `embedding_dimensions`
- `sqlite_open_timeout_secs`

Setting them on this path is a no-op. The local-AI Ollama health-gate
also doesn't run on this path — agentmemory's daemon manages its own
embedder lifecycle.

## Field mapping

OpenHuman's `MemoryEntry` ↔ agentmemory wire row:

| OpenHuman field | agentmemory field | Notes |
|---|---|---|
| `namespace` | `project` | Defaults to `"default"` when empty |
| `key` | `title` | |
| `content` | `content` | |
| `id` | `id` | agentmemory-generated (`mem_<rand>`) |
| `category: Core` | `type: "fact"` | |
| `category: Daily` | `type: "conversation"` | |
| `category: Conversation` | `type: "conversation"` | |
| `category: Custom(s)` | `type: "fact"` + `concepts: [s]` | Custom tag rolled into the concepts array so it remains queryable |
| `session_id` | `sessionIds: [...]` | OpenHuman exposes a single id; agentmemory persists an array |
| `timestamp` | `updatedAt` (RFC3339) | Falls back to `createdAt` if `updatedAt` is absent |
| `score` (recall hits only) | smart-search `score` | Populated on `recall` responses, `None` on `get` / `list` |

agentmemory carries additional fields — `concepts` (auto-extracted),
`files` (path tags), `strength` (retention score), `version`,
`supersedes` (the lifecycle chain) — that this backend leaves at
defaults. They're internal to agentmemory's lifecycle layer and don't
need to round-trip through OpenHuman's trait.

## Trait method → endpoint

| `Memory` method | agentmemory REST | Notes |
|---|---|---|
| `store` | `POST /agentmemory/remember` | `{project, title, content, type, concepts, sessionIds}` |
| `recall` | `POST /agentmemory/smart-search` | Hybrid BM25 + vector + graph |
| `get` | `POST /agentmemory/smart-search` | + client-side exact-title filter |
| `list` | `GET /agentmemory/memories?latest=true&project=<ns>` | |
| `forget` | `get(ns, key)` → `POST /agentmemory/forget` | Two-step: resolve id then forget |
| `namespace_summaries` | `GET /agentmemory/projects` | Returns `[{name, count, lastUpdated}]` |
| `count` | `GET /agentmemory/health` | Reads `memories` field |
| `health_check` | `GET /agentmemory/livez` | |

`RecallOpts.category`, `RecallOpts.session_id`, and `RecallOpts.min_score`
are applied as **client-side filters** on the smart-search response.
agentmemory's REST surface doesn't expose them as server-side filters
today. For very large recall windows (limit > 100) prefer issuing a
tighter query string to reduce server-side work over relying on
client-side post-filtering.

## Security

When `agentmemory_secret` is set, the client honours agentmemory's
v0.9.12 plaintext-bearer guard contract:

- **Loopback hosts** (`localhost`, `127.0.0.1`, `::1`) over `http://` —
  allowed. Local dev path.
- **`https://`** to any host — allowed.
- **Plaintext HTTP to a non-loopback host** — emits a one-time stderr
  warning at construction time. The bearer is observable on the wire.
- **`AGENTMEMORY_REQUIRE_HTTPS=1`** (process env, ASCII-case-insensitive
  matches `1` or `true`) — escalates the warning into a hard refusal at
  client construction. The backend fails to start rather than leak the
  bearer once.

Production deploys should set `AGENTMEMORY_REQUIRE_HTTPS=1` so a
misconfigured TLS terminator fails loud rather than silently leaking.

The plaintext-bearer guard mirrors the integration plugin guards in
agentmemory's [PR #315](https://github.com/rohitg00/agentmemory/pull/315)
so an operator who's seen the warning on Hermes / OpenClaw / pi will
recognise the same message on OpenHuman.

## Failure modes

| Failure | Backend behaviour |
|---|---|
| Daemon unreachable at startup | `from_config` succeeds (URL parses), but `health_check()` returns false on first call. Trait methods bubble up `reqwest` transport errors |
| Network timeout | `anyhow::Error` per trait contract; surfaces to caller |
| 4xx / 5xx response | `anyhow::Error` with status + body snippet |
| Bearer over plaintext non-loopback (no env) | One-time stderr warning, request proceeds |
| Bearer over plaintext non-loopback + `AGENTMEMORY_REQUIRE_HTTPS=1` | Hard refusal at construction time |
| Empty `agentmemory_url` | Hard refusal at construction time with hint to leave it unset for the default |
| Invalid URL syntax | Hard refusal at construction time with the parser error |

**No automatic fallback to SQLite.** If the daemon is down at boot, the
backend surfaces the transport error loudly. Operators flip back to
`backend = "sqlite"` in `config.toml` to recover. Rationale: a silent
SQLite fallback would hide a misconfigured daemon — "private, simple,
predictable" wins over "magically tolerant".

## Performance notes

The backend is a thin REST proxy — it adds one HTTP round-trip per
trait call. Practical implications:

- `store` and `forget` are single-RTT.
- `recall`, `get`, `list` are single-RTT.
- `forget` against an unknown key is two-RTT (the implicit `get` lookup
  + a no-op confirmation). Caller can short-circuit this by checking
  the return value of a prior `list`.
- agentmemory's REST is `127.0.0.1` by default — same-host latency is
  sub-millisecond. Over a managed deploy with HTTPS termination, expect
  ~10–30ms per RTT.
- The default per-request timeout is 5 seconds. Bump
  `agentmemory_timeout_ms` if you're seeing intermittent timeouts on
  cold-start of the iii engine; agentmemory's first-request latency
  after a long idle can stretch toward 3–5s depending on persistence
  state.

## Migration: from SQLite to agentmemory

There's no in-place migration today. The recommended path:

1. Export your existing memories from the SQLite store via OpenHuman's
   existing export RPC (or by direct SQL).
2. Walk the export and POST each row to `/agentmemory/remember` with
   the same `project` + `title` + `content`. agentmemory will assign
   new ids; the OpenHuman side picks them up on first `list`.
3. Set `backend = "agentmemory"` and restart.

A dedicated bulk import path is filed as a follow-up.

## Implementation reference

In-tree files:

- [`store/agentmemory/mod.rs`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/memory/store/agentmemory/mod.rs) — module surface
- [`store/agentmemory/backend.rs`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/memory/store/agentmemory/backend.rs) — `impl Memory for AgentMemoryBackend`
- [`store/agentmemory/client.rs`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/memory/store/agentmemory/client.rs) — reqwest wrapper + plaintext-bearer guard
- [`store/agentmemory/mapping.rs`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/memory/store/agentmemory/mapping.rs) — `MemoryEntry` ↔ agentmemory JSON
- [`tests/agentmemory_backend.rs`](https://github.com/tinyhumansai/openhuman/tree/main/tests/agentmemory_backend.rs) — 12 axum-mock integration tests

Related upstream:

- agentmemory repo — <https://github.com/rohitg00/agentmemory>
- agentmemory REST contract — `~/.agentmemory/.env` keys + endpoint
  list in the agentmemory README
- v0.9.12 plaintext-bearer guard — agentmemory PR #315
