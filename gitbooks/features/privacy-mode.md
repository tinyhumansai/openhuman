---
description: >-
  One switch, enforced in the Rust core: local-only mode blocks every cloud
  model call and permits only on-device runtimes (Ollama, LM Studio, MLX, local
  OpenAI-compatible endpoints).
icon: lock
---

# Privacy Mode

Most assistants' privacy stories are a paragraph in a system prompt. OpenHuman's is an **enforcement chokepoint in the Rust core**.

The `[privacy]` config block defines three modes:

| Mode | What it means |
| --- | --- |
| **`standard`** *(default)* | Normal operation: managed cloud routing, BYO providers, and local models all available. |
| **`local_only`** | **No inference leaves the device.** Every external chat provider (the managed cloud, BYO cloud keys, even CLI delegates like Claude Code) is refused at construction time. Only local runtimes pass: Ollama, LM Studio, MLX, and local OpenAI-compatible endpoints. |
| **`sensitive`** | Foundation for the upcoming PII-aware tier (detection, redaction, destination disclosure). Today it behaves like `standard`. |

## Why "enforced" matters

Privacy Mode is deliberately **not** a policy the model is asked to follow. The check lives in the inference provider factory (`src/openhuman/inference/provider/factory.rs`): under `local_only`, the core refuses to *build* an external provider at all, and the error names exactly which provider was blocked and tells you how to fix it: switch to a local model, or change the mode in Settings.

That makes the guarantee independent of prompts, agents, tools, or bugs upstream: if a code path anywhere in the app tries to reach a cloud model while you're in local-only mode, it structurally cannot get a client.

Privacy Mode governs **data egress**. It is orthogonal to the [autonomy tiers](privacy-and-security.md) (readonly / supervised / full), which govern what the agent may *do*. You can run a fully autonomous agent that never sends a byte of inference off-device.

## Pairing it with local models

Local-only mode is designed to work with OpenHuman's [Local AI](model-routing/local-ai.md) stack:

* Chat and reasoning via **Ollama / LM Studio / MLX** models you download in Settings.
* **In-process Whisper** speech-to-text (tiny → large-v3-turbo, one-click installer). No external binary, no cloud STT.
* **Piper** text-to-speech, installed the same way.
* Local embeddings for [Memory Tree](obsidian-wiki/memory-tree.md) retrieval.

See the [Use OpenHuman with a local model](../guides/local-model.md) guide for a full local setup, and [Keep sensitive data private](../guides/privacy-sensitive-data.md) for a broader privacy walkthrough.

## See also

* [Privacy & Security](privacy-and-security.md): the full trust model (approval gate, sandboxing, path roots, command classification).
* [OS Keyring & Secret Storage](os-keyring-and-secret-storage.md): where credentials live.
* [Local AI](model-routing/local-ai.md): the on-device model runtimes.
