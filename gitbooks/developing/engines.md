# Pluggable engines

OpenHuman routes four kinds of work, chat and reasoning, embeddings, memory,
and web search, through a config key rather than a hardcoded client. Each has
a managed default that needs no key, plus a bring-your-own-key or local path.
This page is the map of what's supported and the exact key or environment
variable that selects it; the linked pages under Features cover setup and
tradeoffs.

## LLM providers

Chat, reasoning, vision, and coding workloads route through per-workload
provider fields (`chat_provider`, `reasoning_provider`, `agentic_provider`,
`coding_provider`, `vision_provider`, and more), each a string of the form
`<slug>:<model>`. Leaving a field unset, blank, or set to `cloud` keeps it on
the managed default.

Supported routes:

- **Managed (TinyHumans)**: the default. Access to the OpenRouter model
  catalogue with no key to manage.
- **Local**: a runtime the user installs and runs (Ollama, LM Studio, MLX,
  OMLX), addressed as `ollama:<model>`, `lmstudio:<model>`, `mlx:<model>` or
  `omlx:<model>`, with the endpoint in `[local_ai] base_url`. OpenHuman does
  not install the runtime or download models; the user pulls them.
- **A local OpenAI-compatible endpoint**: any server that speaks the OpenAI
  chat API, as `local-openai:<model>` or registered with its own slug and
  endpoint.
- **Claude Code / Claude Agent SDK**: a provider slug that shells out to an
  installed Claude Code CLI instead of calling a hosted API.
- **26 BYOK slugs**, each shipped with a preset endpoint so only a key is
  needed: `openai`, `anthropic`, `google`, `openrouter`, `orcarouter`, `groq`,
  `mistral`, `deepseek`, `together`, `fireworks`, `cerebras`, `xai`,
  `moonshot`, `gmi`, `huggingface`, `nvidia`, `zai`, `minimax`, `stepfun`,
  `kilocode`, `deepinfra`, `novita`, `venice`, `vercel-ai-gateway`, `sumopod`,
  `modelscope`.

Provider definitions live under `crates/openhuman-core/src/inference/provider/`
(`factory.rs` resolves a `<slug>:<model>` string to a client; `types.rs` holds
the provider shapes). Full setup and the local-model capability table are
in [Local models & bring your own
key](../features/model-routing/local-and-byok-models.md); routing behavior
and fallback order are in [Automatic Model
Routing](../features/model-routing/README.md).

## Embeddings

Embeddings route through a separate provider selection
(`embeddings.update_settings` over RPC, or `embeddings_provider` for
per-workload override), independent of the chat provider:

- **Managed** (default): the OpenHuman backend's Voyage-backed embedding
  endpoint. Works on a fresh install with no local daemon.
- **Voyage**: direct Voyage AI API with your own key.
- **OpenAI**: cloud embeddings via the OpenAI API.
- **Cohere**: the Cohere embed API with your own key.
- **Ollama**: a local model the user has pulled, `bge-m3` recommended
  (`ollama pull bge-m3`). Memory v2 does not use these; the memory engine
  embeds on its own side.
- **Custom**: any OpenAI-compatible embeddings endpoint.

Implementation: `crates/openhuman-core/src/inference/embedding_host/` (`mod.rs`
lists the providers; `factory.rs` builds the client; `schemas.rs` defines the
`embeddings.*` RPC surface, including `set_api_key` per provider slug).

## Memory

Memory v2 (`docs/specs/memory-v2.md`) is Recall, Fetch and Store over a
pluggable engine. The contract is `tinymemory-api` (vendored at
`vendor/tinymemory/`): a `MemoryEngine` trait with `recall`, `fetch`, `store`,
`forget`, `list` and `health`, plus an `EngineDescriptor` that declares whether
the engine is hosted, needs an endpoint or key, and which fetch modes it
supports. `tinymemory::list_engines` and `build_engine` are the registry.

Two engines ship, both from the `tinymemory-cortex` crate:

| id | what | endpoint | key |
| --- | --- | --- | --- |
| `tinyhumans` | CortexDB hosted behind the TinyHumans backend (`/memory/*`) | the backend origin | the signed-in session or API key, resolved per request through the host credential seam |
| `cortexdb` | the user's own CortexDB (direct `/v1/*`) | `[memory.engines.cortexdb] endpoint` (default `https://api-v1.cortexdb.ai`) | stored in the OS keychain as `memory-cortexdb` |

Select one with `[memory] engine` or from Connections > Memory > Engine
(`openhuman.memory_engine_set`). `memory::engine::resolve` binds it: signed out
with no CortexDB key means **memory is off** (the `memory` tool is not
registered, ingestion is a no-op and RPCs answer `MEMORY_OFF`). Built engines
are cached by a fingerprint of engine id, endpoint and credential identity, so
changing any of them rebuilds the engine on the next call. There is no engine
migration: switching engines starts empty, and the one-time import of a v1
store uploads to whichever engine is selected, after explicit consent.

Both engines declare `fetch_modes = [hybrid]` because the CortexDB recall wire
has no keyword/vector switch; asking for another mode fails with `UNSUPPORTED`.

Related pages: [Memory](../features/memory.md) and
[Memory architecture](architecture/memory.md).

## Web search

`[search] engine` selects the active provider; only one is active at a time,
mirroring the LLM provider model:

- `managed` (default): backend-proxied, no key needed.
- `parallel`: search, extract, chat, research, enrich, and dataset tools
  against the Parallel API.
- `brave`: Brave Search (web, news, images, videos).
- `querit`: Querit web search.
- `exa`: Exa neural search (search, find-similar, contents). Direct to
  `api.exa.ai`, never through the managed backend.
- `tavily`: Tavily search and extract (web, news, finance). Direct to
  `api.tavily.com`.
- `disabled`: no search tools registered.

A BYOK engine selected without a stored key falls back to `managed` rather
than leaving the agent with no search tool at all.

SearXNG is a separate toggle (`[searxng] enabled`, `base_url`) rather than a
`search.engine` value, since it points at a self-hosted instance instead of a
vendor API. Engine definitions live under
`crates/openhuman-core/src/search/engines/`, one file per provider; the
selector and provider resolution are in `crates/openhuman-core/src/search/mod.rs`
and `crates/openhuman-core/src/config/schema/tools/search.rs`.

See [Web Search](../features/native-tools/web-search.md) for the tool surface
each engine exposes.
