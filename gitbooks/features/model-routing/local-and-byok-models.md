---
description: >-
  Three ways to power OpenHuman: the managed subscription, your own provider key
  (BYOK), or fully local models via Ollama. What each one supports for chat,
  vision, and embeddings, and how to configure it.
icon: sliders
---

# Local models & bring your own key

The OpenHuman subscription is the **default**, not a requirement. Inference can come from any of three places, and you can mix them per workload: run embeddings locally, chat on your own Anthropic key, and leave vision on the managed route, all at once.

This page covers how to set up the two self-owned options and, importantly, **what each one can actually do**. Not every local model can see images, and picking a chat-only model for vision work is the single most common way to end up with a setup that looks configured but quietly does the wrong thing.

## The three routes at a glance

| | **Managed (default)** | **BYOK cloud** | **Local (Ollama / LM Studio)** |
| --- | --- | --- | --- |
| **Chat & reasoning** | Included | Your key, your billing | Yes, quality scales with model size |
| **Vision** | Included | Your key, if the model supports images | Yes, but only with a vision-capable model |
| **Embeddings** | Included | Your key, if the provider serves embeddings | Yes, `bge-m3` recommended |
| **Speech to text** | Included | Not routed through BYOK | Local Whisper available |
| **Text to speech** | Included | Not routed through BYOK | Local Piper available |
| **Web search** | Included, no key needed | Bring your own Exa key | Not applicable |
| **Inference data leaves your machine** | Yes, to the OpenHuman backend | Yes, to your chosen provider | No |
| **API keys to manage** | None | One per provider | None |

That last row is deliberately about **inference data only**. Sign-in, managed integration OAuth, billing, and hosted features such as meeting agents still use the OpenHuman backend even when inference is entirely yours, so running local models is not by itself a guarantee that nothing leaves the machine. If you want a hard guarantee that no inference leaves the machine, use [Privacy Mode](../privacy-mode.md), which enforces the local-only path in the Rust core rather than relying on configuration alone.

## Route A: local models with Ollama

### 1. Install Ollama and pull a model

