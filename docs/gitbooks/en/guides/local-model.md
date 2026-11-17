---
description: >-
  Run a local model runtime yourself (Ollama, LM Studio, MLX or any
  OpenAI-compatible server), pull your own models, and add the endpoint to
  OpenHuman as a provider.
icon: microchip
---

# Use OpenHuman with a local model

This guide moves some or all of OpenHuman's model work onto your own computer, so the data for those workloads never leaves the machine.

OpenHuman does not install a runtime, start it or download models. You run the runtime and pull the models, and OpenHuman calls the endpoint you give it. Local AI is opt-in: adding a local provider reroutes nothing until you point workloads at it.

For the config reference (provider strings, endpoint overrides and every workload field), see [Local AI](../features/model-routing/local-ai.md). This page is the task-oriented version.

## Prerequisites

- A local runtime you install and run yourself:
  - [Ollama](https://ollama.com), default address `http://localhost:11434`.
  - [LM Studio](https://lmstudio.ai) with its local server enabled, default `http://localhost:1234/v1`.
  - MLX (`mlx_lm.server`), OMLX, or any other OpenAI-compatible server.
- Disk and RAM for the models you pick. A small chat model plus `bge-m3` for embeddings takes a few GB on disk. 8 GB of RAM is a sensible minimum.

## Privacy implications

- Workloads you route locally run on-device, and nothing about that work is sent out.
- Anything you leave on the default route still goes through the OpenHuman [model router](../features/model-routing/README.md). Local AI changes only the workloads you move.
- Without [Privacy mode](../features/privacy-mode.md), lightweight hints routed locally can fall back to the remote provider if the runtime is unreachable. Turn on `local_only` privacy mode if strict locality matters. An unreachable runtime then makes the request fail instead.

## Steps

### 1. Start the runtime

Install and launch the runtime so its server is running. For Ollama, confirm from a terminal:

```bash
curl http://localhost:11434/api/tags
```

A JSON list of models, even an empty one, means the server is reachable. For LM Studio or another OpenAI-compatible server, `curl http://localhost:1234/v1/models` (adjust the port) should return a model list.

### 2. Pull the models yourself

OpenHuman never pulls models. Pull every model you plan to use before you configure it. For Ollama:

```bash
ollama pull gemma3:4b-it-qat   # chat, and vision from 4B up
ollama pull bge-m3             # memory embeddings (1024 dimensions)
```

In LM Studio, download the model in LM Studio and load it. For MLX or another server, start it with the model you want served.

See [Local models and bring your own key](../features/model-routing/local-and-byok-models.md) for which models can do chat, vision and embeddings.

### 3. Add the endpoint as a provider

Open **Connections → LLM** and choose **Add a provider**:

- For Ollama, LM Studio or OMLX, pick it under **Local runtimes**. The default endpoint is filled in. Change it if your runtime listens elsewhere.
- For MLX or another server, choose **Add a custom provider** and enter its OpenAI-compatible endpoint, for example `http://127.0.0.1:8080/v1`.

When you save, OpenHuman asks the endpoint for its model list. If the runtime is not reachable, the provider is not saved. Fix step 1 and try again.

### 4. Route workloads to it

Use **Custom routing** on the workloads you want local (chat, reasoning, vision and so on) and pick a model the runtime reported. Only models you have pulled or loaded appear.

For memory embeddings, open **Connections → Embeddings** and choose Ollama with `bge-m3`, or set `embeddings_provider = "ollama:bge-m3"` in `config.toml`.

### 5. Test that it answers

Send a short message on a workload you routed locally, or use the model test in the LLM panel. A coherent reply means the path works end to end: the endpoint is reachable, the model is present and inference runs. For Ollama you can also watch `ollama ps` while the request runs.

## Success checks

Local AI is working when:

- [ ] The local provider shows as connected under **Connections → LLM**, and its model list contains the models you pulled.
- [ ] A turn routed to the local provider returns a real reply.
- [ ] For embeddings, new summaries keep appearing in the Memory tab after the next memory sync, with the runtime running.

## Common failures

| What you see                                                    | Meaning                                                                     | Fix                                                                                                         |
| --------------------------------------------------------------- | --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| The provider will not save and the endpoint is reported unreachable       | OpenHuman cannot reach the runtime at that address                           | Start the runtime, check the port with `curl`, and correct the endpoint                                         |
| The model you want is not in the picker                          | The runtime does not have it                                                 | `ollama pull <model>`, or download and load it in LM Studio, then reopen the picker                         |
| A request fails saying the model is not found                   | The workload names a model the runtime no longer has                        | Pull it again, or route the workload to a model you have                                                    |
| Ollama answers but every model call fails                       | Ollama's model runner is broken                                             | Quit and relaunch Ollama, then retry                                                                        |
| Embeddings fail with a dimension error                          | The embedding model does not produce 1024-dimension vectors                  | Use `bge-m3` |
| Agent turns lose context on Ollama                              | Ollama's default context window is small                                    | Set `local_ai.num_ctx` (for example `8192`)                                                                 |
| Answers feel cloud-quality                                      | The runtime was unreachable and a lightweight hint fell back to remote      | Fix reachability. Use `local_only` privacy mode if fallback is unacceptable                                 |

## Recovery

- **Back to cloud.** Set the routed workloads back to the default route in **Custom routing**, or remove the local provider. No data is lost.
- **Stuck runtime.** Quit the runtime fully and relaunch it. OpenHuman reconnects on the next request.
- **Disk pressure.** Model pulls happen in your runtime, not in OpenHuman. Free space and pull again with the runtime's own command.

## What stays in the cloud anyway

Some workloads use the backend unless you configure something else. Speech-to-text and web search go through the backend proxy. Text-to-speech uses the hosted voice unless you install Piper yourself and set `PIPER_BIN`. See [what stays in the cloud](../features/model-routing/local-ai.md#what-stays-in-the-cloud-by-default).

## See also

- [Local AI](../features/model-routing/local-ai.md): the config reference.
- [Keep sensitive data private](privacy-sensitive-data.md): what stays local and what leaves.
- [Automatic model routing](../features/model-routing/README.md): how tasks get matched to models.
