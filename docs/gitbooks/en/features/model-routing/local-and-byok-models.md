---
description: >-
  Three ways to power OpenHuman: the managed subscription, your own provider key
  (BYOK), or local models on a runtime you run yourself. What each one supports
  for chat, vision and embeddings, and how to set it up.
icon: sliders
---

# Local models and bring your own key

The OpenHuman subscription is the default, not a requirement. Inference can come from any of three places, and you can mix them for each workload. For example, you can run embeddings locally, chat on your own Anthropic key, and leave vision on the managed route.

This page covers how to set up the two self-owned options and what each one can actually do. Not every local model can see images. Picking a chat-only model for vision work is the most common way to end up with a setup that looks configured but quietly does the wrong thing.

## The three routes at a glance

|                                        | Managed (default)             | BYOK cloud                                  | Local (runtime you run)                   |
| -------------------------------------- | ----------------------------- | ------------------------------------------- | ----------------------------------------- |
| **Chat and reasoning**                 | Included                      | Your key, your billing                      | Yes, quality scales with model size       |
| **Vision**                             | Included                      | Your key, if the model supports images      | Yes, but only with a vision-capable model |
| **Embeddings**                         | Included                      | Your key, if the provider serves embeddings | Yes, `bge-m3` recommended                 |
| **Speech to text**                     | Included                      | Your own key, via a voice provider slug     | No local STT engine                       |
| **Text to speech**                     | Included                      | Your own key, via a voice provider slug     | Piper, if you install it (`PIPER_BIN`)    |
| **Web search**                         | Included, no key needed       | Bring your own Exa key                      | Not applicable                            |
| **Inference data leaves your machine** | Yes, to the OpenHuman backend | Yes, to your chosen provider                | No                                        |
| **API keys to manage**                 | None                          | One per provider                            | None                                      |

Speech is configured separately from the LLM workload fields, and the two halves are easy to confuse. `voice_providers` holds the third-party provider definitions (slug, endpoint, key, default model or voice). Editing one does not change which provider is used. The active route is set separately:

- STT reads `stt_provider`, then the older `local_ai.stt_provider`, then `voice_server.stt_engine` when neither names a provider.
- TTS reads `tts_provider`, then the older `local_ai.tts_provider`, then `cloud`.

There is no local STT engine. Speech-to-text is either the hosted proxy or a third-party API with your own key. TTS has a local option in Piper. OpenHuman does not install Piper. Install the binary and a voice yourself, set `PIPER_BIN`, and set `tts_provider = "piper"`.

The last row of the table is about inference data only. Sign-in, managed integration OAuth, billing, and hosted features such as media generation and managed web search still use the OpenHuman backend even when inference is entirely yours. Running local models is not by itself a guarantee that nothing leaves the machine. If you need that guarantee, use [Privacy mode](../privacy-mode.md), which enforces the local-only path in the Rust core instead of relying on configuration alone.

## Route A: local models

OpenHuman does not install a runtime or download model weights. You run the runtime (Ollama, LM Studio, MLX, OMLX, or another OpenAI-compatible server), you pull the models, and OpenHuman calls the endpoint. The examples below use Ollama.

### 1. Install Ollama and pull a model

