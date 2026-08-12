# memory

Host layer over the memory stack. The substance of the memory subsystem was
extracted into [`tinymemory-core`](https://github.com/tinyhumansai/tinymemory): the
SQLite/vector store, the markdown summary tree, the provider sync pipelines,
ingestion, recall/query/search, the ingest queue, conversations, people,
goals and the tool-memory rules. That crate names no OpenHuman type — see
[its README](https://github.com/tinyhumansai/tinymemory#readme) for the extracted
side of this split.

Links to the extracted crate point at GitHub rather than into
`vendor/tinymemory/`: CI checks out this repository without submodule
contents, so a relative link into that directory resolves to nothing on the
runner and fails the link check.

What stays here, per that split:

- **RPC surface** — [`schemas/`](schemas/) + [`schema/`](schema/), the
  memory\_\* controller registrations, and [`read_rpc/`](read_rpc/) for reads.
- **Agent tools** — [`tools/`](tools/), [`agent/`](agent/) (the memory agent
  + prompt), and the consolidated `memory_query` agent tool in
  [`query/`](query/) (it came back from the extracted crate because the
  engine crate cannot name the `Tool` trait).
- **Guard** — [`guard/`](guard/), the taint/scope/budget policy gate over
  every provider call.
- **Driver binding** — [`driver/`](driver/), which provider backs a
  workspace.
- **Ops** — [`ops/`](ops/), RPC handlers that delegate into the core.
- **Seam impls** — [`host.rs`](host.rs) /
  [`host_impls.rs`](host_impls.rs) — `install_memory_event_sink` and
  `MemoryHostConfig for Config`.

Everything else in this module is a **re-export** of the extracted crate
(`pub use tinymemory_core::{chat, global, ingest_pipeline, ingestion,
preferences, remember, rpc_models, store, sync_events, traits, util, …}` in
[`mod.rs`](mod.rs)), so the ~550 `crate::openhuman::memory::…` paths
elsewhere in this crate keep resolving unchanged. Prefer
`tinymemory_core::…` in new code.

## Domains that kept their RPC surface here

Mostly extracted, but each is a thin wrapper (`pub use
tinymemory_core::<domain>::*;` plus the handler/schema modules that name
`RpcOutcome` and `ControllerSchema`):

| Module                          | Role                                                     |
| -------------------------------- | --------------------------------------------------------- |
| [`conversations/`](conversations/) | Conversation-scoped memory RPC.                          |
| [`diff/`](diff/)                 | Git-backed diff RPC (gated by the `memory-git` feature). |
| [`goals/`](goals/)               | Goal tracking RPC.                                       |
| [`people/`](people/)             | People/contacts RPC.                                     |
| [`sources/`](sources/)           | Source-registration RPC.                                 |
| [`sync/`](sync/)                 | Composio + workspace + MCP sync pipeline RPC.            |
| [`tool_memory/`](tool_memory/)   | Tool-scoped rules + agent read/write tools.               |
| [`tree/`](tree/)                 | Tree walk/retrieval RPC.                                  |

## What lives in the extracted crate (for reference)

See [`vendor/tinymemory/core/src/`](https://github.com/tinyhumansai/tinymemory/tree/main/core/src) for
the storage primitives (`store/`), ingestion queue (`ingestion/`), sync
lifecycle types (`sync_events.rs`), remember classification (`remember.rs`),
ingest orchestration (`ingest_pipeline.rs`), the `Memory`/`MemoryEntry`/etc.
traits (`traits.rs`), preferences (`preferences.rs`), and shared RPC shapes
(`rpc_models.rs`). Source → canonical markdown (chat / email / document)
lives in [`tinycortex::memory::ingest::canonicalize`](https://github.com/tinyhumansai/tinycortex/tree/main/src/memory/ingest/canonicalize),
owned by TinyCortex and used at ingest time.

## Layer rules

- **No storage in this module.** All persistence goes through
  `tinymemory_core::store::*`. If you're tempted to open a SQLite
  connection here, the connection helper belongs one layer down, in the
  extracted crate.
- **RPC + tools + seam wiring live here.** Domain logic belongs in
  `tinymemory-core`; this module surfaces it over `/rpc` and to agents.
- **Surface high-level tool calls** that route to the right submodule;
  don't expose internals at the call site.
