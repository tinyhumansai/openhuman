# E2E full-suite hardening — session handoff notes

Branch: `ci/full-e2e-run-2026-05-23` on `senamakel/openhuman` (fork).
Date: 2026-05-23 → 2026-05-24.

This is the snapshot of what's been done, what's known, and what to pick
up next. Pair this with `gitbooks/developing/e2e-testing.md` (the
existing E2E doc) — this file documents the multi-session push to get
the **full** suite (all ~87 specs) reliably green on Linux + reproducible
locally in Docker.

---

## TL;DR — Current state

| Surface | Status |
|---|---|
| Full-suite CI on Linux | **72 / 87 passing** (15 failing), 6 parallel shards, ~25 min wall |
| Two shards 100% green | **commerce (11/0)** + **webhooks (9/0)** |
| Local Docker runs same 6-shard layout | `bash app/scripts/e2e-run-shards.sh` |
| Local + CI agree on shard pass/fail | Yes (per-spec counts differ inside failing shards; see *CEF instability* below) |
| macOS / Windows full-suite | Not yet validated this session — sharded jobs exist in workflow but only Linux was iterated on |

Branch SHA at handoff: `2bad1f046` (`revert: drop Escape press in openConnectorModal`).

---

## What changed (commits, top → bottom is most recent)

1. `revert: drop Escape press in openConnectorModal` — regressed 7
   connectors, reverted.
2. `fix(e2e): only press Escape in openConnectorModal when a modal
   backdrop is actually present` — superseded by revert.
3. `perf(e2e): split integrations into providers + webhooks shards` —
   6-shard matrix.
4. `test(e2e): finish composio_sync URL-drop + close stale modal in
   openConnectorModal`
5. `test(e2e): local shard runner + fix telegram-flow reference +
   connector log refs` — adds `app/scripts/e2e-run-shards.sh`.
6. `test(e2e): drop URL-based assertions for composio_sync/_execute`
7. `perf(e2e): isolate connector smoke specs into their own shard`
8. `test(e2e): orchestrator coverage + state-bleed fixes` — adds 23
   missing specs to `e2e-run-all-flows.sh`.
9. `test(e2e): align Linux specs with PR #2550 settings restructure`
10. `test(e2e): point auth-access-control logout test to /settings/account`
11. `test(e2e): drop assertions for surfaces removed in PR #2550`
12. `test(e2e): switch auth bypass from deep-link to loopback OAuth
    path` — important fidelity change, see below.
13. `perf(e2e): hoist Linux full-suite build into a single job,
    fan-out tests` — build-once → matrix shards.
14. `fix(e2e): gate build-skip on BOTH binary + CEF cache hits`
15. `fix(e2e): align CEF cache paths with actual download location`
16. `fix(e2e): set CEF_PATH when binary cache skips build`
17. `perf(e2e): cache built binary across shard runs`
18. `fix(e2e): install x86_64-apple-darwin target for Mac shard build`
19. `perf(e2e): shard full suite across 4 parallel jobs per OS`
20. `perf(e2e): run full suite in one shared session (no per-spec
    relaunch)` — first big perf jump.
21. `fix(e2e): repair stale assertions in linux-cef-deb-runtime spec`
22. `fix(e2e): put cargo-tauri install root on PATH for macOS/Windows
    CI` — Mac/Win build fix.

---

## Architecture as it stands today

### CI workflow

`.github/workflows/e2e-reusable.yml` defines three Linux job tiers:

