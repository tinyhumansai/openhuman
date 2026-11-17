---
description: >-
  Send different OpenHuman workloads to models on different machines you own,
  behind one OpenAI-compatible gateway, with a Mac mini and Linux GPU PC
  example.
icon: network-wired
---

# Use multiple local LLM servers

This guide shows how to keep a small, fast model and a large, slow one on separate machines you own, and send each OpenHuman workload to the right one.

OpenHuman talks to one endpoint per provider string. If your models live on more than one machine, put a gateway in front of them. The gateway exposes a single OpenAI-compatible endpoint and uses a prefix on the model ID to pick the upstream. This guide uses [Model Router](https://github.com/tjm8874/model-router), an independent, MIT-licensed community project. OpenHuman does not bundle, install or supervise it, and any compatible gateway works the same way.

The example puts the OpenHuman core, the gateway and a small model on a Mac mini, and a larger model server on a Linux GPU PC:

```text
OpenHuman core (Mac mini)
  └─ gateway, 127.0.0.1:18080/v1
       ├─ MacMini:small-model → local model server, 127.0.0.1:8080/v1
       └─ LinuxGPU:large-model → SSH tunnel → Linux PC, 127.0.0.1:8000/v1
```

## Prerequisites

- A single-server local setup you already understand. Start with [Use OpenHuman with a local model](local-model.md) if you have not routed one workload locally yet.
- An OpenAI-compatible model server on each machine. You install it, start it and load models yourself. Neither OpenHuman nor the gateway does that.
- Enough memory and context room on each machine for your models, including KV cache and the concurrency you expect.
- SSH access from the machine running the core to each remote machine.

## Privacy implications

- A gateway on loopback is not on-device inference. It listens on `127.0.0.1`, but it forwards prompts to whatever upstream its config names, which may be another machine.
- OpenHuman treats the `local-openai:` prefix as a local runtime and does not inspect the gateway's upstreams. `local_only` privacy mode will not stop a prompt that the gateway relays off the box, so review those destinations yourself.
- Keep authenticated remote inference behind SSH or HTTPS. Do not send bearer keys over plain HTTP across a network.
- Changing inference destinations does not move the core's workspace, memory or project files.

## Steps

### 1. Start the model servers

Run the small model on the Mac mini at `http://127.0.0.1:8080/v1`, for example under MLX or LM Studio. Run the larger model on the Linux PC at its own `http://127.0.0.1:8000/v1`, for example under vLLM. Follow each runtime's install and model-loading instructions.

Then, on the Mac mini, forward the Linux endpoint over SSH and keep the tunnel up:

```sh
ssh -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 \
  -L 127.0.0.1:18000:127.0.0.1:8000 user@linux-gpu.example
```

The Linux server is now reachable on the Mac mini as `http://127.0.0.1:18000/v1` on the Mac mini.

### 2. Start the gateway

Build and run the gateway following its README. The backend mapping for this example looks like this:

```toml
[server]
host = "127.0.0.1"
port = 18080

[backends.MacMini]
base_url = "http://127.0.0.1:8080/v1"
endpoints = ["chat/completions"]

[backends.LinuxGPU]
base_url = "http://127.0.0.1:18000/v1"
endpoints = ["chat/completions"]
# For an authenticated upstream, name an environment variable the gateway reads:
# api_key_env = "LINUX_LLM_API_KEY"
```

Check the config, start it, and read back the catalogue:

```sh
model-router --config router.toml --check-config
model-router --config router.toml
curl http://127.0.0.1:18080/v1/models
```

Use the binary's real path if it is not on `PATH`. A config check does not contact the upstreams, so read `router_errors` as well as `data`, because one reachable backend does not mean every backend is ready. The catalogue prefixes each upstream model ID with its backend name, so `org/model:4bit` on the Mac mini appears as `MacMini:org/model:4bit`. Colons and slashes inside the upstream ID are left alone.

### 3. Point OpenHuman at the gateway

Merge the following into the active user's `config.toml`, keeping your other settings. Replace the placeholder model IDs with exact IDs from the gateway catalogue. Keep the workload keys at the top level, before the `[local_ai]` table, or TOML reads them as part of it:

```toml
chat_provider = "local-openai:MacMini:your-small-model-id"
memory_provider = "local-openai:MacMini:your-small-model-id"
reasoning_provider = "local-openai:LinuxGPU:your-large-model-id"
agentic_provider = "local-openai:LinuxGPU:your-large-model-id"
coding_provider = "local-openai:LinuxGPU:your-large-model-id"

[local_ai]
runtime_enabled = true
opt_in_confirmed = true
provider = "lm_studio"
base_url = "http://127.0.0.1:18080/v1"
chat_model_id = "MacMini:your-small-model-id"
vision_model_id = ""
```

Three keys need care:

| Key | What it does here |
| --- | --- |
| `local-openai:` prefix | Selects OpenHuman's generic OpenAI-compatible runtime (`crates/openhuman-core/src/inference/provider/factory/local_runtime.rs`). The rest of the string is passed to the endpoint unchanged, so `MacMini:your-small-model-id` reaches the gateway intact and the gateway strips `MacMini:` before forwarding. |
| `local_ai.base_url` | The endpoint every `local-openai:` route falls back to. This is what points OpenHuman at the gateway. |
| `local_ai.provider` | Selects which runtime the Local AI settings page talks to. It does not affect a `local-openai:` route, where the prefix decides. `lm_studio` is simply the existing identifier for a local OpenAI-compatible server. |

`OPENHUMAN_LOCAL_INFERENCE_URL` and `LOCAL_OPENAI_URL` in the core's environment both take priority over `local_ai.base_url`, in that order. Update or unset them when you change endpoints. The full precedence order is in [Local AI](../features/model-routing/local-ai.md#supported-runtimes).

Assign vision only to a vision-capable model, through `vision_provider` and `local_ai.vision_model_id`. This example is text-only and leaves vision and embeddings on their existing routes. Embeddings and the memory engine are configured separately, and a gateway does not reroute them. A gateway can forward an embeddings path to a compatible service, but it does not translate between APIs or start anything. Any workload you do not assign keeps its route.

### 4. Verify each route

Send a short chat message, which should land on the small model. Then run a reasoning or agentic turn, which should land on the large one. Confirm the backend and model in the gateway logs and the reply in OpenHuman. Model discovery and a `/health` probe do not prove inference works. If your models support streaming and tool calls, test both.

## Success checks

- [ ] `curl http://127.0.0.1:18080/v1/models` lists models from every backend, with no entries in `router_errors`.
- [ ] A chat turn answers, and the gateway log shows the Mac mini backend.
- [ ] A reasoning or agentic turn answers, and the gateway log shows the Linux backend.
- [ ] Killing the SSH tunnel makes the large-model workload fail instead of silently answering from somewhere else.

## Common failures

| What you see | Meaning | Fix |
| --- | --- | --- |
| The model is not found | The workload names an ID that is not in the gateway catalogue | Copy the exact prefixed ID from `/v1/models` |
| Only one backend answers | The other upstream is down, or its tunnel dropped | Check `router_errors`, then restart the server or the tunnel |
| Every local route hits the wrong endpoint | An environment override is still set | Unset `OPENHUMAN_LOCAL_INFERENCE_URL` and `LOCAL_OPENAI_URL`, or point them at the gateway |
| Workload keys are ignored | They were written after `[local_ai]`, so TOML read them as part of that table | Move them above the table |

## Keeping it up

Run the gateway under a per-user LaunchAgent or a systemd user service if you need it always available, and supervise the SSH tunnel separately. Re-check the gateway config and restart it after every change. This gateway returns an error when an upstream fails. It has no automatic retry and no fallback, which is what you want when the alternative is a silent reroute.

The gateway listens only on loopback and has no inbound authentication of its own. It keeps upstream keys separate from the core's `Authorization` header.

## Next steps

- [Use OpenHuman with a local model](local-model.md): the single-server version of this setup.
- [Local AI](../features/model-routing/local-ai.md): provider strings, endpoint precedence and every `local_ai` field.
- [Privacy mode](../features/privacy-mode.md): why `local_only` cannot vouch for a gateway's upstreams.
- [Keep sensitive data private](privacy-sensitive-data.md): what leaves your machines.
