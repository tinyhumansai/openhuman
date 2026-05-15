---
description: >-
  Optional, opt-in local AI via Ollama or LM Studio. Powers memory embeddings, summary-tree
  building, and background loops on-device. Chat / vision / voice are cloud.
icon: microchip
---

# Local AI (optional)

OpenHuman can run a local model on your machine for the workloads where keeping data on-device matters most: **memory embeddings, summary-tree building, and background reasoning loops**. It is **opt-in** and ships **off** by default.

This is a deliberate scoping. The previous design tried to put chat, vision, STT and TTS all on-device with Gemma 3, and the result was a heavy, hardware-sensitive footprint that fought with what the rest of the product needed to be. Today, the things that benefit most from being local (recurring, low-latency, privacy-sensitive memory work) run local; the things that benefit most from frontier models (default chat, reasoning, vision) stay cloud.

## What runs local when you turn it on

| Workload                  | Default model                     | Implementation                                                                                                    |
| ------------------------- | --------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| **Memory embeddings**     | `all-minilm:latest`               | `src/openhuman/embeddings/ollama.rs` - used by the [Memory Tree](../obsidian-wiki/memory-tree.md) for vector search. |
| **Summary-tree building** | `gemma3:1b-it-qat` (configurable) | `src/openhuman/tree_summarizer/ops.rs` - source / topic / global summary builders for the Memory Tree.            |
| **Heartbeat loop**        | small chat model                  | `src/openhuman/heartbeat/` - periodic background reflection.                                                      |
| **Learning / reflection** | small chat model                  | `src/openhuman/learning/reflection.rs` - passes that consolidate what was learned.                                |
| **Subconscious**          | small chat model                  | `src/openhuman/subconscious/executor.rs` - background evaluation loop.                                            |

Each of these is a **per-feature opt-in flag**. Turning on local AI does not silently route everything through it, you choose the workloads.

## What stays in the cloud

| Workload           | Why cloud                                                                                           |
| ------------------ | --------------------------------------------------------------------------------------------------- |
| **Chat (default)** | Frontier reasoning quality. Routed via the [model router](README.md) under one subscription. |
| **Vision**         | Same.                                                                                               |
| **STT**            | Backend-proxied transcription (`src/openhuman/voice/cloud_transcribe.rs`).                          |
| **TTS**            | Hosted [text-to-speech](../native-tools/voice.md) under the hood (`reply_speech.rs`).                            |
| **Web search**     | Backend proxy (no API key on your machine).                                                         |

For **lightweight or medium chat hints** (`hint:reaction`, `hint:classify`, `hint:format`, `hint:sentiment`, `hint:summarize`, `hint:medium`, `hint:tool_lite`), the [router](README.md) will prefer the local provider when local AI is enabled and Ollama is reachable. Heavy hints (`hint:reasoning`, `hint:agentic`, `hint:coding`) stay cloud.

## How it works

Under the hood, OpenHuman supports two local provider paths:

* [Ollama](https://ollama.com), used for bundled model lifecycle, embeddings, and the existing model-asset flow.
* [LM Studio](https://lmstudio.ai), used through its local OpenAI-compatible server for chat-style local inference.

For Ollama, OpenHuman talks to its OpenAI-compatible `/v1` endpoint where possible. That means:

* The `OpenAiCompatibleProvider` (`src/openhuman/providers/compatible.rs`) wraps Ollama exactly the way it wraps a remote OpenAI-style provider. No special-case code path.
* The provider router creates a _health-gated_ local provider on startup. If Ollama is not reachable, requests transparently fall back to the remote provider, no broken state.
* Models are pulled on demand by Ollama and cached in its own store. OpenHuman doesn't ship the weights itself.

For LM Studio, set `local_ai.provider = "lm_studio"` and ensure LM Studio's local server is running. OpenHuman defaults to `http://localhost:1234/v1`, probes `GET /v1/models`, and sends chat requests to `POST /v1/chat/completions`. You can override the endpoint with `local_ai.base_url`, `OPENHUMAN_LM_STUDIO_BASE_URL`, or `LM_STUDIO_BASE_URL`.

## Opting in

Local AI is gated by two flags in the core config (`src/openhuman/config/schema/local_ai.rs`):

| Flag                                 | Default | Meaning                                                             |
| ------------------------------------ | ------- | ------------------------------------------------------------------- |
| `local_ai.runtime_enabled`           | `false` | Master switch. `false` ⇒ no local provider is created at all.       |
| `local_ai.opt_in_confirmed`          | `false` | Explicit opt-in marker. Bootstrap forces `false` unless you re-opt. |
| `local_ai.provider`                  | `ollama` | Local provider: `ollama` or `lm_studio`.                            |
| `local_ai.base_url`                  | unset   | Optional provider URL. LM Studio defaults to `http://localhost:1234/v1`. |
| `local_ai.usage.embeddings`          | `false` | Use local for memory embeddings.                                    |
| `local_ai.usage.heartbeat`           | `false` | Use local for the heartbeat loop.                                   |
| `local_ai.usage.learning_reflection` | `false` | Use local for learning passes.                                      |
| `local_ai.usage.subconscious`        | `false` | Use local for the subconscious loop.                                |

In the desktop app, **Settings → AI & Skills → Local AI** exposes presets, pick one ("embeddings only", "memory + reflection", "everything local") and the right combination of flags is set for you. Status (Ollama reachability, model availability, per-subsystem enablement) is surfaced live via `openhuman.local_ai_status`.

## When to turn it on

Local AI is worth turning on if any of these are true:

* You ingest large volumes of email / chat and want **embeddings to never leave the machine**.
* You want **summary-tree building** to work offline.
* You're privacy-sensitive about background reflection ("subconscious") loops.

It is **not** worth turning on if you only have a few sources connected, the cloud path is faster and the privacy benefit is small. There is also a hardware cost: Ollama and a small Gemma model want a few GB of RAM and pull a few GB of weights.

## What you'll need

* [**Ollama**](https://ollama.com) installed and running locally, or [**LM Studio**](https://lmstudio.ai) with the local server enabled.
* Enough disk for the models (`gemma3:1b-it-qat` \~700 MB, `all-minilm:latest` \~23 MB).
* Enough RAM to keep the model resident (8 GB+ recommended, 16 GB+ ideal).

OpenHuman handles the rest: lifecycle (`src/openhuman/local_ai/service/`), API clients (`ollama_api.rs`, `lm_studio_api.rs`), health checks, and graceful fallback to remote when the local provider disappears.

### LM Studio troubleshooting

* Confirm the LM Studio local server is enabled and reachable at `http://localhost:1234/v1`.
* Load the selected model in LM Studio before calling OpenHuman. Diagnostics report `load_lm_studio_model` when the configured `local_ai.chat_model_id` is not present in `/v1/models`.
* If LM Studio uses a different port, set `local_ai.base_url` or `OPENHUMAN_LM_STUDIO_BASE_URL`.
* LM Studio model downloads are managed inside LM Studio. OpenHuman will not pull LM Studio models from the local asset-download controls.

## See also

* [Memory Tree](../obsidian-wiki/memory-tree.md). what local embeddings + summarization power.
* [Automatic Model Routing](README.md). how lightweight chat hints prefer the local provider.
* [Privacy & Security](../privacy-and-security.md). what moves on-device when you opt in.
