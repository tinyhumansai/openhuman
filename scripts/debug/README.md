# scripts/debug

Agent-friendly wrappers around the project's test runners. Each command runs
the underlying tool with full output **teed to a log file** under
`target/debug-logs/`, while keeping stdout small (summary + failure blocks).

Use `--verbose` on any runner to also stream the raw output.

## Usage

The runners are [`unit.sh`](./unit.sh), [`e2e.sh`](./e2e.sh), [`rust.sh`](./rust.sh) and [`logs.sh`](./logs.sh), dispatched by [`cli.sh`](./cli.sh).

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
is created on demand and is safe to delete, nothing else writes there.

## Browser scenarios (`pnpm debug web`)

```sh
pnpm debug web                                   # stack + signed-in page, up until Ctrl-C
pnpm debug web --script scripts/debug/web-scripts/stop-mid-turn.mjs
pnpm debug web --script my-scenario.mjs --keep   # leave the stack up afterwards
```

[`web-ui.mjs`](./web-ui.mjs) starts the shared mock backend ([`scripts/mock-api-server.mjs`](../mock-api-server.mjs)) in-process, a fresh
[`openhuman-core serve`](../../crates/openhuman-cli/README.md) (built if missing) on a scratch workspace, and Vite with
`OPENHUMAN_VITE_NO_WATCH=1` (no file watcher or HMR, so it survives a host whose
inotify watch limit is used up). It opens the SPA in Playwright Chromium through
`/__dev-connect` and signs in through the real GitHub button: the mock answers
`/auth/<provider>/login?redirectUri=<vite>/__dev-auth` like the backend.

A scenario is an ES module whose default export receives
`{ page, context, browser, mock, rpc, urls, logDir, screenshot, log }`.
`mock.set(key, value)` sets a mock behavior (`llmStreamScript` scripts the LLM
stream, see [`scripts/mock-api/routes/llm.mjs`](../mock-api/routes/llm.mjs)), `rpc(method, params)` calls the
core, and `screenshot(name)` saves into the run's artifact directory
(`target/debug-logs/web-<ts>/`: `core.log`, `vite.log`, `browser.log`,
screenshots, the scratch workspace). A thrown error saves `failure.png`.

## Why

- **Filtering**: positional pattern + `-t "<name>"` for Vitest, single spec
  for WDIO; agents don't have to grep the whole tree on every change.
- **Bounded output**: the default summary fits in agent context. Full output
  is one `pnpm debug logs last` away.
- **Stable surface**: the runners' flags can churn; this wrapper keeps the
  contract small (positional + a couple of flags) so prompts don't break.

The wrappers don't replace the project test runners, they invoke the
underlying tools/scripts with log capture.

## Related tools

- [`capture-first-inference.mjs`](./capture-first-inference.mjs) and [`prompt-breakdown.mjs`](./prompt-breakdown.mjs), the inference capture proxy and prompt pricer described in [`AGENTS.md`](../../AGENTS.md).
- [`web-scripts/`](./web-scripts) holds example scenarios for `pnpm debug web`.
- [Testing strategy](../../docs/gitbooks/en/developing/testing-strategy.md), [E2E testing](../../docs/gitbooks/en/developing/e2e-testing.md) and [Agent observability](../../docs/gitbooks/en/developing/agent-observability.md).
- [`scripts/README.md`](../README.md) and [`tests/README.md`](../../tests/README.md).
