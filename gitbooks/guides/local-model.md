---
description: >-
  Run OpenHuman's inference on your own machine with Ollama: detection, model
  selection, a real test, and every way it commonly breaks with the fix.
icon: microchip
---

# Use OpenHuman with a local model

**Goal:** move some or all of OpenHuman's model work onto your own computer, so that data used for those workloads never leaves the machine.

Local AI is **opt-in** and ships **off**. Turning it on doesn't silently reroute everything. You choose which workloads go local.

For the config-level reference (every flag and provider field), see [Local AI (optional)](../features/model-routing/local-ai.md). This guide is the task-oriented version: get it running and confirm it works.

***

## Prerequisites

* [**Ollama**](https://ollama.com) installed. OpenHuman talks to it at its default address `http://localhost:11434`. (LM Studio is also supported at `http://localhost:1234/v1`; see the [reference page](../features/model-routing/local-ai.md#lm-studio-troubleshooting).)
* **8 GB+ RAM** to get real value. Machines with less than 8 GB fall back to cloud summarization by design, because a small local model won't have the headroom.
* Disk for the weights: a small chat model plus an embedding model is a few GB. OpenHuman does not ship weights; Ollama pulls them on demand.

## Privacy implications

* Workloads you route locally (embeddings, summary building, background loops, and, if you choose, chat/reasoning) run **entirely on-device**. Nothing about that work is sent out.
* Anything you leave on the default route still goes through the OpenHuman [model router](../features/model-routing/). Local AI is additive; it doesn't change what you didn't move.
* If the local provider becomes unreachable mid-session, requests **transparently fall back** to the remote provider. That means a crashed Ollama can send that data to the cloud path instead. If strict locality matters, watch the diagnostics (below).

***

## Steps

### 1. Start Ollama

Install and launch Ollama so its local server is running. You can confirm it's up from a terminal:

```bash
curl http://localhost:11434/api/tags
```

A JSON list of models (even an empty one) means the server is reachable. This is the exact probe OpenHuman uses to detect Ollama.

### 2. Turn on Local AI in OpenHuman

Open **Settings → AI & Skills → Local AI**. It's off until you opt in here. Pick the **model tier** that matches your machine's memory (for example the `ram_2_4gb` tier on a typical laptop). OpenHuman selects sensible on-device models for you, for example `gemma3:1b-it-qat` for chat and `bge-m3` for embeddings.

By default the tier keeps **embeddings and memory** on-device while chat and reasoning stay on the cloud route. To move those workloads local too, use **custom routing** to point the chat and reasoning workloads at the local provider, then test that a message stays on the machine.

### 3. Let it pull the models

When a workload needs a model that isn't installed yet, OpenHuman pulls it through Ollama and shows **download progress**. You can also pull manually:

```bash
ollama pull gemma3:1b-it-qat
ollama pull bge-m3
```

### 4. Test that it actually answers

Don't assume; verify. The Local AI settings expose a **test action** that sends a short prompt to the configured local provider and shows you the reply. If you get a coherent response back, the path is live end-to-end (detection → model loaded → inference).

***

## Success checks

Local AI is operational when:

* [ ] **Settings → AI & Skills → Local AI** shows Ollama as reachable and your selected models as available (not "downloading" or "missing").
* [ ] The test action returns a real reply from the local model.
* [ ] The inference status reads **`ready`** (not `degraded`, `downloading`, or `disabled`).
* [ ] For an embeddings-only setup: after the next memory sync, new summaries keep appearing in the **Memory** tab with Ollama running. That confirms embeddings are being produced locally.

{% hint style="info" %}
**Where to look under the hood.** OpenHuman surfaces a live diagnostics view for the local runtime (Ollama reachable? runner OK? which models are installed vs expected? what issues?). If a check fails, the diagnostics name the specific problem. Start there before changing config.
{% endhint %}

## Common failures

These are the actual failure states the runtime reports, and what each one means:

| What you see | Meaning | Fix |
| ------------ | ------- | --- |
| **"Ollama server is not running or not reachable"** (status `degraded`) | The app can't reach Ollama at its base URL | Start Ollama; confirm `curl http://localhost:11434/api/tags` works; if you use a non-default port, set the base URL in Local AI settings |
| **"…reachable but cannot execute models. Restart the external runtime and retry."** | Ollama is answering but its model runner is broken (a fork/exec failure) | Quit and relaunch Ollama itself, then retry |
| **"Chat model '…' is not installed"** | The configured model isn't pulled yet | Let OpenHuman pull it, or run `ollama pull <model>` |
| **"…not reachable after fresh install. Start `ollama serve` manually and retry."** | Ollama was just installed but the server isn't up | Run `ollama serve` (or launch the Ollama app) and retry |
| Embedding model rejected for **context window too small** | The chosen embedding model can't hold enough tokens for the memory layer | Choose a larger-context embedding model such as **`bge-m3`** |
| Status stuck at **`downloading`**, then a retry message | A model pull stream was interrupted | It retries automatically; if it keeps failing, check disk space and network, then pull manually |
| It "works" but answers feel cloud-quality | The local provider was unreachable and OpenHuman **fell back to remote** | Fix reachability above; strict-local users should confirm status is `ready` before relying on it |

## Recovery

* **Back to cloud in one step:** turn Local AI back off in **Settings → AI & Skills → Local AI**. Workloads return to the default route immediately; no data is lost.
* **Free up a stuck runtime:** quit Ollama fully and relaunch it, then re-open the Local AI settings so OpenHuman re-probes.
* **Disk pressure:** interrupted pulls are usually low disk. Clear space and let the pull resume, or `ollama pull` the model by hand.

***

## Notes on what stays cloud anyway

Even with "everything local", some workloads are cloud by default unless you explicitly route them: speech-to-text, text-to-speech, and web search go through the backend proxy. See [what stays in the cloud](../features/model-routing/local-ai.md#what-stays-in-the-cloud-by-default).

## See also

* [Local AI (optional)](../features/model-routing/local-ai.md): the full config reference.
* [Keep sensitive data private](privacy-sensitive-data.md): the plain-language version of local vs external.
* [Automatic Model Routing](../features/model-routing/): how tasks get matched to models.
