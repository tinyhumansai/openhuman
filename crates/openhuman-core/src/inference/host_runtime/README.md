# host_runtime

OpenHuman-owned bindings around `tinyinference-local`.

The reusable runtime implementation lives in the `tinyinference-local` crate:
endpoint resolution and probing for Ollama / LM Studio / MLX / OMLX /
OpenAI-compatible runtimes, status, prompts, vision, and embeddings. The user
installs and runs that runtime and pulls its models; neither crate downloads,
installs, or spawns anything. This directory contains only OpenHuman policy
and integration seams:

- [`core.rs`](./core.rs): process-wide service ownership.
- [`ops.rs`](./ops.rs) and `ops/`: RPC operations, prompt access checks, agent dispatch,
  scheduler-gate acquisition, and temporary-file handling.
- [`schemas.rs`](./schemas.rs): OpenHuman controller registration and wire compatibility.
- [`service/speech.rs`](./service/speech.rs): OpenHuman STT credential routing and Piper output.
- `service/transcription.rs`: channel-facing transcription adapter shape.

Code here must call TinyInference directly. Do not copy local-runtime behavior
back into this host module.

## Further reading

- [Parent module README](../README.md)
- [Automatic model routing](../../../../../docs/gitbooks/en/features/model-routing/README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
