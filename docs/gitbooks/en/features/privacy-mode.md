---
description: >-
  One switch, enforced in the Rust core. Local-only mode blocks cloud model
  calls, network tools, web search, integrations and cloud embeddings, and
  allows only on-device runtimes. Hosted and external speech uploads are blocked;
  local Piper speech synthesis remains available.
icon: lock
---

# Privacy mode

Most assistants' privacy story is a paragraph in a system prompt. OpenHuman's is an enforcement point in the Rust core.

The `[privacy]` config block defines three modes:

| Mode | What it means |
| --- | --- |
| `standard` (default) | Normal operation. Managed cloud routing, BYO providers and local models are all available. |
| `local_only` | No inference leaves the device. Every external chat provider is refused when it is built: the managed cloud, BYO cloud keys, even CLI delegates like Claude Code. Network tools, web search, integrations and cloud embeddings are refused at their egress points. Only local runtimes pass: Ollama, LM Studio, MLX and local OpenAI-compatible endpoints. Hosted and external speech recognition and synthesis are refused. Local Piper synthesis remains available; dictation has no local STT fallback. |
| `sensitive` | The base for an upcoming PII-aware tier (detection, redaction, destination disclosure). Today it behaves like `standard`. |

## Why enforcement matters

Privacy mode is not a policy the model is asked to follow. The first check lives in the inference provider factory (`crates/openhuman-core/src/inference/provider/factory/`). Under `local_only`, the core refuses to build an external provider at all. The error names the blocked provider and tells you how to fix it: switch to a local model, or change the mode in Settings.

That makes the guarantee independent of prompts, agents, tools or upstream bugs. If any code path tries to reach a cloud model in local-only mode, it cannot get a client.

`local_only` is also enforced at the egress points where the agent sends your data anywhere else (`crates/openhuman-core/src/security/egress/enforce.rs`):

| Egress point | Behavior under `local_only` |
| --- | --- |
| Network tools (`http_request`, `web_fetch`, `curl`) | Refused, with a policy-blocked message in the tool result |
| Web search (Exa, Tavily) | Refused the same way                                      |
| Composio tool calls and integration requests | Refused with an error                                     |
| Hosted and external STT/TTS | Refused before audio or speech text is uploaded |
| Local Piper TTS | Permitted; speech synthesis stays on the device |
| Cloud embeddings | Refused with an error                                     |
| Local runtimes (Ollama, LM Studio, MLX, local OpenAI-compatible) | Always permitted, nothing leaves the device               |

One deliberate exemption: backend control-plane round-trips keep flowing. These are sign-in, session, team, billing, and integration connection management and catalog reads. Blocking them breaks the app for no privacy gain. They carry auth tokens, ids and routing metadata, never user content.

Hosted and external speech requests enforce the live privacy policy before dispatch, so local-only mode blocks microphone uploads and cloud speech synthesis. Dictation fails with a privacy error because no local speech-to-text fallback exists in the current product. Local Piper synthesis remains available because it processes speech text on the device without contacting an external provider.

Permitted speech requests disclose their configured destination origin without including audio or spoken text in the disclosure event emitted before the request. External speech clients refuse HTTP redirects so user content cannot silently move to an undisclosed destination. Realtime voice sessions use a separate transport and remain outside this request-level enforcement change.

Privacy mode governs data egress. It is separate from the [autonomy tiers](approval-gate.md) (readonly, supervised, full), which govern what the agent may do. You can run a fully autonomous agent that never sends a byte of inference off-device.

## Pairing it with local models

Local-only mode works with [local AI](model-routing/local-ai.md). OpenHuman does not install runtimes or download models, so you set these up yourself before turning it on:

- Chat and reasoning on a runtime you run (Ollama, LM Studio, MLX, OMLX, or another OpenAI-compatible server), with the models you pulled, added as a provider under **Connections → LLM**.
- Piper text-to-speech, if you install the Piper binary and a voice yourself and set `PIPER_BIN`.
- Memory on a CortexDB endpoint you run yourself. The hosted TinyHumans engine is a cloud call, so local-only mode blocks it.

With local-only on, a workload routed to a runtime that is not running, or to a model you have not pulled, fails instead of falling back to the cloud.

See [Use OpenHuman with a local model](../guides/local-model.md) for a full local setup, and [Keep sensitive data private](../guides/privacy-sensitive-data.md) for a broader privacy walkthrough.

## See also

- [Privacy and security](privacy-and-security.md): the full trust model (approval gate, sandboxing, path roots, command classification).
- [OS keyring and secret storage](os-keyring-and-secret-storage.md): where credentials live.
- [Local AI](model-routing/local-ai.md): the on-device model runtimes.
