# Harness Cache Audit Cases

Reusable live-audit scenarios for `pnpm debug harness-cache-audit`.

These cases require an active OpenHuman backend session in the selected user
workspace. They spend live model credentials. Use a stable `--thread-id` per
run so the hosted backend can group inference/cache logs like production web
chat.

## Workspace

By default, `pnpm debug harness-cache-audit` reads
`~/.openhuman/active_user.toml` and uses that user's workspace. To pin a
workspace explicitly, use the active per-user workspace, not the top-level
`~/.openhuman` directory:

```bash
ACTIVE_USER_ID="$(sed -n 's/^user_id = "\(.*\)"$/\1/p' "$HOME/.openhuman/active_user.toml")"
WORKSPACE="$HOME/.openhuman/users/$ACTIVE_USER_ID/workspace"
```

Or inspect the active user directly:

```bash
cat "$HOME/.openhuman/active_user.toml"
```

## Case 1: Baseline Delegation

Purpose: repeated normal orchestrator turns with one subagent delegation.

```bash
pnpm debug harness-cache-audit \
  --spawn-core \
  --workspace "$WORKSPACE" \
  --turns 4 \
  --min-hit-rate 20 \
  --max-turns-without-cache 1 \
  --thread-id "harness-cache-audit-baseline-$(date +%Y%m%d-%H%M%S)"
```

Observed on 2026-06-22:

- Without explicit `thread_id`: `68.56%` cache hit, `$0.011535`.
- With explicit `thread_id`: `85.49%` cache hit, `$0.040142`.

Note: the generic prompt may route to heavier agents such as `researcher`.

## Case 2: Complex Internal Tool Loop

Purpose: repeated turns that exercise planning plus worker delegation while
trying to avoid external services.

```bash
pnpm debug harness-cache-audit \
  --spawn-core \
  --workspace "$WORKSPACE" \
  --turns 6 \
  --min-hit-rate 50 \
  --max-turns-without-cache 1 \
  --thread-id "harness-cache-audit-complex-$(date +%Y%m%d-%H%M%S)" \
  --prompt 'Complex harness cache audit. Do not use research or external service tools. Exercise multiple internal tool steps only: first call plan with a tiny 2-step audit checklist, then delegate exactly one compact subagent/tool-worker style task if a delegation tool is available. The delegated task should only return one sentence about stable repeated prompts and must not browse, read email, inspect files, or call external APIs. Finish with one concise sentence summarizing the two internal steps.'
```

Observed on 2026-06-22:

- Aggregate: `83.57%` cache hit, `$0.072063`.
- `orchestrator`: `96.54%`.
- `planner`: `77.57%`.
- `tools_agent`: `85.63%`.
- `integrations_agent`: `88.34%`.

Note: one `integrations_agent` session still appeared, so this is not fully
deterministic.

## Case 3: Read-Only Coding Agent

Purpose: exercise code-repo delegation without file edits or test execution.

```bash
pnpm debug harness-cache-audit \
  --spawn-core \
  --workspace "$WORKSPACE" \
  --turns 4 \
  --min-hit-rate 50 \
  --max-turns-without-cache 1 \
  --thread-id "harness-cache-audit-coding-$(date +%Y%m%d-%H%M%S)" \
  --prompt 'Coding-agent cache audit. Delegate exactly one read-only code-repo task to the coding/code executor agent if available. The delegated task: inspect the OpenHuman repo only enough to identify where the harness cache audit script is wired into pnpm debug; do not edit files, do not run tests, do not browse, do not call external services. After the coding agent returns, reply with one concise sentence naming the script path and dispatcher path.'
```

Expected behavior:

- At least one coding/code-executor subagent transcript appears.
- No repository files are modified.
- Cache should be judged by aggregate hit rate and by the coding-agent row.

Observed on 2026-06-22:

- Aggregate: `93.00%` cache hit, `$0.046437`.
- `orchestrator`: `96.41%`.
- `code_executor`: `92.72%`.
- Turn 1 was slow (`240406ms`), then turns 2-4 completed much faster.

## Interpreting Results

- A cold first turn is expected. With four turns, one cold root/subagent call
  can make the aggregate look lower than steady state.
- Compare per-agent rows, not only aggregate. A high root-agent cache rate with
  lower planner/coding-agent rates usually means tool-loop history or subagent
  prompts are the variable section.
- For prompt stability, compare the system-message hashes in the new transcript
  files. Stable system hashes with low cache usually point to volatile user
  context, tool results, or backend grouping.
