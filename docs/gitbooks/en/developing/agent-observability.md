---
description: >-
  How to see what the app and the agent did: E2E artifacts, the inference
  capture proxy, and Langfuse traces.
icon: eye
---

# Agent observability

This page covers three ways to see what OpenHuman did. E2E artifacts let a coding agent (Codex, Claude Code, Cursor) inspect the desktop app through the WDIO harness (see [E2E testing](e2e-testing.md)). The capture proxy shows what the core sent to inference. Langfuse traces cover real agent runs.

The E2E layer is narrow on purpose. It covers one onboarding and privacy flow and saves screenshots, page-source dumps and mock backend request logs to disk.

## Quick start

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
| Canonical spec   | `app/test/e2e/specs/agent-review.spec.ts`                                                                  | Welcome, onboarding, then the privacy panel, with named checkpoints                   |
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
automatically on any failing test. Specs should not call it directly.

## Inference on the wire: the capture proxy

Logs tell you a turn was slow. They do not tell you what the harness sent or
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

Across the turns of one thread, check three things. `cache_key` must stay
identical, because it is the harness's stable-prefix fingerprint and
OpenRouter's sticky-routing key. `served_by` should not change. `cached`
should approach `prompt` from the second call on. A drift in any of them has
been a real bug.

The proxy binds loopback only. It refuses a plaintext non-loopback upstream
unless you override that, because it forwards the bearer token verbatim.
`--help` lists every `CAPTURE_*` option.

## Production agent traces

Everything above is about local artifacts. For a real agent run, the
core can export trace spans to Langfuse instead: when
`observability.share_usage_data` is on (the default), a completed run's spans
go as OTLP/HTTP JSON to the OpenHuman backend's Langfuse proxy,
`/telemetry/langfuse/otel/v1/traces`, over the same session bearer every other
backend call uses. The backend stamps the signed-in user on every span, injects
the project keys server-side and forwards to Langfuse v4's
`/api/public/otel/v1/traces` with `x-langfuse-ingestion-version: 4`. Clients
never hold Langfuse credentials directly. Flow runs export the same way
(`crates/openhuman-core/src/flows/tinyflows/langfuse_export.rs`).

Langfuse v4 has no trace record of its own and filters observations directly,
so the exporters copy the trace name, session, tags, release, environment and
trace metadata onto every span, and put the run's overall input and output on
the root observation. Each finished span is exported once; v4 duplicates a
span id it receives twice. Langfuse Cloud removes the legacy
`/api/public/ingestion` trace path on 2026-11-16, and nothing in the core sends
traces there. Prompt and reply text ride along only while
`observability.agent_tracing.capture_content` is also on; turning it off keeps
metadata (names, timings, token and cost figures) but drops content. See
`crates/openhuman-core/src/agent/progress_tracing/otlp.rs` and
`crates/openhuman-core/src/config/schema/observability.rs`.

## Out of scope for the E2E layer

- Visual baselines and image diffs across every component state.
- Screenshot capture on every click (too noisy).
- Live integrations (Gmail, Notion, Telegram); mock server only.
- A new test framework or reporter.

Add more flows only after this loop proves itself.
