---
description: Artifact-capture layer that makes E2E tests debuggable. Logs, traces, screenshots.
icon: eye
---

# Agent Observability

This doc describes the artifact-capture layer that makes the desktop app
inspectable by coding agents (Codex, Claude Code, Cursor) through the
existing WDIO / tauri-driver E2E harness (see [E2E Testing](e2e-testing.md)).

It is intentionally narrow: one canonical onboarding + privacy flow with
on-disk screenshots, page-source dumps, and mock backend request logs.

## TL;DR

```bash
bash app/scripts/e2e-agent-review.sh
```

Artifacts land under:

```text
app/test/e2e/artifacts/<ISO-timestamp>-agent-review/
  01-welcome.png
  01-welcome.source.xml
  02-post-welcome.png
  02-post-welcome.source.xml
  03-post-onboarding.png
  03-post-onboarding.source.xml
  04-privacy-panel.png
  04-privacy-panel.source.xml
  mock-requests-after-welcome.json
  mock-requests-after-onboarding.json
  mock-requests-after-privacy.json
  failure-<test>.png              # only on failure
  failure-<test>.source.xml       # only on failure
  meta.json                       # run metadata + checkpoint index
```

The script prints the resolved artifact directory at the end.

## Pieces

| Piece            | Path                                                                                                       | Role                                                                          |
| ---------------- | ---------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| Helper           | `app/test/e2e/helpers/artifacts.ts`                                                                        | Run dir, `captureCheckpoint`, `captureFailureArtifacts`, `saveMockRequestLog` |
| WDIO hook        | `app/test/wdio.conf.ts` (`afterTest`)                                                                      | Always dumps screenshot + source on any failing test                          |
| Canonical spec   | `app/test/e2e/specs/agent-review.spec.ts`                                                                  | Welcome → onboarding → privacy panel with named checkpoints                   |
| Wrapper script   | `app/scripts/e2e-agent-review.sh`                                                                          | Build + run + print artifact dir                                              |
| Stable selectors | `data-testid` on `OnboardingNextButton`, `Onboarding` overlay + skip button, `WelcomeStep`, `PrivacyPanel` | Agent-reliable navigation anchors                                             |

## Environment overrides

| Variable             | Effect                                                                                      |
| -------------------- | ------------------------------------------------------------------------------------------- |
| `E2E_ARTIFACT_DIR`   | Force a specific run dir (skips auto-timestamped name)                                      |
| `E2E_ARTIFACT_ROOT`  | Parent dir for auto-generated run dirs (default: `app/test/e2e/artifacts`)                  |
| `E2E_ARTIFACT_LABEL` | Label used in the auto-generated run dir name (default: `run`; wrapper sets `agent-review`) |

## Using the helper from new specs

```ts
import { captureCheckpoint, saveMockRequestLog } from "../helpers/artifacts";
import { getRequestLog } from "../mock-server";

await captureCheckpoint("after-connect-click");
saveMockRequestLog("after-connect-click", getRequestLog());
```

`captureCheckpoint` numbers captures so the run dir reads chronologically.
`captureFailureArtifacts` is wired into `wdio.conf.ts` and fires
automatically on any failing test, specs should not call it directly.

## Inference on the wire: the capture proxy

Logs tell you a turn was slow; they do not tell you what the harness sent or
which endpoint answered. `scripts/debug/capture-first-inference.mjs`
(`pnpm debug capture`) is a loopback proxy between the core and its inference
backend that records both sides:

```bash
CAPTURE_ALL=1 pnpm debug capture                # listens on 127.0.0.1:18765
# then, in another shell, point a core at it:
BACKEND_URL=http://127.0.0.1:18765 ./target/debug/openhuman-core run --port 7799
# or set api_url = "http://127.0.0.1:18765" in the user's config.toml
```

Every inference request body is written, numbered, under
`target/debug-logs/inference-sequence/` (the exact system prompt, tool
schemas and `prompt_cache_key` the harness assembled), and every response
yields one summary line and one JSONL record in
`target/debug-logs/inference-capture.jsonl`:

```text
[capture] #000 200 model=z-ai/glm-5.3-flash msgs=2 tools=19 served_by=StreamLake ttfb=7.38s total=8.43s prompt=12344 cached=12288 cache_key=tap-25675927a3f2160d
```

`CAPTURE_UPSTREAM=https://openrouter.ai` captures a direct BYOK OpenRouter
route instead of the hosted backend. Non-2xx response bodies are saved next to
the request dumps so an HTML 503 from an ingress is not lost behind a generic
"model error".

What to look for across the turns of one thread: `cache_key` must stay
identical (it is the harness's stable-prefix fingerprint and OpenRouter's
sticky-routing key), `served_by` should not change, and `cached` should
approach `prompt` from the second call on. Each of those drifting has been a
real bug (openhuman#6434). The proxy binds loopback only and refuses a
plaintext non-loopback upstream unless overridden, because it forwards the
bearer verbatim; `--help` lists every `CAPTURE_*` knob.

## Production agent traces are a separate mechanism

Everything above is about E2E artifacts on disk. For a real agent run, the
core can export trace spans to Langfuse instead: when
`observability.share_usage_data` is enabled by an explicit opt-in, a completed run's spans
go to the OpenHuman backend's Langfuse ingestion proxy over the same session
bearer every other backend call uses, and the backend forwards them to
Langfuse with the project keys injected server-side. Clients never hold
Langfuse credentials directly. Prompt and reply text ride along only while
`observability.agent_tracing.capture_content` is also enabled; turning it off keeps
metadata (names, timings, token and cost figures) but drops content. See
`crates/openhuman-core/src/agent/progress_tracing/langfuse.rs` and
`crates/openhuman-core/src/config/schema/observability.rs`.

Both settings default to `false`. To share metadata, set
`share_usage_data = true` under `[observability]` in the user's `config.toml`.
To also capture prompts, replies, system prompts and tool inputs/results, set
`capture_content = true` under `[observability.agent_tracing]`. The local
exporter has its own `enabled` switch and stays independent of backend sharing.
Stored explicit choices are preserved when loading an existing configuration.

For declarative installs, `OPENHUMAN_SHARE_USAGE_DATA` controls sharing and
`OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT` controls content capture. Both accept
`true`/`false`, `on`/`off`, `yes`/`no` and `1`/`0`, overriding the stored value.

## What is intentionally out of scope

- Visual baselines / image diffs across every component state.
- Screenshot capture on every click (too noisy).
- Live integrations (Gmail, Notion, Telegram); mock server only.
- New test framework / reporter.

Widen to more flows only after this loop proves out.
