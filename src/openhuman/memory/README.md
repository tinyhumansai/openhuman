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
- **Seam impls** — [`host.rs`](host.rs) — `install_memory_event_sink` and
  `MemoryHostConfig for Config`. Its sibling `host_impls.rs` held the half that
  only an in-process engine could use, and went with the engine when the test
  build stopped linking one (openhuman#6161).

This module used to be mostly a **re-export** of the engine crate — a wall of
`pub use tinymemory_core::{chat, global, ingest_pipeline, ingestion,
preferences, remember, rpc_models, store, sync_events, traits, util, …}` in
[`mod.rs`](mod.rs), so the ~550 `crate::openhuman::memory::…` paths elsewhere
in this crate kept resolving after the extraction. Those re-exports are gone
with the engine: `tinymemory-core` left the product build in openhuman#5560 and
the test build in openhuman#6161, and it is now in neither this crate's normal
nor its dev dependency graph.

**Prefer `tinymemory_api::…` in new code, never `tinymemory_core::…`.** The
contract crate is what both this host and the loaded TinyMemory module compile
against; the engine crate is what the module carries and this binary does not
link. `memory::api` is the re-export of the contract, and its own module docs
explain which parts of `tinymemory-api` are the *bus* surface and which are the
host's own use of the crate — they are not the same set.

## Domains that kept their RPC surface here

Each is the RPC surface for a family the *driver* serves: the handler and
schema modules that name `RpcOutcome` and `ControllerSchema`, resolving through
the bound provider rather than through a linked engine. Before the engine left,
each was a thin wrapper over `pub use tinymemory_core::<domain>::*;` as well.

| Module                          | Role                                                     |
| -------------------------------- | --------------------------------------------------------- |
| [`conversations/`](conversations/) | Conversation-scoped memory RPC.                          |
| [`goals/`](goals/)               | Goal tracking RPC.                                       |
| [`people/`](people/)             | People/contacts RPC.                                     |
| [`sources/`](sources/)           | Source-registration RPC.                                 |
| [`sync/`](sync/)                 | Composio + workspace + MCP sync pipeline RPC.            |
| [`tool_memory/`](tool_memory/)   | Tool-scoped rules + agent read/write tools.               |
| [`tree/`](tree/)                 | Tree walk/retrieval RPC.                                  |

## What lives in the extracted crate (for reference)

See [`vendor/tinymemory/crates/tinymemory-core/src/`](https://github.com/tinyhumansai/tinymemory/tree/1d6b997874a06600ba0c4922708b5613497c9ffe/crates/tinymemory-core/src) for
the storage primitives (`store/`), ingestion queue (`ingestion/`), sync
lifecycle types (`sync_events.rs`), remember classification (`remember.rs`),
ingest orchestration (`ingest_pipeline.rs`), the `Memory`/`MemoryEntry`/etc.
traits (`traits.rs`), preferences (`preferences.rs`), and shared RPC shapes
(`rpc_models.rs`). Source → canonical markdown (chat / email / document)
lives in [`tinycortex::memory::ingest::canonicalize`](https://github.com/tinyhumansai/tinycortex/tree/main/src/memory/ingest/canonicalize),
owned by TinyCortex and used at ingest time.

## Layer rules

- **No storage in this module.** All persistence goes through the bound
  driver — `memory::binding::for_config(..)` and the `MemoryProvider`
  capability families behind it. If you are tempted to open a SQLite
  connection here, it belongs on the other side of that contract, in whatever
  engine the driver fronts. This crate does not link one.
- **RPC + tools + guard live here.** Domain logic belongs behind the contract;
  this module surfaces it over `/rpc` and to agents, and owns the policy that
  is genuinely the host's — the taint/scope/budget guard, the approval gate,
  and the workspace a binding is keyed on.
- **Surface high-level tool calls** that route to the right submodule;
  don't expose internals at the call site.