```text
e2e-linux         (smoke + mega-flow only, runs when inputs.full == false)
rust-e2e-linux    (Rust-side `tests/*_e2e.rs` against mock backend)
build-linux-full  (one job: cargo tauri build + tar artifact, uploads)
e2e-linux-full    (matrix of 6 shards, each `needs: build-linux-full`)
```

The build job tars `app/src-tauri/target/debug/OpenHuman`, `app/dist`,
and `$HOME/Library/Caches/tauri-cef/` into a single `tar -czf`
artifact (~600 MB). Each shard downloads + extracts to the canonical
paths and skips the build step entirely. CEF/binary caches still live
on the build job to keep cold builds fast.

### Shard layout

```text
foundation   = auth,navigation,system          (~21 specs)
chat         = chat,skills,journeys            (~19 specs)
providers    = providers,notifications         (~14 specs)
webhooks     = webhooks                        (~9 specs)
connectors   = connectors                      (~16 specs)
commerce     = payments,settings               (~11 specs)
```

The 6th shard (`webhooks` carved out of the original `integrations`)
was added because anything over ~18-20 specs in one shared CEF
session goes unstable on Linux. The `connectors` suite is its own
category in `e2e-run-all-flows.sh` for the same reason.

### Local equivalent

`app/scripts/e2e-run-shards.sh` is the local mirror of the CI matrix.
Runs each shard as a fresh `e2e-run-all-flows.sh --suite=…`
invocation, so each shard gets a fresh CEF process.

```bash
docker compose -f e2e/docker-compose.yml run --rm e2e \
  bash -lc "bash app/scripts/e2e-run-shards.sh"
# or one shard:
docker compose -f e2e/docker-compose.yml run --rm e2e \
  bash -lc "bash app/scripts/e2e-run-shards.sh foundation"
```

### Orchestrator (`app/scripts/e2e-run-all-flows.sh`)

Collects all spec paths into one list (`_spec_paths[@]`) and calls
`e2e-run-session.sh` ONCE with the full list, instead of per-spec.
That restored the design intent in `wdio.conf.ts` ("WDIO creates ONE
session per worker ... all specs run sequentially in the same
session"). Per-spec relaunch was costing ~15-30s of CEF cold-start
× 65 specs = 15+ min of pure overhead before this change.

`--suite=` accepts a comma-separated list now (`--suite=auth,navigation,system`).

slack-flow is explicitly **commented out** in the orchestrator — it
crashed the CEF session mid-spec consistently. Investigate before
re-enabling.

---

## Loopback auth bypass (production fidelity)

Per PR #2550 the real OAuth login flow uses an RFC 8252 loopback
listener (`http://127.0.0.1:53824/auth?state=…`) instead of the
`openhuman://` deep-link. E2E auth bypass was still firing
`openhuman://auth?token=…` directly through `window.__simulateDeepLink`,
which is now the legacy fallback path.

Switched in `app/test/e2e/helpers/loopback-auth-helpers.ts` +
`reset-app.ts`:

1. WebView calls production `startLoopbackOauthListener()` (exposed
   on `window.__startLoopbackOauthListener` when the E2E build flag
   `VITE_OPENHUMAN_E2E_RESTART_APP_AS_RELOAD === 'true'` is set in
   `app/src/utils/loopbackOauthListener.ts`).
2. WebView wires `awaitCallback()` → `__simulateDeepLink` so the
   callback URL is rewritten `http://127.0.0.1:…/auth?…` →
   `openhuman://auth?…` and dispatched through the existing deep-link
   handler — mirroring exactly what `OAuthProviderButton.tsx` does in
   production.
3. Node-side `fetch()` hits the loopback URL with the bypass JWT +
   state nonce appended; the Rust listener accepts, validates state,
   emits `loopback-oauth-callback`.

This means every spec's `resetApp()` now exercises the same Rust HTTP
server + state nonce check + Tauri event emit that ships to users.

`triggerAuthDeepLink` / `triggerAuthDeepLinkBypass` are still kept for
oauth-success deep links (e.g. mega-flow's connector callbacks) that
the loopback path doesn't cover.

---

## Known failures and root causes

### Foundation (2 failing on CI, more on local)

| Spec | Cause | Difficulty |
|---|---|---|
| `onboarding-modes` (Phase B) | After Phase A reaches `/home`, `resetOnboardingFlagAndReload` resets `onboarding_completed=false` + reloads, but the Custom-card click in Phase B doesn't register (data-testid found but click is intercepted or stale). Needs DOM inspection of the wizard re-mount. | medium |
| `runtime-picker-login` | `resetApp(skipAuth: true)` should land on Welcome screen, but the renderer re-hydrates from a persisted snapshot and lands on /home instead. `resetApp` already polls for the Welcome heading + re-replaces `#/` for up to 10s; insufficient. Likely needs to wait for `snapshot.sessionToken` to be cleared (via `fetchCoreAppSnapshot`) before considering the reset done. | medium |

### Chat (3 failing)

| Spec | Cause |
|---|---|
| `chat-harness-subagent` | Agent orchestrator doesn't produce expected canary string. Real product/agent behavior — not a test bug. |
| `chat-harness-wallet-flow` | Crypto agent doesn't produce wallet quote. Real product behavior. |
| `chat-multi-tool-round` | T2.1 (`agent calls tool 1 (file_read); timeline shows it`) — `expect.toBe(true)` fails. Could be timing or real product change. |

`chat-conversation-history` H1.4 was fixed earlier — root cause was
`getSelectedThreadId()` returning the prior-spec's stale thread id
before the New-thread click had time to update Redux. Fix: capture
prior id, wait for `selectedThreadId !== priorThreadId`.

### Providers (3 failing)

`conversations-web-channel-flow`, `telegram-channel-flow`,
`whatsapp-flow` — likely the same shared-CEF-session instability
hitting late-shard specs. Worth re-checking after any further shard
reduction.

### Connectors (7 failing — all hit "expired auth" subtest)

The other 9 connector tests in each spec pass. The one consistent
failure is "expired auth shows Reconnect button and does not log user
out" — `openConnectorModal()`'s card click is intercepted because the
previous test left a modal backdrop up. Attempted fix (`Escape`
before click) regressed other tests; reverted. Real fix probably:
guarantee modal close in `afterEach` rather than working around it in
the open helper.

---

## CEF shared-session instability — the recurring theme

Empirically, the shared-CEF debug build becomes unreliable past
~18-20 specs in a single session. Symptoms vary:

- `__simulateDeepLink ready? false (poll N)` after the listener was
  previously fine
- `A sessionId is required for this command`
- ECONNREFUSED to Appium :4723 mid-suite
- Mysterious `esbuild` platform-mismatch errors during WDIO's TS
  transform of a spec file (red herring — sub-symptom of WDIO
  failing to bring the spec into scope after a session loss)

Mitigations applied:

- Shard so no shard runs more than ~16 specs (and the busiest two —
  foundation 21, chat 19 — are at the edge of what works).
- Run each shard as a fresh `e2e-run-session.sh` invocation locally
  (mirrors CI matrix isolation).

What might fix it for real (not attempted this session):

- Bump WDIO `specFileRetries` so a session loss restarts the failing
  spec.
- Periodic `openhuman.test_reset` + reload at a fixed cadence (every
  10 specs?) to clear in-process leaks.
- Build the test binary in `--release` to reduce per-process memory
  pressure (debug CEF + tauri builds are heavy).

---

## Stale assertions / PR #2550 drift — handled

PR #2550 ("fix(oauth): make loopback redirect actually work, plus
settings cleanup") moved a bunch of settings surfaces. Fixed tests:

- Logout/Clear App Data lives at `/settings/account` (was `/settings`).
  Updated `logoutViaSettings` helper + `settings-data-management` +
  `auth-access-control`.
- `/settings/connections` route deleted (ConnectionsPanel removed).
  `settings-account-preferences` dropped the post-recovery-phrase
  wallet status assertion; `navigation-settings-panels` N2.2
  `.skip`-ed with PR pointer.
- "Notification Routing" no longer a top-level Developer Options
  entry — moved into a tab on `/settings/notifications#routing`.
  `settings-advanced-config` navigates to `/settings/notifications`
  and clicks the Routing tab.
- `screen-intelligence` dropped the "Permissions" assertion on Linux
  (the section is gated behind `status.platform_supported`, true
  only on macOS).

---

## Composio connector specs — the `composio_sync` URL gotcha

The 15 connector smoke specs each had:

```ts
clearRequestLog();
await callOpenhumanRpc('openhuman.composio_sync', { toolkit: TOOLKIT_SLUG });
const syncReq = getRequestLog().find(
  r => r.method === 'POST' && r.url.includes('/composio/sync')
);
expect(syncReq).toBeDefined();  // always failed
```

`/composio/sync` does not exist in the mock router and the
`composio_sync` RPC short-circuits with "no native provider
registered" for any connector without a Rust-side provider, so no
HTTP request is ever logged. The probe-style assertion never had a
chance.

The real intent (per the spec's `PASS:` log message: "sync does not
nuke session") is covered by `assertSessionNotNuked()` on the next
line. Dropped the URL check across all 15 specs.

Same fix for `composio_execute` / `/composio/execute`.

---

## Local docker quirks

- The docker-compose has named volumes per-platform for `node_modules`
  and `.pnpm-store` (the bind-mounted host `node_modules` would
  clobber Linux binaries with macOS ones).
- `e2e-bootstrap` (in `e2e/docker-entrypoint.sh`) installs Appium 3 +
  chromium driver on first entry and caches into the npm volume.
- Docker Desktop dies if the host has < ~1 GB free. Watch for
  `ENOSPC` while running long suites — output files grow fast.
- `tee /tmp/local-shards.log` is the recommended way to capture the
  sharded run output; the bg-task output file gets cleaned up
  aggressively by the harness.

---

## Suggested next-session priorities

1. **Foundation Phase B onboarding + runtime-picker Welcome.**
   `resetApp(skipAuth)` is close — it polls for Welcome heading,
   force-replaces hash to `#/`, gives 10s. Needs to additionally
   poll `fetchCoreAppSnapshot()` until `sessionToken` is gone before
   returning. Probably 1-2 hours of careful work.

2. **Connector expired-auth `openConnectorModal`.** Add an
   `afterEach` that explicitly closes any open modal (`Escape` +
   wait for backdrop to disappear) rather than the failed
   "Escape-before-open" approach. ~30 min.

3. **CEF session retry.** Add WDIO `specFileRetries: 1` so a
   session-loss in shard N+1 retries spec N+1 in a fresh slot
   instead of cascading the rest of the shard. This should recover
   maybe 5-8 of the late-shard failures.

4. **Validate macOS + Windows full-suite.** Workflow already has the
   shard structure for both, but they haven't been exercised this
   session (Linux focus). Re-dispatch with `-f run_macos=true
   -f run_windows=true -f full=true` and triage.

5. **Re-enable slack-flow once the CEF stability fix lands.** It's
   the only spec the orchestrator deliberately skips today.

---

## Key file paths

- Workflow: `.github/workflows/e2e-reusable.yml`
- Orchestrator: `app/scripts/e2e-run-all-flows.sh`
- Local sharder: `app/scripts/e2e-run-shards.sh`
- Session runner: `app/scripts/e2e-run-session.sh`
- Build script: `app/scripts/e2e-build.sh`
- WDIO config: `app/test/wdio.conf.ts`
- Loopback auth helper: `app/test/e2e/helpers/loopback-auth-helpers.ts`
- Production loopback (exposes `__startLoopbackOauthListener` for
  E2E): `app/src/utils/loopbackOauthListener.ts`
- Reset-app helper: `app/test/e2e/helpers/reset-app.ts`
- Composio test helper: `app/test/e2e/helpers/composio-helpers.ts`
- Docker setup: `e2e/docker-compose.yml`, `e2e/docker-entrypoint.sh`

---

## Useful commands cheatsheet

```bash
# CI: dispatch a Linux-only full run on the fork (only run_macos / run_windows are inputs)
gh workflow run E2E --repo senamakel/openhuman \
  --ref ci/full-e2e-run-2026-05-23 \
  -f run_macos=false -f run_windows=false -f full=true

# CI: shard summary
gh run view <run-id> --repo senamakel/openhuman | grep -E '^(✓|X|\*|-) '

# CI: per-shard pass/fail + failing spec list
gh api repos/senamakel/openhuman/actions/jobs/<job-id>/logs > /tmp/job.log
grep -c 'PASSED in linux' /tmp/job.log
grep -c 'FAILED in linux' /tmp/job.log
grep 'FAILED in linux' /tmp/job.log \
  | sed -E 's|.*specs/||;s|\.spec\.ts.*||' | sort -u

# Local: full sharded run
docker compose -f e2e/docker-compose.yml run --rm e2e \
  bash -lc "bash app/scripts/e2e-run-shards.sh" 2>&1 | tee /tmp/local-shards.log

# Local: single shard
docker compose -f e2e/docker-compose.yml run --rm e2e \
  bash -lc "bash app/scripts/e2e-run-shards.sh foundation"

# Local: single spec
docker compose -f e2e/docker-compose.yml run --rm e2e \
  bash -lc "bash app/scripts/e2e-run-session.sh test/e2e/specs/<spec>.spec.ts"
```
