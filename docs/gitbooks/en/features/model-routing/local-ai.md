---
description: >-
  Use a model runtime you run yourself (Ollama, LM Studio, MLX, OMLX, or any
  OpenAI-compatible server) as an OpenHuman provider. OpenHuman does not install
  the runtime or download models.
icon: microchip
---

# Local AI (optional)

OpenHuman can send any workload to a model running on your own machine: chat, reasoning, agentic turns, coding, vision and embeddings. Local AI is opt-in and off by default.

OpenHuman does not install, start, stop or update a local runtime, and it does not download model weights. You do three things:

1. Install and run the runtime yourself (Ollama, LM Studio, MLX, OMLX, or another OpenAI-compatible server).
2. Pull or load the models you want with that runtime's own tools, for example `ollama pull bge-m3`.
3. Add the runtime's endpoint to OpenHuman as a provider and route workloads to it.

If a workload names a model the runtime does not have, the request fails with the runtime's error. Pull the model and retry. OpenHuman will not fetch it for you.

## Supported runtimes

OpenHuman reaches every local runtime over its OpenAI-compatible HTTP API. The prefix of the provider string decides which runtime a workload uses.

| Runtime                          | Provider string        | Default endpoint           | Endpoint overrides (highest first)                                                                            |
| -------------------------------- | ---------------------- | -------------------------- | ------------------------------------------------------------------------------------------------------------- |
| [Ollama](https://ollama.com)     | `ollama:<model>`       | `http://127.0.0.1:11434`   | `local_ai.base_url`, then `OPENHUMAN_OLLAMA_BASE_URL`, then `OLLAMA_HOST`. OpenHuman appends `/v1` for chat.  |
| [LM Studio](https://lmstudio.ai) | `lmstudio:<model>`     | `http://localhost:1234/v1` | `OPENHUMAN_LM_STUDIO_BASE_URL`, then `LM_STUDIO_BASE_URL`, then `local_ai.base_url`                           |
| MLX (`mlx_lm.server`)            | `mlx:<model>`          | `http://127.0.0.1:8080/v1` | `OPENHUMAN_LOCAL_INFERENCE_URL`, then `MLX_SERVER_URL`, then `local_ai.base_url`                              |
| OMLX                             | `omlx:<model>`         | `http://127.0.0.1:8000/v1` | `OPENHUMAN_LOCAL_INFERENCE_URL`, then `OMLX_SERVER_URL`, then `local_ai.base_url`                             |
| Any OpenAI-compatible server     | `local-openai:<model>` | `http://127.0.0.1:8080/v1` | `OPENHUMAN_LOCAL_INFERENCE_URL`, then `LOCAL_OPENAI_URL`, then `local_ai.base_url`                            |

LM Studio, OMLX and `local-openai` send `local_ai.api_key` as a Bearer token when one is set. Ollama and MLX send no auth. Local runtimes do not need an OpenHuman account.

The model part of the provider string goes to the runtime unchanged, so it must match the runtime's own model id. For Ollama, that is the name you pulled, including the tag. An optional `@<temperature>` suffix pins a temperature, for example `ollama:qwen2.5:14b@0.2`.

## Multiple self-hosted servers

If you run several OpenAI-compatible model servers, a gateway can expose one endpoint and use named model ids to pick the upstream server. See [Use multiple local LLM servers](../../guides/multiple-local-llm-servers.md) for an optional community example with a Mac mini and a Linux GPU PC. OpenHuman does not bundle or supervise the gateway.

## Adding a local runtime in the app

1. Start the runtime and make sure its server is listening. For Ollama, `curl http://localhost:11434/api/tags` should return JSON.
2. Pull or load the models you plan to use (see [Use OpenHuman with a local model](../../guides/local-model.md)).
3. Open **Connections > LLM**, choose **Add a provider**, and pick the runtime under **Local runtimes** (Ollama, LM Studio, OMLX). For MLX or another server, use **Add a custom provider** with its OpenAI-compatible endpoint.
4. Confirm or edit the endpoint. On save, OpenHuman writes `local_ai.base_url` and `local_ai.provider`, turns on `local_ai.runtime_enabled`, and lists the endpoint's models. If the runtime is not reachable, the provider is not saved.
5. Route workloads to it with **Custom routing**, choosing from the models the runtime reports.

The model list comes from the runtime. A model you have not pulled does not appear. Pull it in the runtime and reopen the picker.

## Configuration

Here is the same setup by hand in `config.toml`:

```toml
[local_ai]
runtime_enabled = true                # needed for the background local features below
provider = "ollama"                   # "ollama", "lm_studio", or "omlx"
base_url = "http://localhost:11434"   # optional; omit to use the default
chat_model_id = "gemma3:4b-it-qat"    # model used by local summary-tree building
embedding_model_id = "bge-m3"
vision_model_id = "gemma3:4b-it-qat"  # leave empty for no local vision
```

Workload routing sits at the top level of `config.toml`. Each field takes a provider string:

```toml
chat_provider = "ollama:gemma3:4b-it-qat"
reasoning_provider = "ollama:qwen2.5:14b"
embeddings_provider = "ollama:bge-m3"
```

The workload fields are `chat_provider`, `reasoning_provider`, `agentic_provider`, `coding_provider`, `vision_provider`, `memory_provider` and `embeddings_provider`. A field that is unset, blank or `cloud` stays on the default route. For background workloads, a workload counts as local only when its field is `ollama:<model>`.

The older `local_ai.usage.*` booleans remain only so older configs migrate. They do not override the workload fields.

## What the local runtime is used for

| Workload                         | Configured by                                 | Notes                                                              |
| -------------------------------- | --------------------------------------------- | ------------------------------------------------------------------ |
| Chat, reasoning, coding, agentic | `chat_provider`, `reasoning_provider`, ...    | Any local prefix from the table above.                             |
| Vision                           | `vision_provider`, `local_ai.vision_model_id` | Must be a vision-capable model. See [Local vision](#local-vision). |

For lightweight chat hints (`hint:reaction`, `hint:classify`, `hint:format`, `hint:sentiment`, `hint:summarize`, `hint:medium`, `hint:tool_lite`), the [router](README.md) prefers the local provider when `local_ai.runtime_enabled = true` and the runtime is reachable. Heavy hints (`hint:reasoning`, `hint:agentic`, `hint:coding`) stay on the default route unless the matching workload field points at a local provider.

## What stays in the cloud by default

| Workload   | Why                                                                                                                  |
| ---------- | -------------------------------------------------------------------------------------------------------------------- |
| STT        | Transcription goes through the backend or a third-party key. There is no local speech-to-text engine.                |
| TTS        | [Text-to-speech](../native-tools/voice.md) is hosted by default. Local Piper works if you install it and set `PIPER_BIN`. |
| Web search | Backend proxy, or your own search provider key.                                                                      |

Any workload you have not pointed at a local provider keeps using the default route.

### Local text-to-speech with Piper

Setting `local_ai.tts_provider = "piper"` runs [Piper](https://github.com/rhasspy/piper) for spoken replies. OpenHuman does not ship or install Piper or its voices. Install the Piper binary and a voice model yourself, and point the `PIPER_BIN` environment variable at the binary.

## Privacy

[Privacy mode](../privacy-mode.md) set to `local_only` refuses every non-local chat provider and permits only the local prefixes above. It does not start a runtime or fetch models. With privacy mode on and no reachable local runtime, workloads fail instead of falling back to the cloud.

Without privacy mode, a lightweight hint routed to an unreachable local provider falls back to the remote provider. Use privacy mode if strict locality matters.

OpenHuman also keeps local and hosted endpoints apart. If `api_url` looks like a local model server (a loopback or private host on a common runner port such as 11434, 1234, 8000 or 8080, or a `/v1/chat/completions` path), backend requests ignore it and go to the hosted API. Set local endpoints through a provider or `local_ai.base_url`, not `api_url`.

## Local vision

Vision is separate from chat, and most small local models cannot do it. Ollama does not reject an image sent to a text-only model. It drops the image and answers from the prompt text, so you get a description of something the model never saw. OpenHuman checks the vision model's capabilities and refuses to route a vision request to a known chat-only model.

- `vision_provider` and `local_ai.vision_model_id` must name a vision-capable model that you have pulled. `moondream:1.8b-v2-q4_K_S` (about 1.7 GB) is a small option. `gemma3:4b-it-qat` handles chat and vision with one set of weights.
- Gemma 3 is text-only at 270M and 1B, and multimodal from 4B up. Gemma 3n is a different model and is text-only at every size.
- Leaving `vision_model_id` empty is a valid "no local vision" setup.
- To attach an image to a chat turn, the model also needs its vision flag set in the model registry. See [Local models and bring your own key](local-and-byok-models.md#attaching-images-in-chat-needs-one-more-flag).

## What you need

- The runtime installed and running. OpenHuman does not manage it.
- The models pulled or loaded in that runtime.
- Enough disk and RAM for the models you chose. As a rough guide, `bge-m3` is about 1.2 GB on disk and a 4B chat model about 4 GB. Plan for 8 GB or more of RAM, and 16 GB or more for larger models.

## Troubleshooting

- **The provider will not save, or the model list is empty.** The runtime is not reachable at the endpoint. Start it and check the port. For Ollama, run `curl http://localhost:11434/api/tags`.
- **The runtime says the model is missing.** The model is not pulled or loaded. Run `ollama pull <model>`, or load it in LM Studio, then retry.
- **LM Studio runs on a different port.** Set `local_ai.base_url` or `OPENHUMAN_LM_STUDIO_BASE_URL`. Load the model in LM Studio before OpenHuman calls it.
- **Ollama's context is too small for agent turns.** Set `local_ai.num_ctx` to send `options.num_ctx` with each Ollama chat request.

## See also

- [Use OpenHuman with a local model](../../guides/local-model.md): step-by-step setup.
- [Local models and bring your own key](local-and-byok-models.md): per-model capability table and BYOK setup.
- [Memory](../memory.md): memory is stored by its engine, not by a local model.
- [Model routing](README.md): how lightweight chat hints prefer the local provider.
- [Privacy mode](../privacy-mode.md): enforcing local-only inference.
