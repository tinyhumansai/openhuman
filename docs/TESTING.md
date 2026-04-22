# Testing Guide (Unit + E2E)

## Unit tests (Vitest)

- **Where tests live**: co-locate as `*.test.ts` / `*.test.tsx` under `app/src/**`.
- **Runner/config**: Vitest with `app/test/vitest.config.ts` and shared setup in `app/src/test/setup.ts`.
- **Run**:

```bash
yarn test:unit
yarn test:coverage
```

- **Authoring rules**:
  - Prefer testing behavior over implementation details.
  - Use existing helpers from `app/src/test/` (`test-utils.tsx`, shared mock backend) before adding new harness code.
  - Keep tests deterministic: avoid real network calls, time-sensitive flakes, or hidden global state.

## Shared mock backend (app + Rust tests)

- **Core implementation**: `scripts/mock-api-core.mjs`
- **Standalone server entrypoint**: `scripts/mock-api-server.mjs`
- **E2E wrapper**: `app/test/e2e/mock-server.ts`
- **Vitest unit setup**: `app/src/test/setup.ts` starts the shared mock server by default on `http://127.0.0.1:5005`.

Key admin endpoints:

- `GET /__admin/health`
- `POST /__admin/reset`
- `POST /__admin/behavior`
- `GET /__admin/requests`

Run manually:

```bash
yarn mock:api
curl -s http://127.0.0.1:18473/__admin/health
```

## E2E tests (WDIO — dual platform)

Full guide: [`E2E-TESTING.md`](E2E-TESTING.md).

Two automation backends:
- **Linux (CI default)**: `tauri-driver` (WebDriver, port 4444) — drives the debug binary directly
- **macOS (local dev)**: Appium Mac2 (XCUITest, port 4723) — drives the `.app` bundle

- **Where specs live**: `app/test/e2e/specs/*.spec.ts`
- **Shared harness**:
  - Platform detection: `app/test/e2e/helpers/platform.ts`
  - Element helpers: `app/test/e2e/helpers/element-helpers.ts`
  - Deep link helpers: `app/test/e2e/helpers/deep-link-helpers.ts`
  - App lifecycle: `app/test/e2e/helpers/app-helpers.ts`
  - Mock backend: `app/test/e2e/mock-server.ts`
  - WDIO config: `app/test/wdio.conf.ts` (auto-detects platform)

- **Build + run**:

```bash
# Build app + stage core sidecar (detects macOS vs Linux automatically)
yarn test:e2e:build

# Run one spec
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke

# Run all flow specs
yarn test:e2e:all:flows

# Docker on macOS (run Linux E2E locally)
docker compose -f e2e/docker-compose.yml run --rm e2e
```

- **Authoring rules**:
  - Ensure each spec is runnable in isolation.
  - Use helpers from `element-helpers.ts` — never use raw `XCUIElementType*` selectors in specs.
  - Use `clickNativeButton()`, `hasAppChrome()`, `waitForWebView()`, `clickToggle()` for cross-platform element interaction.
  - Assert both UI outcomes and backend/mock effects when relevant.
  - Add failure diagnostics (request logs, `dumpAccessibilityTree()`) for faster debugging by agents.

## Deterministic core-sidecar reset

By default, `app/scripts/e2e-run-spec.sh` creates and cleans a temp `OPENHUMAN_WORKSPACE`
automatically when the variable is not provided.

If you need a fixed workspace for debugging, provide one explicitly:

```bash
export OPENHUMAN_WORKSPACE="$(mktemp -d)"
yarn test:e2e:build
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
rm -rf "$OPENHUMAN_WORKSPACE"
```

- `OPENHUMAN_WORKSPACE` redirects core config + workspace storage away from `~/.openhuman`.
- Default reset strategy:
  - Rebuild/stage sidecar once per E2E run (`yarn test:e2e:build`).
  - Isolate state per test case with a fresh temp workspace (default behavior in `e2e-run-spec.sh`).

## Rust tests with mock backend

Use the shared mock backend runner so Rust unit/integration tests get deterministic API behavior:

```bash
yarn test:rust
# or targeted
bash scripts/test-rust-with-mock.sh --test json_rpc_e2e
```

Example per-test-case pattern inside a harness script:

```bash
run_case() {
  export OPENHUMAN_WORKSPACE="$(mktemp -d)"
  bash app/scripts/e2e-run-spec.sh "$1" "$2"
  rm -rf "$OPENHUMAN_WORKSPACE"
}
```

## Test authoring checklist

- Add/update unit tests for logic changes before stacking additional features.
- Add/update E2E coverage for user-visible flows and cross-process integration behavior.
- Keep new tests independent, deterministic, and debuggable from logs alone.
- When touching core/sidecar behavior, validate both:
  - `yarn test:unit`
  - targeted E2E spec(s) via `app/scripts/e2e-run-spec.sh`