Install [Ollama](https://ollama.com), then pull what you need. Every model named on this page is in the public Ollama library.

```bash
ollama pull gemma3:1b-it-qat      # small chat model
ollama pull bge-m3                # embeddings
ollama pull moondream:1.8b-v2-q4_K_S   # vision, small
```

### 2. Know what each model supports

A text-only model on Ollama still accepts an image request. It silently drops the image and answers from the prompt text alone, which reads as a confident but invented description. OpenHuman refuses to route a vision request to a chat-only model, but it helps to know which is which.

| Model                      | Size     | Chat    | Vision  | Embeddings                         |
| -------------------------- | -------- | ------- | ------- | ---------------------------------- |
| `gemma3:270m-it-qat`       | 0.2 GB   | Yes     | No      | No                                 |
| `gemma3:1b-it-qat`         | 1.0 GB   | Yes     | No      | No                                 |
| `gemma3:4b-it-qat`         | 4.0 GB   | Yes     | Yes     | No                                 |
| `gemma3n:e4b-it-q8_0`      | 9.5 GB   | Yes     | No      | No                                 |
| `gemma4:e4b-it-q8_0`       | 11.6 GB  | Yes     | Yes     | No                                 |
| `moondream:1.8b-v2-q4_K_S` | 1.7 GB   | Minimal | Yes     | No                                 |
| `llava:7b`                 | 4.7 GB   | Minimal | Yes     | No                                 |
| `bge-m3`                   | 1.2 GB   | No      | No      | Yes, 1024 dimensions               |
| `all-minilm:latest`        | 0.05 GB  | No      | No      | 384 dimensions                     |

Two traps:

- Gemma 3 is split by size. The 270M and 1B builds are text-only. Vision starts at 4B. If you pick `gemma3:1b-it-qat` for vision, you get a text-only model.
- `gemma3n` is not `gemma3`. Despite the name, Gemma 3n is a separate, text-only model on Ollama. It is a fine chat model and a bad vision model.

For embeddings, prefer `bge-m3` (1024 dimensions). An `embeddings_provider` of `ollama:bge-m3` routes OpenHuman's own embedding calls to your machine. It does not move memory, because the memory engine embeds on its own side, so recall still runs wherever the engine runs. Three separate settings are involved. This field routes OpenHuman's embeddings. [Privacy mode](../privacy-mode.md) enforces where inference may go. The memory engine you choose decides where memory lives.

### 3. Point OpenHuman at it

In the desktop app, open **Connections > LLM > Add a provider** and pick Ollama (or LM Studio or OMLX) under **Local runtimes**, or add a custom OpenAI-compatible endpoint. OpenHuman saves the endpoint, turns on the local runtime, and lists the models the runtime reports. Models you have not pulled do not appear. See [Local AI](local-ai.md#adding-a-local-runtime-in-the-app).

To configure by hand, use the `[local_ai]` section of `config.toml`:

```toml
[local_ai]
runtime_enabled = true
opt_in_confirmed = true
provider = "ollama"                       # or "lm_studio"
chat_model_id = "gemma3:4b-it-qat"
vision_model_id = "gemma3:4b-it-qat"      # must be vision-capable
embedding_model_id = "bge-m3"
```

Every model you name must already be pulled. Leaving `vision_model_id` empty means "no local vision", which is a valid setup.

### 4. Route workloads to it

Adding a local provider does not move everything on-device. You choose for each workload with a provider string such as `ollama:<model>` (or `lmstudio:`, `mlx:`, `omlx:`, `local-openai:`):

```toml
chat_provider = "ollama:gemma3:4b-it-qat"
vision_provider = "ollama:gemma3:4b-it-qat"
embeddings_provider = "ollama:bge-m3"
```

The workload fields are `chat_provider`, `reasoning_provider`, `agentic_provider`, `coding_provider`, `vision_provider`, `memory_provider`, `embeddings_provider` and `learning_provider`. A field that is unset, blank or `cloud` stays on the default route.

#### Attaching images in chat needs one more flag

`vision_provider` routes the vision workload: image summaries and the OCR and description path. It does not by itself let you attach an image to a chat or agent turn.

A turn passes image attachments through only when the chat model is known to accept them. For a local model, that knowledge comes from the per-model registry, not from `vision_provider`. Set the model's vision flag in **Settings > AI** (the custom-model dialog), which records it in `model_registry`:

```toml
[[model_registry]]
id = "gemma3:4b-it-qat"
provider = "ollama"
vision = true
```

Without that flag, images are stripped before the request is sent. The model answers from the text alone, fluently, with no sign that it never saw the picture.

See [Local AI](local-ai.md) for endpoint overrides, the other runtimes and troubleshooting.

## Route B: bring your own key

BYOK keeps the routing, memory, tools and agent harness exactly as they are, and changes only who serves the tokens. It is your key, your account and your billing, with no OpenHuman inference charges.

### 1. Add the provider

Add your key in the desktop app under the LLM settings. It is stored in the OS keyring, not in plain config. OpenHuman ships presets for these slugs, so you don't need to supply an endpoint:

`openai`, `anthropic`, `google`, `openrouter`, `orcarouter`, `groq`, `mistral`, `deepseek`, `together`, `fireworks`, `cerebras`, `xai`, `moonshot`, `gmi`, `huggingface`, `nvidia`, `zai`, `minimax`, `stepfun`, `kilocode`, `deepinfra`, `novita`, `venice`, `vercel-ai-gateway`, `sumopod`, `modelscope`

Anything else that speaks the OpenAI-compatible API also works. Register it with your own slug and endpoint, and it routes the same way.

For an HTTPS endpoint signed by a private or corporate CA, upload its PEM CA certificate in the custom provider editor. OpenHuman trusts that certificate for this provider's model listing and inference requests. You can also install the CA in the operating system's trust store; on macOS and Linux, `SSL_CERT_FILE` supplies a PEM bundle to the core process.

On a headless core (`openhuman-core serve`), custom cloud providers are built only when a backend session or TinyHumans API key is present. If you run the core with no account, set the endpoint up as the `local-openai` runtime instead: set `LOCAL_OPENAI_URL`, put the key in `local_ai.api_key`, and pin workloads to `local-openai:<model>`. See [Headless without a TinyHumans account](../cloud-deploy.md#headless-without-a-tinyhumans-account).

### 2. Route workloads to it

Provider strings follow `<slug>:<model>` and use the same workload fields as the local route:

```toml
chat_provider = "anthropic:claude-sonnet-4"
reasoning_provider = "openai:gpt-5.1"
coding_provider = "deepseek:deepseek-coder"
vision_provider = "openai:gpt-5.1"
```

To make one provider the default for everything that is not pinned, set `primary_cloud` to its slug. Every workload left on `cloud` then goes to that provider and not to the OpenHuman backend.

### 3. Check that the model supports the workload

BYOK gets your provider's capabilities, not OpenHuman's. Before you pin `vision_provider`, confirm the model accepts image input. Before you pin `embeddings_provider`, confirm the provider serves an embeddings endpoint. Not every chat provider does.

## Mixing routes

The workload fields are independent. A common privacy-minded setup keeps recurring background work on-device and saves a strong cloud model for the turns that need it:

```toml
# On-device: everything that runs constantly over personal data
embeddings_provider = "ollama:bge-m3"
memory_provider = "ollama:gemma3:1b-it-qat"
learning_provider = "ollama:gemma3:1b-it-qat"

# Your own key: the turns where quality matters
chat_provider = "anthropic:claude-sonnet-4"
reasoning_provider = "anthropic:claude-sonnet-4"
```

## Troubleshooting

**The model you want is not offered, or a request says the model is missing.** The runtime does not have it. Run `ollama pull <model>` (or load it in LM Studio), then retry. OpenHuman does not pull models.

**Vision answers look plausible but describe the wrong image.** You are almost certainly on a chat-only model. Check `vision_model_id` against the capability table above. Current builds refuse this routing and fall back to a vision-capable model, so this points to an older build or a provider outside the local path.

## See also

- [Local AI](local-ai.md): supported runtimes, endpoints and the opt-in flags.
- [Model routing](README.md): how hints pick a model for each task.
- [Privacy mode](../privacy-mode.md): enforcing local-only inference in the core.
- [Privacy and security](../privacy-and-security.md): what moves on-device when you opt in.
