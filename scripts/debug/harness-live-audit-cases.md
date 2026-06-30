# Harness Live Audit Cases

These checks exercise the real harness through JSON-RPC with live model credentials. They are debug audits, not CI tests. They intentionally avoid printing prompt bodies, response bodies, tokens, or transcript contents.

Use an isolated spawned core when validating managed OpenHuman backend behavior without relying on a running desktop core:

```bash
node scripts/debug/harness-subagent-rpc-audit.mjs --spawn-core --isolated-workspace --model agentic-v1 --scenario all
```

`--isolated-workspace` defaults to `--provider-mode openhuman-backend`. It writes a temporary backend config, seeds the temp auth store from `JWT_TOKEN`, starts `openhuman-core`, and removes the temp workspace by default. Only pass `--keep-workspace` when you need to inspect artifacts locally.

The optional `--provider-mode direct-openai` control path writes a temporary direct-provider config using `OPENAI_API_KEY` or `OPENAI_KEY`.

## Cases

- `async-steer`: starts a durable async subagent with `spawn_subagent`, waits until the live registry shows it running, then sends `openhuman.subagent_steer` mid-run and verifies the steer was accepted.
- `parallel-research-code`: asks the orchestrator to call `spawn_parallel_agents` with two workers. One researches `https://example.com`; the other writes a small Python code snippet. The audit checks that the parent turn completed and at least two subagent transcripts changed.
- `reuse-parent-comm`: runs two parent prompts. The first prompt spawns two blocking durable subagents that return parent-facing status updates. The second prompt uses the same durable task keys and verifies that the same subagent sessions are reused. Each turn prints transcript usage deltas: input tokens, cached input tokens, output tokens, and charged USD.
- `all`: runs every case against the same spawned core.

Useful focused commands:

```bash
node scripts/debug/harness-subagent-rpc-audit.mjs --spawn-core --isolated-workspace --model agentic-v1 --scenario async-steer
node scripts/debug/harness-subagent-rpc-audit.mjs --spawn-core --isolated-workspace --model agentic-v1 --scenario parallel-research-code
node scripts/debug/harness-subagent-rpc-audit.mjs --spawn-core --isolated-workspace --model agentic-v1 --scenario reuse-parent-comm
node scripts/debug/harness-subagent-rpc-audit.mjs --spawn-core --isolated-workspace --provider-mode direct-openai --model gpt-4.1-mini --scenario reuse-parent-comm
```
