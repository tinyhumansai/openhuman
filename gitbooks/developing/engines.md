---
description: >-
  The four swappable engines (models, embeddings, memory, search) and the
  config key that selects each one.
icon: gears
---

# Pluggable engines

OpenHuman sends four kinds of work through a config key instead of a hardcoded client: chat and reasoning, embeddings, memory and web search. Each has a managed default that needs no key, plus a bring-your-own-key or local option. This page lists what is supported and the key that selects it. The linked Features pages cover setup and tradeoffs.

## LLM providers

Chat, reasoning, vision and coding work each have their own provider field (`chat_provider`, `reasoning_provider`, `agentic_provider`, `coding_provider`, `vision_provider` and more). Each is a string of the form `<slug>:<model>`. A field that is unset, blank or set to `cloud` stays on the managed default.

Supported routes:

- Managed (TinyHumans): the default. It gives you the OpenRouter model catalogue with no key to manage.
- Local: a runtime you install and run (Ollama, LM Studio, MLX, OMLX). Address it as `ollama:<model>`, `lmstudio:<model>`, `mlx:<model>` or `omlx:<model>`, and set the endpoint in `[local_ai] base_url`. OpenHuman does not install the runtime or download models. You pull them yourself.
- A local OpenAI-compatible endpoint: any server that speaks the OpenAI chat API. Use `local-openai:<model>`, or register it with its own slug and endpoint.
- Claude Code / Claude Agent SDK: a provider slug that runs your installed Claude Code CLI instead of calling a hosted API.
- 27 bring-your-own-key slugs, each with a preset endpoint so you only need a key: `openai`, `anthropic`, `google`, `openrouter`, `orcarouter`, `groq`,
  `mistral`, `deepseek`, `together`, `fireworks`, `cerebras`, `xai`,
  `moonshot`, `gmi`, `huggingface`, `nvidia`, `zai`, `minimax`, `stepfun`,
  `kilocode`, `deepinfra`, `novita`, `venice`, `vercel-ai-gateway`, `sumopod`,
  `modelscope`, `llmtr`.

Provider definitions live under `crates/openhuman-core/src/inference/provider/`
(`factory.rs` turns a `<slug>:<model>` string into a client, and `types.rs` holds the provider shapes). Setup and the local-model capability table are in [Local models and bring your own key](../features/model-routing/local-and-byok-models.md). Routing and fallback order are in [Automatic model routing](../features/model-routing/README.md).

## Embeddings

Embeddings have their own provider setting, separate from the chat provider. Set it with `embeddings.update_settings` over RPC, or override it per workload with `embeddings_provider`.

- Managed (default): the OpenHuman backend's Voyage-backed embedding endpoint. It works on a fresh install with no local daemon.
- Voyage: the Voyage AI API with your own key.
- OpenAI: cloud embeddings through the OpenAI API.
- Cohere: the Cohere embed API with your own key.
- Ollama: a local model you have pulled. `bge-m3` is recommended (`ollama pull bge-m3`). The memory engine does not use these, because it embeds on its own side.
- Custom: any OpenAI-compatible embeddings endpoint.

The code is in `crates/openhuman-core/src/inference/embedding_host/`. `mod.rs` lists the providers, `factory.rs` builds the client, and `schemas.rs` defines the `embeddings.*` RPC surface, including `set_api_key` per provider slug.

## Memory

Memory is Recall, Fetch and Store over a pluggable engine. The contract is `tinymemory-api`, vendored at `vendor/tinymemory/`. It defines a `MemoryEngine` trait with `recall`, `fetch`, `store`, `forget`, `list` and `health`. An `EngineDescriptor` says whether the engine is hosted, whether it needs an endpoint or key, and which fetch modes it supports. `tinymemory::list_engines` and `build_engine` are the registry.

Two engines ship, both served by `tinymemory-integrations`:

| id | what | endpoint | key |
| --- | --- | --- | --- |
| `tinyhumans` | CortexDB hosted behind the TinyHumans backend (`/memory/*`) | the backend origin | the signed-in session or API key, resolved per request through the host credential seam |
| `cortexdb` | the user's own CortexDB (direct `/v1/*`) | `[memory.engines.cortexdb] endpoint` (default `https://api-v1.cortexdb.ai`) | stored in the OS keychain as `memory-cortexdb` |

Select one with `[memory] engine` or from Connections > Memory > Engine (`openhuman.memory_engine_set`). `memory::engine::resolve` binds it. If you are signed out and have no CortexDB key, memory is off: the `memory` tool is not registered, ingestion does nothing and RPCs answer `MEMORY_OFF`.

Built engines are cached by a fingerprint of engine id, endpoint and credential identity. Changing any of them rebuilds the engine on the next call. Engines do not migrate, so switching starts empty. The one-time import of an older store uploads to whichever engine is selected, after you consent.

Both engines declare `fetch_modes = [hybrid]`, because the CortexDB recall wire has no keyword or vector switch. Asking for another mode fails with `UNSUPPORTED`.

Related pages: [Memory](../features/memory.md) and
[Memory architecture](architecture/memory.md).

## Web search

`[search] engine` selects the active provider. Only one is active at a time:

- `managed` (default): proxied through the backend, no key needed.
- `parallel`: search, extract, chat, research, enrich and dataset tools on the Parallel API.
- `brave`: Brave Search (web, news, images, videos).
- `querit`: Querit web search.
- `exa`: Exa neural search (search, find-similar, contents). It calls `api.exa.ai` directly, never through the managed backend.
- `tavily`: Tavily search and extract (web, news, finance). It calls `api.tavily.com` directly.
- `disabled`: no search tools registered.

If you pick a bring-your-own-key engine without a stored key, search falls back to `managed` so the agent is never left without a search tool.

SearXNG is a separate toggle (`[searxng] enabled`, `base_url`), not a `search.engine` value, because it points at a self-hosted instance instead of a vendor API.

The providers live in the `tinysearch` module (`vendor/tinysearch`), which owns the per-provider transports and role dispatch. The host keeps provider resolution and credential policy in `crates/openhuman-core/src/search/` and `crates/openhuman-core/src/config/schema/tools/search.rs`. Its provider list comes from the module contract.

See [Web search](../features/native-tools/web-search.md) for the tools each engine exposes.
