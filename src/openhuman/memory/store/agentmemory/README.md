# agentmemory backend

Optional `Memory` backend that delegates every trait call to a locally-running
[agentmemory](https://github.com/rohitg00/agentmemory) REST server (default
`http://localhost:3111`).

Selected via:

```toml
[memory]
backend = "agentmemory"
```

The default backend stays `sqlite`; selecting `"agentmemory"` is opt-in and
non-breaking for existing configs.

## Why another backend (and why not via MCP tools)

OpenHuman already supports MCP servers as tools, but that path is
agent-facing — the LLM picks `recall` / `save` as tool calls per turn. The
`Memory` trait is the *host-facing* surface: harness, archivist, reflection,
prompt-section builders all consume it directly, without going through
tool-call latency.

Plugging in as a `Memory` backend lets agentmemory back things like
`mem::context` injection in `loadMemoryRules`, reflection passes, and the
namespace summaries that drive agent-side discovery — none of which would
route through MCP today.

It also lets operators who self-host agentmemory across multiple agents
(Claude Code, Cursor, Codex, OpenCode, plus OpenHuman) share a single
durable memory.

## Config keys

| Field | Default | Purpose |
|---|---|---|
| `agentmemory_url` | `http://localhost:3111` | Base URL for the agentmemory REST server |
| `agentmemory_secret` | `None` | Optional HMAC bearer token sent as `Authorization: Bearer <secret>` |
| `agentmemory_timeout_ms` | `5000` | Per-request reqwest timeout |

When `backend == "agentmemory"`, the existing `embedding_provider` /
`embedding_model` / `embedding_dimensions` fields are **ignored** —
agentmemory owns its own embedding stack via `~/.agentmemory/.env`. Setting
them on this path is a no-op.

## Field mapping

| OpenHuman `MemoryEntry` | agentmemory wire |
|---|---|
| `namespace` | `project` (defaults to `"default"` if empty) |
| `key` | `title` |
| `content` | `content` |
| `MemoryCategory::Core` | `type: "fact"` |
| `MemoryCategory::Daily` | `type: "conversation"` |
| `MemoryCategory::Conversation` | `type: "conversation"` |
| `MemoryCategory::Custom(s)` | `type: "fact"`, `concepts: [s]` |
| `session_id` | `sessionIds: [...]` |
| `timestamp` | `updatedAt` (RFC3339), falling back to `createdAt` |
| `score` (recall hits) | smart-search `score` |

agentmemory has additional fields (`concepts`, `files`, `strength`,
`version`, `supersedes`) that this backend leaves at defaults — they're
internal to agentmemory's lifecycle layer.

## Trait method → endpoint

| `Memory` method | agentmemory REST |
|---|---|
| `store` | `POST /agentmemory/remember` |
| `recall` | `POST /agentmemory/smart-search` (hybrid BM25 + vector + graph) |
| `get` | `POST /agentmemory/smart-search` then exact-title filter |
| `list` | `GET /agentmemory/memories?latest=true&project=<ns>` |
| `forget` | `get(ns, key)` → `POST /agentmemory/forget` with the id |
| `namespace_summaries` | `GET /agentmemory/projects` |
| `count` | `GET /agentmemory/health` (`memories` field) |
| `health_check` | `GET /agentmemory/livez` |

`RecallOpts.category` / `session_id` / `min_score` are applied as
client-side filters on the smart-search response (agentmemory's REST
surface doesn't expose them as server-side filters today).

## Security: plaintext-bearer guard

When `agentmemory_secret` is set, the client refuses to send the token to a
non-loopback host over `http://`. Loopback (`localhost`, `127.0.0.1`, `::1`)
+ plaintext is allowed for local dev; everything else needs `https://` or
the daemon must be reachable on loopback.

Set `AGENTMEMORY_REQUIRE_HTTPS=1` as a process env var to harden this from
"warn on stderr" to "refuse to construct the client" — useful in
production where a misconfigured TLS terminator should fail loud rather
than leak the secret once.

## Failure modes

| Failure | Behaviour |
|---|---|
| agentmemory daemon down at construction time | `health_check()` returns false; trait methods bubble up the `reqwest` transport error |
| Network timeout | Returns `anyhow::Error` per trait contract; surfaces to caller |
| 4xx / 5xx response | Returns `anyhow::Error` with the response status + body snippet |
| Bearer over plaintext HTTP non-loopback | Warns on stderr (matches agentmemory's own client guard from v0.9.12 PR #315) |
| Bearer over plaintext HTTP + `AGENTMEMORY_REQUIRE_HTTPS=1` | Hard refusal at construction time |

No automatic fallback to `sqlite` — if the daemon is down at boot, the
service fails loud. Operators flip back to `backend = "sqlite"` in config
to recover. Rationale (per issue #1664 alignment): "private, simple,
predictable" — a silent SQLite fallback hides a misconfigured daemon.
