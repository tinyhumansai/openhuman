---
description: >-
  One switch, enforced in the Rust core: local-only mode blocks every cloud
  model call, plus network tools, web search, integrations and cloud embeddings,
  permitting only on-device runtimes (Ollama, LM Studio, MLX, local
  OpenAI-compatible endpoints). Voice is the documented exception: there is no
  local speech-to-text engine, so transcription still leaves the device.
icon: lock
---

# Privacy Mode

Most assistants' privacy stories are a paragraph in a system prompt. OpenHuman's is an **enforcement chokepoint in the Rust core**.

The `[privacy]` config block defines three modes:

| Mode                       | What it means                                                                                                                                                                                                                                                                                                                                                                                                                             |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`standard`** _(default)_ | Normal operation: managed cloud routing, BYO providers, and local models all available.                                                                                                                                                                                                                                                                                                                                                   |
| **`local_only`**           | **No inference leaves the device.** Every external chat provider (the managed cloud, BYO cloud keys, even CLI delegates like Claude Code) is refused at construction time, and network tools, web search, integrations, and cloud embeddings are refused at their egress points. Only local runtimes pass: Ollama, LM Studio, MLX, and local OpenAI-compatible endpoints. Voice is the one exception, because no local STT engine exists. |
| **`sensitive`**            | Foundation for the upcoming PII-aware tier (detection, redaction, destination disclosure). Today it behaves like `standard`.                                                                                                                                                                                                                                                                                                              |

## Why "enforced" matters

Privacy Mode is deliberately **not** a policy the model is asked to follow. The first check lives in the inference provider factory (`crates/openhuman-core/src/inference/provider/factory/`): under `local_only`, the core refuses to _build_ an external provider at all, and the error names exactly which provider was blocked and tells you how to fix it: switch to a local model, or change the mode in Settings.

That makes the guarantee independent of prompts, agents, tools, or bugs upstream: if a code path anywhere in the app tries to reach a cloud model while you're in local-only mode, it structurally cannot get a client.

Inference is no longer the only chokepoint. `local_only` is also enforced at the egress points where the agent ships your data anywhere else (`crates/openhuman-core/src/security/egress/enforce.rs`):

| Egress point                                                     | Behaviour under `local_only`                              |
| ---------------------------------------------------------------- | --------------------------------------------------------- |
| Network tools (`http_request`, `web_fetch`, `curl`)              | Refused, with a policy-blocked message in the tool result |
| Web search (Exa, Tavily)                                         | Refused the same way                                      |
| Composio tool calls and integration requests                     | Refused with an error                                     |
| Cloud embeddings                                                 | Refused with an error                                     |
| Local runtimes (Ollama, LM Studio, MLX, local OpenAI-compatible) | Always permitted, nothing leaves the device               |

One deliberate exemption: backend **control-plane** round-trips (sign-in, session, team, billing, and the integration connection-management and catalog reads) keep flowing, because blocking them breaks the app for no privacy gain. They carry auth tokens, ids, and routing metadata, never user content.

**Voice is not covered.** There is no local speech-to-text engine, so dictation and transcription still leave the device under `local_only` - to the hosted OpenHuman STT proxy, or to whichever third-party STT provider you configured. If that matters for your threat model, leave voice off.

Privacy Mode governs **data egress**. It is orthogonal to the [autonomy tiers](privacy-and-security.md) (readonly / supervised / full), which govern what the agent may _do_. You can run a fully autonomous agent that never sends a byte of inference off-device.

## Pairing it with local models

Local-only mode is designed to work with [Local AI](model-routing/local-ai.md). OpenHuman does not install runtimes or download models, so you set these up yourself before turning it on:

- Chat and reasoning on a runtime you run (**Ollama, LM Studio, MLX, OMLX**, or another OpenAI-compatible server), with the models you pulled, added as a provider under **Connections → LLM**.
- **Piper** text-to-speech, if you install the Piper binary and a voice yourself and set `PIPER_BIN`.
- Memory stays on a CortexDB endpoint you run yourself; the hosted TinyHumans engine is a cloud call and is blocked by local-only mode.

With local-only on, a workload routed to a runtime that isn't running, or to a model you haven't pulled, fails instead of falling back to the cloud.

See the [Use OpenHuman with a local model](../guides/local-model.md) guide for a full local setup, and [Keep sensitive data private](../guides/privacy-sensitive-data.md) for a broader privacy walkthrough.

## See also

- [Privacy & Security](privacy-and-security.md): the full trust model (approval gate, sandboxing, path roots, command classification).
- [OS Keyring & Secret Storage](os-keyring-and-secret-storage.md): where credentials live.
- [Local AI](model-routing/local-ai.md): the on-device model runtimes.
