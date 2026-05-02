# scripts/debug

Agent-friendly wrappers around the project's test runners. Each command runs
the underlying tool with full output **teed to a log file** under
`target/debug-logs/`, while keeping stdout small (summary + failure blocks).

Use `--verbose` on any runner to also stream the raw output.

## Usage

```sh
# Vitest
pnpm debug unit                                 # full suite
pnpm debug unit src/components/Foo.test.tsx     # one file (positional pattern)
pnpm debug unit -t "renders empty state"        # filter by test name
pnpm debug unit Foo -t "renders empty" --verbose

# WDIO E2E (one spec at a time)
pnpm debug e2e test/e2e/specs/smoke.spec.ts
pnpm debug e2e test/e2e/specs/cron-jobs-flow.spec.ts cron-jobs --verbose

# cargo tests (uses scripts/test-rust-with-mock.sh)
pnpm debug rust
pnpm debug rust json_rpc_e2e

# Inspect saved logs
pnpm debug logs                  # list 50 most recent
pnpm debug logs last             # print most recent (last 400 lines)
pnpm debug logs unit             # most recent matching prefix "unit"
pnpm debug logs last --tail 100
```

Logs land in `target/debug-logs/<kind>-<suffix>-<timestamp>.log`. The directory
is created on demand and is safe to delete — nothing else writes there.

## Why

- **Filtering** — positional pattern + `-t "<name>"` for Vitest, single spec
  for WDIO; agents don't have to grep the whole tree on every change.
- **Bounded output** — the default summary fits in agent context. Full output
  is one `pnpm debug logs last` away.
- **Stable surface** — the runners' flags can churn; this wrapper keeps the
  contract small (positional + a couple of flags) so prompts don't break.

The wrappers don't replace `pnpm test:*` — they call into them.