Install [Ollama](https://ollama.com), then pull what you need. Every model named on this page is pullable from the public Ollama library with no extra setup:

```bash
ollama pull gemma3:1b-it-qat      # small chat model
ollama pull bge-m3                # embeddings
ollama pull moondream:1.8b-v2-q4_K_S   # vision, small
```

### 2. Know what each model supports

This is the part that bites people. A model that only does text will still **accept** an image request on Ollama: it silently drops the image and answers from the prompt text alone, which reads as a confident but entirely invented description. OpenHuman guards against this by refusing to route a vision request at a chat-only model, but it is worth knowing which is which.

| Model | Download | Chat | Vision | Embeddings |
| --- | --- | --- | --- | --- |
| `gemma3:270m-it-qat` | 0.2 GB | Yes | No | No |
| `gemma3:1b-it-qat` | 1.0 GB | Yes | No | No |
| `gemma3:4b-it-qat` | 4.0 GB | Yes | **Yes** | No |
| `gemma3n:e4b-it-q8_0` | 9.5 GB | Yes | No | No |
| `gemma4:e4b-it-q8_0` | 11.6 GB | Yes | **Yes** | No |
| `moondream:1.8b-v2-q4_K_S` | 1.7 GB | Minimal | **Yes** | No |
| `llava:7b` | 4.7 GB | Minimal | **Yes** | No |
| `bge-m3` | 1.2 GB | No | No | **Yes**, 1024 dim |
| `all-minilm:latest` | 0.05 GB | No | No | 384 dim, too small for Memory Tree |

Two traps worth calling out:

- **Gemma 3 is split by size.** The 270M and 1B builds are text-only. Vision starts at 4B. Picking `gemma3:1b-it-qat` for vision gets you a text-only model.
- **`gemma3n` is not `gemma3`.** Despite the name, Gemma 3n is a separate, text-only model on Ollama. It is a fine chat model and a bad vision model.

For embeddings, prefer **`bge-m3`**. The Memory Tree stores vectors in a fixed 1024-dimension on-disk format, so a 384-dimension model such as `all-minilm` or a 768-dimension model such as `nomic-embed-text` will fail the dimension check at embed time.

### 3. Point OpenHuman at it

The quickest path is the desktop app: **Settings → AI & Skills → Local AI** exposes RAM tier presets that set every model ID for you and pull the weights. The tiers are:

| Tier | Chat | Vision | Embeddings | Download |
| --- | --- | --- | --- | --- |
| 1 GB | `gemma3:270m-it-qat` | Disabled | `all-minilm:latest` (see note) | ~0.3 GB |
| 2-4 GB | `gemma3:1b-it-qat` | Disabled | `bge-m3` | ~2.3 GB |
| 4-8 GB | `gemma3:1b-it-qat` | `moondream:1.8b-v2-q4_K_S` | `all-minilm:latest` (see note) | ~2.8 GB |
| 8-16 GB | `gemma3:4b-it-qat` | `gemma3:4b-it-qat` | `bge-m3` | ~5.2 GB |
| 16 GB+ | `gemma4:e4b-it-q8_0` | `gemma4:e4b-it-q8_0` | `bge-m3` | ~12.8 GB |

The two highest tiers use one multimodal model for both chat and vision, so you download a single set of weights rather than a chat model plus a separate vision sidecar.

{% hint style="warning" %}
**The 1 GB and 4-8 GB tiers ship `all-minilm:latest`, which the Memory Tree cannot use.** It emits 384-dimension vectors and the Memory Tree's on-disk format is fixed at 1024, so memory embedding fails the dimension check at embed time. Those two tiers are usable for local chat and, on the 4-8 GB tier, vision, but if you want local Memory Tree embeddings set `embedding_model_id = "bge-m3"` explicitly after applying the preset, or pick the 2-4 GB tier or above. Aligning those presets is tracked as follow-up.
{% endhint %}

To configure by hand, the keys live under `[local_ai]` in `config.toml`:

```toml
[local_ai]
runtime_enabled = true
opt_in_confirmed = true
provider = "ollama"                       # or "lm_studio"
chat_model_id = "gemma3:4b-it-qat"
vision_model_id = "gemma3:4b-it-qat"      # must be vision-capable
embedding_model_id = "bge-m3"
```

Leaving `vision_model_id` empty means "no local vision", which is a valid setup. A vision request then returns a message telling you what to set, rather than failing silently.

### 4. Route workloads to it

Turning local AI on does not move everything on-device. You choose per workload with a provider string of the form `ollama:<model>`:

```toml
chat_provider = "ollama:gemma3:4b-it-qat"
vision_provider = "ollama:gemma3:4b-it-qat"
embeddings_provider = "ollama:bge-m3"
```

The full set of workload fields is `chat_provider`, `reasoning_provider`, `agentic_provider`, `coding_provider`, `vision_provider`, `memory_provider`, `embeddings_provider`, `heartbeat_provider`, `learning_provider`, and `subconscious_provider`. Any field left unset, blank, or set to `cloud` stays on the default route.

#### Attaching images in chat needs one more flag

`vision_provider` routes the **vision workload** — image summaries and the OCR/description path. It does not by itself let you attach an image to a chat or agent turn.

A turn only rehydrates image attachments when the resolved chat model is known to accept them, and for a local model that knowledge comes from the per-model registry, not from `vision_provider`. Set the model's **vision** flag in Settings → AI (the custom-model dialog), which records it in `model_registry`:

```toml
[[model_registry]]
id = "gemma3:4b-it-qat"
provider = "ollama"
vision = true
```

Without that flag the images are stripped before dispatch and the model answers from the text alone — fluently, and with no indication that it never saw the picture.

See [Local AI (optional)](local-ai.md) for the deeper runtime detail, LM Studio setup, and troubleshooting.

## Route B: bring your own key

BYOK keeps the routing, memory, tools, and agent harness exactly as they are, and swaps out who serves the tokens. Your key, your account, your billing, no OpenHuman inference charges.

### 1. Add the provider

Add your key in the desktop app under the LLM settings, which stores it in the OS keyring rather than in plain config. OpenHuman ships presets for these slugs, so you do not need to supply an endpoint:

`openai`, `anthropic`, `google`, `openrouter`, `orcarouter`, `groq`, `mistral`, `deepseek`, `together`, `fireworks`, `cerebras`, `xai`, `moonshot`, `gmi`, `huggingface`, `nvidia`, `zai`, `minimax`, `stepfun`, `kilocode`, `deepinfra`, `novita`, `venice`, `vercel-ai-gateway`, `sumopod`, `modelscope`

Anything else that speaks the OpenAI-compatible API works too: register it with your own slug and endpoint, and it routes the same way.

### 2. Route workloads to it

Provider strings follow `<slug>:<model>`, using the same workload fields as the local route:

```toml
chat_provider = "anthropic:claude-sonnet-4"
reasoning_provider = "openai:gpt-5.1"
coding_provider = "deepseek:deepseek-coder"
vision_provider = "openai:gpt-5.1"
```

To make one provider the default for everything that is not pinned, set `primary_cloud` to its slug. Every workload left on `cloud` then resolves to that provider instead of the OpenHuman backend.

### 3. Check the model supports the workload

BYOK inherits your provider's capabilities, not OpenHuman's. Before pinning `vision_provider`, confirm the model you named accepts image input, and before pinning `embeddings_provider`, confirm the provider serves an embeddings endpoint. Not every chat provider does.

## Mixing routes

The workload fields are independent, so a common privacy-conscious setup keeps recurring background work on-device and reserves a strong cloud model for the turns that need it:

```toml
# On-device: everything that runs constantly over personal data
embeddings_provider = "ollama:bge-m3"
memory_provider = "ollama:gemma3:1b-it-qat"
heartbeat_provider = "ollama:gemma3:1b-it-qat"
subconscious_provider = "ollama:gemma3:1b-it-qat"

# Your own key: the turns where quality matters
chat_provider = "anthropic:claude-sonnet-4"
reasoning_provider = "anthropic:claude-sonnet-4"
```

## Troubleshooting

**"no local vision model is configured"** means `local_ai.vision_model_id` is empty. Set it to a vision-capable model and pull it, or point `vision_provider` at a cloud model instead.

**"local vision model ... is not available"** means the model is configured but not pulled. Run the `ollama pull` command in the message.

**Vision answers look plausible but describe the wrong image.** You are almost certainly on a chat-only model. Check `vision_model_id` against the capability table above. Current builds refuse this routing and fall back to a vision-capable model, so this points at an older build or a provider outside the local path.

**A model you selected keeps reverting.** Local chat model IDs are checked against a supported list, and an unrecognized ID falls back to the default. Use one of the IDs from the tier table.

**Embeddings fail with a dimension error.** The Memory Tree needs 1024-dimension vectors. Use `bge-m3`.

## See also

- [Local AI (optional)](local-ai.md). Runtime detail, LM Studio, and the opt-in flags.
- [Automatic Model Routing](README.md). How hints pick a model per task.
- [Privacy Mode](../privacy-mode.md). Enforcing local-only inference in the core.
- [Privacy & Security](../privacy-and-security.md). What moves on-device when you opt in.
