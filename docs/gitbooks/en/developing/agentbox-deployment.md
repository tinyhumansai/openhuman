---
description: >-
  The removed AgentBox container surface, kept for anyone debugging an older
  build.
icon: box-archive
---

# AgentBox deployment (removed)

OpenHuman once shipped a container surface for the GMI Cloud AgentBox marketplace. It is gone from `openhuman-core`. A regression test, `agentbox_run_and_jobs_paths_are_no_longer_public` in `crates/openhuman-rpc/src/server/auth_tests.rs`, checks that `/run` and `/jobs/{job_id}` no longer exist.

`OPENHUMAN_AGENTBOX_MODE` has no reader left in `crates/openhuman-core/src`. The Dockerfile and `.env.example` still mention it, and those mentions are stale. If you need to run OpenHuman on AgentBox today, ask the team that owns the GMI Cloud deployment where that surface lives now.

The rest of this page describes how the old surface worked. Read it only if you are debugging an older build that still has it.

## Container contract

With `OPENHUMAN_AGENTBOX_MODE=1`, the core HTTP server exposed three routes:

- `POST /run` accepts work and returns `202 { "job_id": "<uuid>" }`. The body is `{ "payload": { "message": "<string>", "thread_id": "<optional string>" } }`.
- `GET /jobs/{job_id}` returns `{ "status": "pending|running|completed|failed", "result": ..., "error": ... }`.
- `GET /health` is a liveness check.

`/run` and `/jobs/*` were unauthenticated at the container boundary. AgentBox's edge handled auth before traffic reached the container.

## Registering in the AgentBox console

The console has a four-step wizard.

1. **Basic info.** Name it `OpenHuman` and add a description.
2. **Infrastructure.** Pick the Docker image source, compute tier and region. Turn on the "GMI MaaS" toggle so the platform injects `GMI_MAAS_BASE_URL` and `GMI_MAAS_API_KEY` at runtime.
3. **Env variables.** Set these:
   - `OPENHUMAN_AGENTBOX_MODE=1`
   - `OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS` (optional, default 600)
   - `GMI_MODELS`, the marketplace-approved model id, for example `deepseek-ai/DeepSeek-V4-Pro`
   - `OPENHUMAN_WORKSPACE`, a writable container path such as `/home/openhuman/.openhuman`
   - `RUST_LOG=info` (use `debug` for the first deploy)
4. **Review and register.** Confirm, then test from the console panel.

The platform API key is shown once, on the registration confirmation screen. Save it to your secrets manager right away. The console cannot show it again.

## Pushing the image

Build from `main` with the existing `Dockerfile` and push to your registry:

```bash
docker build -t <registry>/openhuman-core:<tag> .
docker push <registry>/openhuman-core:<tag>
```

The first deploy takes 10 to 25 minutes to reach `running`. Later deploys are faster.

## Long-running requests

AgentBox treats requests over 2 minutes as long-running, so OpenHuman used polling. The agent ran inside a worker task, capped by `OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS` (10 minutes by default). There was no streaming.

A polling client should:

1. Send `POST /run` and keep the `job_id`.
2. Call `GET /jobs/{job_id}` every 1 to 3 seconds.
3. Stop when `status` is `completed` or `failed`.

Finished jobs were kept for one hour, then garbage-collected. A long pause between polling and reading can return `404`.

## Local smoke test

```bash
OPENHUMAN_AGENTBOX_MODE=1 \
GMI_MAAS_BASE_URL=https://api.gmi-serving.com \
GMI_MAAS_API_KEY=sk-... \
GMI_MODELS=deepseek-ai/DeepSeek-V4-Pro \
./target/debug/openhuman-core serve &

curl -X POST http://127.0.0.1:7788/run \
  -H 'content-type: application/json' \
  -d '{"payload":{"message":"hello"}}'

# Then poll the returned job_id:
curl http://127.0.0.1:7788/jobs/<job_id>
```

## Troubleshooting

- `404 job not found` after a successful submit: the one-hour retention window passed, or the container restarted. The job store was in memory only.
- `status: "failed"` with `error: "agentbox: agent runtime bridge not wired"`: you are on a build from before the runtime bridge landed. Rebuild from a current `main`.
- `status: "failed"` with `error: "job timeout after Ns"`: the agent ran past `OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS`. Raise it on the next deploy.
- `[agentbox::gmi] not registering GMI MaaS provider: missing/blank: GMI_MAAS_API_KEY`: the platform did not inject the key. Check the MaaS toggle in step 2 of the wizard.
- `[agentbox::gmi] current-thread runtime detected, skipping provider registration`: the core booted in a single-threaded tokio runtime. Use the standard `serve` subcommand, which starts a multi-thread runtime.
