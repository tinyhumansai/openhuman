# Test Suite Audit & Improvement Plan

Multi-agent audit of the OpenHuman test surface (2,367 files / ~25,900 test declarations per
`docs/test-inventory/REPORT.md`). Ten parallel auditors each read their slice of the inventory
**plus the actual test source**; every "drop this test" / "this is overfitted" recommendation was
then adversarially reviewed by a second skeptic pass that tried to refute it by re-opening the
cited files (40 verdicts: 32 confirmed, 8 refuted — the refuted ones are documented below so
nobody deletes them later). A final completeness critic audited the audit itself.

**Headline findings**

1. The suite is **broadly healthy** — security/policy adversarial tests, tunnel crypto, composio
   client, core_process tests are genuinely behavioral and strong.
2. The two **highest-risk untested surfaces in the codebase**:
   `src/openhuman/security/policy/command_checks.rs` + `path_checks.rs` (the documented
   autonomy-enforcement core: `classify_command`, `gate_decision`, `is_workspace_internal_path`)
   and `src/openhuman/encryption/core.rs` (Argon2id + AES-256-GCM primitives) have **zero unit
   tests**.
3. A meaningful chunk of the suite **never runs in CI**: the mock server's own socket-auth tests,
   most of `scripts/__tests__/`, and the Pester Windows-install test are invoked by no workflow.
   A never-run test is worse than no test — it reads as coverage.
4. The 80% changed-lines coverage gate has a **scope hole**: `scripts/**` (including the entire
   shared mock backend every other suite depends on) is outside all path filters.
5. ~130 test declarations are **safe to delete** (verified duplicates, typeof-only boilerplate,
   empty compile-only tests, copy-paste connector templates) — see §2.

---

## 1. Method

- **Audit fan-out (Sonnet):** frontend unit (components/pages; services/store/hooks/lib),
  frontend E2E (WDIO + Playwright), Rust unit (agent/memory; channels/providers/platform;
  security/config/infra), Rust integration/E2E, Tauri shell, the mock/harness infrastructure
  itself, and a cross-cutting gap hunter comparing product domains ↔ inventory.
- **Skeptic verify (Opus, high effort):** every drop/overfit finding re-verified against source;
  refuted items retained with the refutation recorded.
- **Completeness critic (Opus):** audited the audit for missing dimensions (mutation testing,
  fuzzing, a11y, perf, flake data, CI orphans…).

Legend: ✅ = skeptic-confirmed · ⚠️ = plausible, not independently re-verified · ❌ = refuted (do **not** act).

---

## 2. Tests to DROP

### 2.1 Confirmed safe to delete

| # | File | What | Why |
|---|------|------|-----|
| ✅ | `app/src/components/settings/hooks/__tests__/useSettingsNavigation.test.tsx` | whole file | Asserts a retired, hardcoded-empty `breadcrumbs` field across 8 routes — tests a constant. Route resolution is fully covered by the sibling `useSettingsNavigation.coverage.test.tsx`. |
| ✅ | `app/src/services/api/{graphCentralityApi,graphCohesionApi,memoryFreshnessApi,connectionPathApi,memoryTimelineApi,entityAssociationsApi,namespaceOverviewApi}.test.ts` | the copy-pasted `exposes the public surface` test in each (7 files) | typeof-only assertions on aggregate objects **no consumer imports** (tabs import the named exports directly); the functions are behaviorally tested above in each file. Do one grep-and-delete pass. |
| ✅ | `src/openhuman/agent/harness/harness_gap_tests.rs` | `datetime_section_is_static_grounding_rule_not_a_volatile_timestamp` | Strict subset of `agent/prompts/mod_tests.rs::datetime_section_is_static_grounding_rule_without_volatile_timestamp`; the file's own header lists item 6 as covered elsewhere. |
| ✅ | `app/src-tauri/src/lib_tests.rs` | `setup_tray_function_signature_compiles`, `tray_setup_logging_patterns_exist`, `app_runtime_type_exists`, `is_daemon_mode_detects_daemon_flag` | Empty bodies / comments-only / the one real assertion is commented out / result discarded with `let _`. Cannot fail; verify nothing. |
| ✅ | `app/src-tauri/src/deep_link_ipc.rs` | `extract_deep_link_urls_filters_correctly` | Re-implements the prefix filter inline instead of calling the production `extract_deep_link_urls()` — passes even if the real filter regresses. Better fix: refactor to take an args slice like the Windows sibling (`collect_deep_link_urls_from_args`), then test the real function. |
| ✅ | `src/openhuman/routing/factory.rs` | `factory_constructs_without_panic_when_runtime_enabled`, `factory_llamacpp_provider_constructs_without_panic`, `factory_custom_openai_provider_constructs_without_panic`, `factory_lm_studio_provider_constructs_without_panic` | Skeptic traced the whole construction path — provably infallible (pure struct init), so the tests cannot fail. One test's comment claims to verify probe-URL selection but the body asserts nothing; fields are private so strengthening is blocked. |
| ⚠️ | `app/src/components/chat/ArtifactCard.test.tsx` | whole file (281 lines) | Duplicates `__tests__/ArtifactCard.test.tsx` test-for-test (in-progress label, download→done flow, error truncation + Show more, Retry). Keep the `__tests__/` version. |
| ⚠️ | `app/src/components/settings/panels/RecoveryPhrasePanel.test.tsx` | whole file (1 test) | Subsumed by the 35-test `__tests__/RecoveryPhrasePanel.test.tsx`. |
| ⚠️ | `src/openhuman/threads/ops_tests.rs` | `sanitize_generated_title_*`, `collapse_whitespace_*`, `title_log_fingerprint_*`, `title_from_user_message_*` | `threads/title.rs` inline tests already cover the same functions with equivalent cases. Keep the copies in `title.rs` (the owning module). |
| ⚠️ | `src/openhuman/channels/providers/whatsapp_tests.rs` | 7 × `whatsapp_parse_<type>_message_skipped` | All hit the identical `type != "text" → continue` branch; collapse to one parameterized loop over the type strings. |
| ⚠️ | `src/openhuman/routing/telemetry.rs` | `emit_does_not_panic` ×3 variants | Fire-and-forget calls with no assertion. |
| ⚠️ | `app/src-tauri/src/deep_link_ipc.rs` | `no_primary_returns_appropriate_result` | Its own comment admits it can't reach the production branch; asserts stdlib `UnixStream::connect` errors on a bogus path. |
| ⚠️ | `app/test/e2e/specs/smoke.spec.ts` | the permanently-`it.skip`ped auth-deep-link test | Skipped for a documented flake with no tracking issue/owner. File an issue and fix, or delete. |

### 2.2 Consolidate, don't delete: the connector template family

`connector-{airtable,asana,clickup,confluence,google-calendar,google-drive,google-sheets,notion,slack-composio,todoist,youtube}.spec.ts`
— **11 WDIO files, ~99 test declarations, byte-identical** except for toolkit name/slug strings.
Playwright already proves the fix: `connector-session-guard-matrix.spec.ts` parametrizes the same
assertions over a `TOOLKITS` array in one file. Extract a `runConnectorContract(toolkit, opts)`
helper, keep **`connector-jira.spec.ts`** (subdomain-field UI) and
**`connector-gmail-composio.spec.ts`** (400-on-fetch-emails handling) as bespoke specs.
Net: ~100 declarations → ~10 with identical per-toolkit coverage.

### 2.3 ❌ Refuted — keep these (recorded so they don't get re-proposed)

| File | What | Why it stays |
|------|------|--------------|
| `src/openhuman/memory/schema_tests.rs` | registry-sync + unknown-fn tests | **False duplicate.** `memory/schema/` (singular, `memory_tree` namespace) and `memory/schemas/` (plural, `memory` namespace) are two distinct live registries with disjoint function sets. These are the *only* parity/unknown-fn guards for the `memory_tree` controller surface. |
| `src/openhuman/channels/providers/qq_tests.rs` | `test_name` | Sole coverage of `QQChannel::name()`, which keys routing (`routes.rs:345`) and the channel map (`runtime/startup.rs:701`). A rename would ship uncaught. |
| `src/openhuman/provider_surfaces/schemas.rs` | `all_schemas_returns_two` etc. | Weak but the only guard that the registration lists are populated. **Improve** (see §3), don't delete. |
| `src/core/jsonrpc_tests.rs` | wallet-message drift guard (L1421) | The literal pin is load-bearing: three Rust producers and six frontend components hardcode the same string; the pin is what forces a human to update the desynced sites on a wording change. |
| `tests/x402_twit_sh_live.rs` | `#[ignore]`d live x402 test | Only coverage artifact for the money-moving `X402RequestTool` (compile-pins its API + manual E2E harness). Costs nothing in CI. |
| `app/test/e2e/specs/gmail-flow.spec.ts` | blanket drop of 8 tests | Over-reach: 4 of the 8 carry a real "app still boots with revoked/expired-token mock state" guard nothing else reproduces. Only 9.1.2 and 9.3.3 are true no-ops. Fix the fixture instead (§3). |

---

## 3. OVERFITTED tests to rewrite

| # | File / test | Problem | Fix |
|---|-------------|---------|-----|
| ✅ | `src/openhuman/agent/prompts/mod_tests.rs::grounding_contract_requires_exact_numeric_evidence` | Pins 5 verbatim prose substrings of the grounding contract — breaks on any copywriting pass. | Behavioral guarantee ("contract appended on every build path") already covered by the marker-based test; convert this to a single explicitly-labeled wording-lock, or assert stable structural markers. |
| ✅ | `src/openhuman/agent/prompts/mod_tests.rs::identity_section_creates_missing_workspace_files` | Also string-matches SOUL.md brand-voice prose (`"Don't validate FUD"`). | Split: (a) files created + seeded from the checked-in template (compare against template file content); (b) a narrow, labeled brand-voice lock if the phrase must stay pinned. |
| ✅ | `app/src-tauri/src/core_process_tests.rs::startup_timeout_cleanup_aborts_task_and_clears_slot` | 5 substring checks against one human-readable diagnostic string. **Caveat:** it also asserts real behavior (task slot cleared, shutdown token cancelled) — preserve those two assertions. | Return a small struct (attempt, port, ready_signal, port_open, task_state) + display string; assert struct fields, one loose `contains` on the text. |
| ✅ | `src/openhuman/hooks/../useDaemonLifecycle.test.ts` (`app/src/hooks/__tests__/`) | Pins exact `console.log` strings as an effect-rerun proxy. Listener-count assertions are legit — keep them. | Drop the log-text pinning; keep listener/startDaemon observable assertions. |
| ✅ | `src/openhuman/provider_surfaces/schemas.rs::all_schemas_returns_two` / `all_controllers_returns_two` | Magic-number count breaks on any legitimate 3rd controller. | Replace with `schemas().len() == controllers().len()` parity + presence of a known op (`list_queue`). Standardize this as a shared `assert_schema_controller_parity()` helper — the `== N` pattern repeats across ~15 domains. |
| ✅ | every `connector-*.spec.ts::composio_sync RPC routes to mock backend` | Name promises routing; body only asserts the session didn't crash (original assertion removed per inline comment). | Rename to what it checks, or move the real routing assertion to a native-provider connector where sync actually hits the mock. |
| ✅ | `tests/composio_raw_coverage_e2e.rs::composio_controller_schema_catalog_covers_all_declared_functions` | Hardcoded `inputs.len() == N` per function breaks on additive optional params. | Assert on specific input *names*, keep the meaningful namespace/name/output checks. |
| ⚠️ | `app/src/components/chat/__tests__/ApprovalRequestCard.test.tsx` opaque-surface test | Asserts exact Tailwind class names + a regex banning opacity utilities. | Assert computed style, or accept it as a labeled visual-regression lock. |
| ⚠️ | `AppWalkthrough.test.tsx` | `querySelector('div.bg-gradient-to-r')` and hardcoded `returns 13 steps`. | `data-testid` for the progress bar; drop the step count (first/last-target tests carry the signal). |
| ⚠️ | `app/test/e2e/specs/gmail-flow.spec.ts` | "if UI not found, log and pass" pattern — degrades to no-op instead of failing. | **Fix the fixture:** seed deterministic Gmail-skill discovery in the mock, then assert unconditionally with no branching. |

---

## 4. MISSING coverage — prioritized backlog

### P0 — security, crypto, data-loss, ingress (do these first)

1. **`security/policy/command_checks.rs` + `path_checks.rs` — ZERO tests.** The enforcement core
   of the entire autonomy model. Build a parametrized gate-matrix suite:
   (tier `readonly/supervised/full`) × (`CommandClass` Read/Write/Network/Install/Destructive) ×
   path-root, asserting `gate_decision` Allow/Prompt/Block; `is_always_forbidden` blocks
   system/credential dirs even when tier+trusted_roots would allow; `is_workspace_internal_path`
   fail-closed regardless of tier; `classify_command` "unrecognized ⇒ Write" fail-safe against
   pipes/redirects/chained commands. (Note: `policy_tests.rs` — 2,606 lines — is excellent but
   covers the *higher-level* policy, not these modules.)
2. **`encryption/core.rs` — ZERO tests** on the Argon2id/AES-256-GCM primitives. Round-trip;
   wrong password / tampered ciphertext / tampered nonce rejection; fresh salt+nonce per call;
   KDF determinism. Plus one RPC→crypto→RPC integration test (schema tests currently never touch
   real ciphertext).
3. **`memory/read_rpc/admin.rs::delete_source_rpc` — ZERO tests** on a 427-line destructive
   cascade-delete (chunks, embeddings, entity-index, content files, orphan-tree). Cover: exact
   source_id scoping (prefix siblings untouched); shared path_scope/collection trees NOT torn down
   while referenced; orphan cascade when fully orphaned; idempotency; legacy partial-delete
   recovery.
4. **Event bus panic isolation** (`src/core/event_bus/bus.rs`): production `catch_unwind` exists
   precisely so one handler can't kill the loop — no test exercises it. Two subscribers, one
   panics, assert both keep receiving subsequent events.
5. **Webhook ingress flood** (`webhooks/router.rs`): externally-triggered, unauthenticated-by-default
   surface with no rate-limit/backpressure test. If no such logic exists, that's a **product** gap.
6. **E2E: core-process crash/recovery.** Blocked on a missing debug-only `stop_core_process`
   Tauri command (documented in `connectivity-state-differentiation.spec.ts`'s own header). Add the
   command, then: kill core mid-chat-turn → recoverable error (not a hang) → offline state →
   `restart_core_process` restores a working session.
7. **E2E: RPC bearer auth failure.** No spec covers `core_rpc_relay` with a missing/tampered/expired
   token; `tauri-commands.spec.ts` is happy-path only. Also add the in-memory token-handoff
   round-trip test (`core_rpc_token` → relay → Authorization header → 401 without it).
8. **CI wiring (see §5):** un-run harness self-tests are a P0 process gap.

### P1 — core workflows

- **Approval gate × agent turn**: harness-level test that a Write/Destructive-class turn parks
  pending approval while background/cron turns bypass, plus the **concurrent double-decide race**
  on one request_id (exactly-one-terminal-outcome). Frontend twin: composer blocked while an
  approval card is pending on the thread (`composerInteractionBlocked` wiring is untested).
- **`TransportManager.raceLanAndTunnel()`**: the LAN-vs-tunnel race is entirely untested despite
  the docstring claiming otherwise — LAN wins (tunnel closed), tunnel wins (LAN closed), both
  fail, win-then-fail → `reset()` re-race.
- **Memory `path_scope` invariant**: two source_ids sharing a path_scope must summarize into one
  collection tree (the documented dedupe-key-vs-scope contract).
- **Hostile webhook payloads** (whatsapp/lark/linq/dingtalk/mattermost): wrong JSON types on
  expected fields, deep nesting, oversized bodies — current "malformed" tests only cover missing
  fields.
- **Socket reconnect/backoff state machine**: only yuanbao has explicit backoff-schedule tests;
  mirror them on the primary socket client (increase → cap → reset-on-success), plus
  socketService's reaction to mid-session auth expiry (teardown + fresh token on reconnect).
- **web3 swap/bridge/dapp tool wiring** (money-moving, untested at tool layer): schema rejection,
  delegation to ops with stubbed quote, error propagation. Plus one cross-domain quote→execute
  JSON-RPC E2E against a stubbed wallet.
- **`threadSlice.ts` (501 lines) and `accountsSlice.ts`**: no test files at all.
- **`CoreProcessHandle::restart()`** and **`core_rpc.rs::apply_auth()`** error paths (Tauri side).
- **E2E journey spec**: onboarding → connect channel → chat turn using it → approval gate →
  tool result renders → new thread → memory recall of the earlier interaction. Individual pieces
  exist; no single continuous-session spec does.
- **Approval-gate Playwright mirror**: `agent-harness-behaviors.spec.ts` exists only in the slower
  WDIO lane.
- **AgentAccessPanel tier cross-check**: which sub-controls are enabled/hidden per autonomy tier.
- **Onboarding credential forms**: happy-path persistence + invalid-input-blocks-navigation for
  CustomInference/CustomSearch pages.
- **Coverage-gate scope**: add `scripts/**` (and `packages/**`) to the path filters (§5).
- **~20 RPC controller domains with zero E2E references** (`recall_calendar`, `tinyplace`,
  `devices`, `people`, `redirect_links`, …) — one RPC round-trip each; full list in Appendix A.3.

### P2 — polish / lower blast radius

- Unicode-homoglyph spoofing cases through `is_command_allowed` / path prefix checks (current
  multibyte tests are panic-safety only).
- Tunnel `framing.ts` adversarial reassembly (out-of-order, duplicate seq, truncated final chunk).
- Composio second-consecutive-auth-error must surface, not loop.
- `socketSlice` / `channelConnectionsSlice` / `pttSlice` / `providerSurfaceSlice` reducer tests;
  `Invites.tsx` page test.
- `redact_url_for_log` direct unit test (token-leak prevention).
- Port native-free WDIO-only specs (memory-sync-schedule, skill-activation-persistence,
  chat-thread-todo-strip, chat-background-activity-panel) to Playwright; maintain a documented
  allowlist of intentionally native-only specs.
- Symlink-chain (depth >1) cases for `is_workspace_internal_path` / `validate_path_within_root`.
- `tauri-plugin-ptt` Rust command surface (iOS is non-shipping — lowest priority).

---

## 5. Harness, mocks & CI — audit results

**Good news:** the three mock entry points (`scripts/mock-api-core.mjs`, `scripts/mock-api-server.mjs`,
`app/test/e2e/mock-server.ts`) are thin shims over **one** implementation in `scripts/mock-api/`,
so drift risk is ~nil. Deterministic seeding (`fuzzyNumber`/`fuzzyPick` off `mockBehavior.seed`)
already exists. `httpFaultRules` gives generic per-route status/body/latency injection.

**Holes found (verified by exhaustive grep of package.json + workflows):**

1. **Orphaned tests — never run in any CI job:** `scripts/mock-api/socket.auth.test.mjs`,
   `socket.transport.test.mjs`, most of `scripts/__tests__/*.test.mjs`, and the Pester
   Windows-install test (`OpenHumanWindowsInstall.Tests.ps1`). The mock server's socket-auth
   behavior — which every E2E suite depends on — is itself unverified in CI.
   → Add `pnpm test:scripts` (`node --test scripts/**/*.test.mjs`) + a CI step; wire the Pester lane.
2. **Coverage-gate scope hole:** the diff-cover gate only arms on `frontend || rust-core || rust-tauri`
   path filters; `scripts/**` (mock backend, debug runners, coverage checkers) is outside all of
   them. A mock-backend rewrite ships with zero coverage pressure.
   → Add `scripts/mock-api/**` + `scripts/*.mjs` to the filters; optionally feed a c8 lcov into the
   same diff-cover invocation.
3. **Fault-injection ceiling:** the mock can inject clean HTTP errors and latency, but **cannot**
   simulate connection reset mid-response, hung requests, truncated chunked bodies, or malformed
   (non-JSON 200) responses — all real outage shapes.
   → Add `mode: "reset"` and `mode: "malformed"` to `httpFaultRules`; document a small "chaos
   toolkit" (500 storm, slow drip, auth-expiry-mid-session) so authors stop adding bespoke
   one-off behavior flags (auth.mjs already has ~6 that `httpFaultRules` could express).
4. **Order-dependent E2E flake source:** `wdio.conf.ts` runs all specs in ONE session with state
   deliberately carried spec-to-spec, over a mock with module-level mutable state that only resets
   when a spec remembers to call `/__admin/reset`. A spec failing before its reset poisons the next.
   → Unconditional `/__admin/reset` in a WDIO `afterTest`/per-spec hook.
5. **Mock↔backend contract drift:** nothing validates mock route responses against the TS types the
   frontend api clients expect. → zod/JSON-schema contract test over a sample of mock responses;
   extend the existing `rpcMethods.test.ts` drift-guard pattern (frontend constants ↔ Rust schema
   source) to tool names and event names.

---

## 5bis. Re-check under the two-lane CI (2026-07-03, after #4486)

CI moved to a two-branch model: PRs → `main` run a fast lane (`pr-ci.yml`: quality checks +
unit tests **only for changed files** — `vitest related` / domain-scoped `cargo llvm-cov`), and a
standing main→release PR (auto-refreshed by `prepare-release-pr.yml` on every push to main) runs
the full lane (`release-ci.yml`: full unit suites, Rust mock-backend E2E, Playwright, desktop E2E
on 3 OSes, gated by "Release CI Gate"). Re-audit of every CI-related finding:

### Improved by the change

- **Full-suite cadence is now structural**: because the standing release PR synchronizes on every
  main push, the complete unit + Rust E2E + desktop matrix runs against every mainline state —
  cross-domain breakage can no longer reach a *release* unseen. (It can still land on `main`
  unseen; see below.)
- Legacy `e2e.yml` / `e2e-playwright.yml` / `test.yml` are now `workflow_dispatch`-only — the
  fan-out lives behind the release gate instead of ad-hoc triggers.

### Findings from §5 that are STILL open (re-verified by grep on the new workflows)

| Plan item | Status |
|---|---|
| Orphaned harness self-tests (`scripts/mock-api/socket.{auth,transport}.test.mjs`, most of `scripts/__tests__/`, Pester `test:install-ps1`) | **Still orphaned.** No workflow references them in either lane (`pnpm docs:test` remains the only `scripts/__tests__` entry point; no `pwsh` anywhere). |
| Coverage-gate scope hole for `scripts/**` | **Still open, and slightly worse.** The new `pr-ci.yml` path filters still exclude `scripts/mock-api/**` and `scripts/*.mjs` (only `ci-cancel-aware.sh`, `test-rust-e2e.sh`, `test-rust-with-mock.sh`, `scripts/ci/*.sh` appear). A mock-backend rewrite now triggers **zero tests of any kind on the PR lane** — the code it can break (Rust E2E, Playwright, desktop E2E) only runs at the release PR. |
| Per-spec mock `/__admin/reset` WDIO hook | **Not landed** (`wdio.conf.ts` unchanged). Matters more now: the full WDIO matrix is the release gate, so its order-dependent flakes directly block releases. |
| Test-file→CI orphan check | **Not implemented.** |
| `check-domain-e2e-coverage.mjs` | Still not wired into any workflow (`check-coverage-matrix.mjs` runs in `pr-quality.yml`). |

### §4/§A missing-test items: unchanged

The restructure moved *where* suites run; it added no tests. Re-verified the P0s are still
untested: `command_checks.rs`, `path_checks.rs`, `encryption/core.rs` have zero `#[test]`s and
`delete_source_rpc` has none. Everything in §4 and Appendix A.3 stands.

### NEW gaps introduced by the two-lane model

1. **Playwright is decorative even at the release gate.** `playwright-e2e` has
   `continue-on-error: true` (TODO #3615), and a job with `continue-on-error` reports `success`
   to `needs`, so "Release CI Gate" can never fail on it. The whole web-E2E lane is currently a
   can-never-fail check — the same anti-pattern as the gmail-flow specs, one level up. Either fix
   the flaky specs and drop the flag, or exclude the job from the gate's `needs` and say so.
2. **Cross-domain regressions land on `main` undetected.** The fast lane's own script comments
   state it: coverage/tests scoped to the changed domain only — a change in domain A that breaks
   domain B's tests, or breaks `tests/*.rs` integration behavior, merges green and only fails on
   the release PR, where failures from many merged PRs arrive **batched** and need bisection.
   Mitigations: treat a red release PR as build-cop priority with a revert-first policy; and/or a
   scheduled full unit run on `main` (cheaper than per-PR, catches breakage within hours with
   per-commit granularity via `git bisect` against a known-good tag).
3. **Rust integration tests (`tests/*.rs`) never run on the PR lane for `src/**` changes** —
   `rust-coverage-changed.sh` maps `src/<a>/<b>` to unit-test filters and only runs a `--test`
   target when the *test file itself* changed. An RPC-behavior regression in a domain with thin
   unit tests but good E2E coverage sails through the fast lane. Consider mapping changed domains
   to their obvious integration targets too (e.g. `src/core/**` → `--test json_rpc_e2e`).
4. **`vitest related` is import-graph-based**: tests coupled to a changed file through non-import
   seams (mock fixtures, runtime registration, `?raw` prompt assets) won't be selected. The
   0%-lcov backstop protects *changed lines lacking any test*, not *existing tests that would now
   fail*. Low frequency, but worth knowing when a release-PR failure looks "impossible".

### Phase 0 (updated) — ✅ landed

Done in `ci/phase0-test-substrate`: wired orphaned tests (`test:scripts` + `scripts-tests` job,
`pester-install` job), added `scripts/mock-api/**` + `scripts/*.mjs` to PR-lane filters (mock
changes now trigger the scripts self-test job), WDIO per-spec-file `/__admin/reset` hook,
orphan-check + controller-domain check (`generate-test-inventory.mjs` → `test-inventory` job),
and resolved the Playwright `continue-on-error` gate bypass (#3615, excluded from the gate).
**Still open (process, not code):** adopt a red-release-PR policy (build-cop + revert-first).

---

## 6. Strategy: how to get a lot of functional tests cheaply

The audit shows the marginal value is in **behavioral/functional tests over shared harnesses**,
not more per-file unit tests. Concrete leverage:

1. **Table-driven contract suites.** The repo's biggest waste is copy-paste template families:
   11 connector WDIO specs, 4 scanner-registry suites, ~4 allowlist tests × N channel providers,
   6× registry-composition patterns in `tools/ops_tests.rs`, ~15 `all_schemas_returns_N` files.
   Build shared helpers/macros once (`runConnectorContract()`, `allowlist_contract_tests!`,
   `assert_schema_controller_parity()`, generic `ScannerRegistry<T>`) — each collapses dozens of
   declarations while *increasing* consistency (some copies have drifted, e.g. one provider's
   allowlist suite missing the case-sensitivity check others have).
2. **The JSON-RPC E2E lane is the functional sweet spot.** Follow the repo's own ladder
   (Rust → RPC → UI): most new functional coverage should be `tests/*.rs` against the mock —
   cheap, deterministic, cross-domain. Priority suites: security-gate matrix over RPC, approval
   park/decide/TTL, encryption round-trip, memory ingest→recall→delete_source, wallet/web3
   quote→execute.
3. **Property-based tests where inputs are adversarial** (proptest — currently absent):
   `classify_command` ("never Read for a command containing an unescaped shell operator; never
   panics"), path validation with symlink chains, encryption round-trip, tunnel framing
   reassembly, webhook parsers. One property replaces dozens of hand-enumerated cases.
4. **Fuzzing the untrusted-input parsers** (cargo-fuzz — currently absent): webhook JSON decode,
   tunnel frame decode, deep-link URL parse. Subsumes most of the missing hostile-payload unit
   tests at far higher coverage-per-effort; run time-boxed in CI.
5. **Mutation testing to find vacuous tests systematically.** Auditors hand-found "asserts
   nothing" tests one at a time; `cargo-mutants` scoped to the P0 zones (security/policy,
   encryption, approval, webhooks) turns that into a measured, ranked list. Consider Stryker on
   `app/src/services/` later.
6. **One long journey spec per lane** (§4 P1) instead of many more element-visible smoke specs.
7. **Fixture discipline over test branching:** the gmail-flow "if UI missing, pass" pattern is the
   anti-pattern to ban — deterministic mock seeding, then unconditional assertions. Consider a
   lint/review rule: no `console.log`-and-return early exits in specs.

---

## 7. Systemic improvements (from the completeness critic)

Dimensions the suite (and this audit) currently have **zero** coverage of:

| Dimension | State | Action |
|---|---|---|
| Test→CI orphan detection | Pester + scripts tests confirmed orphaned; no systematic map | Extend `scripts/generate-test-inventory.mjs` to assert every discovered test file is invoked by ≥1 CI job; fail on orphans. **Highest-leverage systemic fix.** |
| Mutation testing | None (no cargo-mutants/Stryker) | Scoped cargo-mutants run on P0 zones. |
| Property-based testing | None (no proptest/quickcheck) | §6.3 targets. |
| Fuzzing | None (no cargo-fuzz targets) | §6.4 targets. |
| Accessibility | Zero a11y assertions in the whole frontend | jest-axe smoke lane over ~10 key screens (approval card, recovery phrase, onboarding, composer). |
| Performance/load | No benchmarks anywhere (no criterion/k6) | Start with criterion micro-benches on memory ingest + embeddings; defer load tests. |
| Flake quantification | WDIO: maxInstances 1, no retries, order-dependent state; flake rate unmeasured | Pull CI rerun/failure history, rank flaky specs; land the per-spec mock reset (§5.4). |
| Test-speed budget | Unknown critical path; 30s vitest timeout can hide sleeps | Emit per-suite duration report from the inventory generator; slowest-N list + soft budget. |
| Upgrade/migration fixtures | migrate_openclaw/hermes logic exists; no prior-version snapshot test | Check in a prior-version workspace/DB/config snapshot; assert forward migration with no data loss. |
| Local↔CI parity | Different E2E drivers per OS; GGML_NATIVE=OFF only locally | Document which specs run where; fold into the lane-parity report (§4 P2). |
| Concurrency as a category | Races found anecdotally only | Start with the approval double-decide + event-bus tests; consider loom for the event bus later. |

---

## 8. Execution plan

**Phase 0 — CI substrate (hours, do immediately)** — ✅ landed (PR: `ci/phase0-test-substrate`)
- [x] Wire orphaned tests into CI (`test:scripts` runs `scripts/__tests__`, `scripts/mock-api`
  socket auth/transport + route tests; new `scripts-tests` PR-lane job) and a Pester lane
  (`pester-install` job → `test:install-ps1`).
- [x] Add `scripts/mock-api/**` + `scripts/*.mjs` (+ `scripts/__tests__/**`) to the PR-lane path
  filters so mock-backend/script changes trigger the scripts self-tests.
- [x] Unconditional mock `/__admin/reset` per spec file in WDIO (`beforeSuite`, file-path-guarded).
- [x] Orphan-check in the inventory generator (`scripts/generate-test-inventory.mjs`, wired as the
  `test-inventory` PR-lane job via `test:inventory`): every script-level test file is invoked by
  ≥1 package.json script or workflow.
- [x] Controller-domain coverage check: every domain in `src/core/all.rs` referenced by ≥1 file in
  `tests/` (Appendix A.4). 26 currently-uncovered domains seeded into a burn-down allowlist.
- [x] Resolve the Release CI Gate Playwright `continue-on-error` bypass (§5bis item 1, #3615):
  `playwright-e2e` excluded from the gate's `needs`/results with an explanatory comment.

**Phase 1 — P0 coverage** — ⏳ partial (PR: `test/phase1-p0-coverage`). Each item was
**re-verified against current source first** (the §4 backlog was never skeptic-verified); three
of the five "ZERO tests" claims turned out **already covered**, so only the genuine gaps were filled.
- [x] **Encryption core round-trip + tamper suite** — GENUINE GAP (verified 0 prior tests in
  `encryption/core.rs`). Added 11 tests: bytes/string round-trip, KDF determinism (behavioural —
  key_bytes is private), wrong-password / different-salt rejection, tampered-ciphertext and
  tampered-nonce GCM auth failure, fresh-random-nonce-per-call, salt length+randomness,
  malformed-JSON and empty-plaintext edge cases.
- [x] **Event-bus panic isolation** — GENUINE GAP (verified: `catch_unwind` in `bus.rs` had no
  test; only `publish_without_subscribers`). Added a two-subscriber test: one handler panics on its
  first event then recovers; asserts the panicked handler's own loop survives AND a peer keeps
  receiving every event.
- [~] **Security gate-matrix (command_checks/path_checks)** — ALREADY COVERED. `policy_tests.rs`
  comprehensively tests `classify_command` (unknown⇒Write, redirect-lifts, highest-segment-wins,
  all classes), `gate_decision` (all tiers × classes + Install), `is_always_forbidden`, and
  `is_workspace_internal_path`. Plan's "ZERO tests" was inaccurate; no suite added (would duplicate).
- [~] **`delete_source_rpc` cascade** — ALREADY COVERED. `memory_store/chunks/store_tests.rs` has
  `delete_source_rpc_purges_document_source_fully`, `_unknown_id_is_idempotent`,
  `_rejects_empty_source_id`, `_cleans_legacy_partial_delete`, plus the cascade (shared-tree
  preserved while referenced, orphan cascade, escape-path/symlink rejection, per-owner scoping).
- [~] **Webhook flood** — PRODUCT GAP (verified: no rate-limit/backpressure/throttle anywhere in
  `webhooks/router.rs`; it is registration/routing/logging only). Filed as a product gap, not a test.
- [ ] **core-process crash/recovery E2E** and **RPC bearer-auth-failure E2E** — deferred: both need
  the Tauri crate (blocked locally by missing system `glib-2.0`) and/or the WDIO desktop harness.
  `tests/json_rpc_e2e.rs` already covers RPC bearer auth/401s at the transport layer; the remaining
  gap is the frontend `core_rpc_relay` E2E. To be done in the CI-capable environment.

**Phase 2 — deletions & rewrites (parallel with Phase 1, low risk)** — ✅ landed (PR: `test/phase2-drops-rewrites`)
- [x] §2.1 deletions applied (⚠️ items re-verified against source before deleting):
  useSettingsNavigation.test.tsx, RecoveryPhrasePanel.test.tsx, the 7 graph-api
  "exposes the public surface" tests, the duplicate root ArtifactCard.test.tsx (unique
  kind-label assertion ported into `__tests__/`), the it.skip smoke test; Rust:
  harness_gap datetime test, 4 lib_tests no-ops, 4 factory construct-only tests,
  3 telemetry emit-no-assert tests, 15 threads/ops_tests title dups (lowercase-hex
  assertion folded into title.rs), whatsapp 7→1 parameterized skip test; deep_link_ipc
  refactored to `collect_deep_link_urls_from_args` + real-fn test, vacuous no_primary test dropped.
- [x] §2.2 connector consolidation: 11 specs → `connector-composio-contract.spec.ts` +
  `runConnectorContract()`; jira/gmail-composio kept bespoke; misleading `composio_sync`
  test renamed across the contract + remaining specs.
- [x] §3 overfit rewrites (load-bearing assertions preserved): grounding wording-lock,
  identity split (template-compare + labeled brand-voice lock), provider_surfaces parity via
  new `assert_schema_controller_parity()`, composio catalog input-names, core_process
  loose-contains (keeps task-slot/shutdown-token asserts), useDaemonLifecycle (keeps
  listener asserts), ApprovalRequestCard (labeled visual lock), AppWalkthrough (data-testid).
- [x] Shared helper `assert_schema_controller_parity()` added (`src/core/all.rs`) and the
  connector contract runner. (`allowlist_contract_tests!` / envelope-unwrap helper: follow-up.)
- Deferred: §3 gmail-flow fixture fix (needs deterministic mock Gmail-skill seeding validated
  against a running desktop E2E app — not runnable in the authoring env; §2.3 flagged those
  tests as load-bearing boot guards, so they were left intact rather than risk an unvalidated
  E2E change).

**Phase 3 — P1 backlog** — ⏳ partial (PR: `test/phase3-p1-gaps`). Verification-first pass: like §4
P0, most P1 "untested" claims were **inaccurate against current `main`**. Verified state:
- [x] **`TransportManager.raceLanAndTunnel`** — GENUINE GAP (the sibling test only covered profile
  *selection*, never the race). Added a dedicated `TransportManager.race.test.ts`: LAN-wins /
  tunnel-wins (loser closed), both-fail throws, `reset()` re-race, and winner-caching. Surfaced a
  real timing quirk: when the first `isHealthy()` to settle is `false`, the
  `Promise.race`→`Promise.any` fallback grabs that already-fulfilled `null` and throws instead of
  waiting for a later-healthy peer — worth a production follow-up.
- [~] ALREADY COVERED (no gap): `composerInteractionBlocked` (ChatComposer + composerSendDecision
  tests), `threadSlice`/`accountsSlice` (multiple `__tests__`), CustomInference/CustomSearch
  onboarding pages, AgentAccessPanel (23 tests), and every P2 slice reducer
  (socket/channelConnections/ptt/providerSurface) + TeamInvites. web3 tool layer has `web3_tests.rs`.
- [~] NOT A GAP: primary socket-client backoff is delegated to socket.io
  (`reconnectionAttempts: Infinity`), not custom logic — nothing bespoke to unit-test.
- [ ] Genuine-but-deferred (need a full Rust build + deep store/parser fixtures, high iteration cost
  in the authoring env): memory two-source-one-tree `path_scope` invariant; broad hostile-webhook
  payload matrix (wrong types / deep nesting / oversized) beyond the current missing-field tests;
  the ~20 controller domains with no `tests/` reference (the Phase 0 inventory allowlist to burn
  down). Approval×turn integration, journey spec, and the Playwright approval mirror need WDIO/PW.

**Phase 4 — new dimensions** — ⏳ partial (PR: `test/phase4-new-dimensions`). Unlike the §4 backlog,
these are genuinely greenfield (the suite had zero of them), so they were built and validated:
- [x] **proptest** (was absent): added as a dev-dep; property suite for the command classifier +
  path check (never-panic on arbitrary/unicode input; fail-closed redirect floor). NOTE: an
  encryption round-trip property was trialled but dropped — under coverage Argon2id is ~2.5s/case,
  so it held the lib-test binary ~60s and deterministically exposed a pre-existing env-var race in
  the unrelated `config::schema::load` env-overlay tests; the round-trip stays covered by the
  Phase 1 fixed-input tests.
- [x] **Mock chaos modes** (§5.3): `httpFaultRules` gains `mode: "reset"` (connection reset) and
  `mode: "malformed"` (non-JSON 200) beyond clean statuses; `scripts/mock-api/chaos.test.mjs`
  drives the real server (auto-run by the Phase 0 `test:scripts` job).
- [x] **jest-axe a11y smoke lane** (§7 — the frontend had zero a11y assertions): axe-core over
  ArtifactCard + ApprovalRequestCard asserting no violations.
- [ ] Still open (larger tooling / own PRs): cargo-fuzz targets, scoped cargo-mutants on the P0
  zones, criterion micro-benches, migration snapshot fixture, per-suite duration report,
  mock↔backend contract test.

---

## Appendix A — Rust integration/E2E slice

*(Re-audited separately after the original workflow auditor returned an unusable result. These
findings did not go through the skeptic-verify pass — treat as ⚠️ plausible; spot-check before
deleting anything.)*

**Summary.** The 129-file suite is dominated by `*_roundNN_raw_coverage_e2e.rs` files (coverage
sprints, rounds 13–26) that re-exercise the same internal helpers (`app_state::snapshot`,
`AuthProfilesStore`, `threads::ops`, `memory_sources::sync`) across 5–8 files under different
names — **in-process calls, not RPC** — i.e. unit tests mislabeled `_e2e`. Real transport coverage
(bearer auth, 401s, public-path bypass, SSE auth, `/schema`) in `json_rpc_e2e.rs` is genuinely good.

### A.1 Consolidate / drop

- **The same app_state quarantine invariant is asserted in ~5 files** (`near90_closure_…`,
  `config_auth_app_state_connectivity_e2e.rs` ×2, `…round26…`, `config_credentials_raw_coverage…`).
  Keep the `config_auth_app_state_connectivity_e2e.rs` pair; drop the rest.
- **Four whole files are in-process unit tests wearing `_e2e` names**
  (`app_credentials_threads_round24_…`, `…sources_round26_…`, `…memory_sources_raw_coverage…`,
  `composio_credentials_state_raw_coverage_e2e.rs`): identical import quartet, no
  `build_core_http_router`, no `/rpc` call. Fold their unique branch cases into the owning domains'
  unit test modules; the RPC-level equivalents already exist in `json_rpc_e2e.rs`.
- **`x402_twit_sh_live.rs` / `live_routing_e2e.rs`** (`#[ignore]`, need real wallet/backend):
  contribute zero CI coverage but the x402 one is the *only* coverage artifact for the money-moving
  `X402RequestTool` (skeptic-confirmed keep in §2.3). Reconciled action: **move to a `tests/manual/`
  dir or Cargo feature gate** so they stop inflating the E2E count, and add mock-backed variants of
  the same assertions so the code paths actually run in CI.

### A.2 Overfitted

- `config_auth_app_state_connectivity_e2e.rs::worker_a_controller_schemas_are_fully_exposed` —
  exact-match hardcoded lists of 40+ RPC method names across 5 namespaces; any additive method
  breaks it. Fix: must-exist superset assertion, or diff against the live registry; or `insta`
  snapshots so additions are a `cargo insta review`, not a hand-edit.
- `composio_raw_coverage_e2e.rs::…covers_all_declared_functions` — (also §3) claims "all" but never
  checks `expected.len()` against the live catalog, so a new composio RPC silently gets zero
  coverage while the test stays green. Assert catalog-length parity so omissions fail loudly.

### A.3 Missing (adds to §4)

- **P1 — ~20 registered controller domains with zero references anywhere in `tests/`**, including
  real backend-facing surface: `recall_calendar`, `tinyplace`, `redirect_links`,
  `desktop_companion`, `devices`, `announcements`, `provider_surfaces`, `people`,
  `council_registry`, `audio_toolkit`, `agent_experience`, `http_host`, `skill_runtime`,
  `session_import`. Minimum: one RPC round-trip (primary read + write) per domain via
  `build_core_http_router`.
- **P1 — SSE `/events` payload delivery**: auth is well tested; nothing asserts an actual
  `DomainEvent` published via `publish_global` arrives as a correctly-shaped SSE frame.
- **P1 — embedded-core crash mid-RPC**: `ollama_lifecycle_e2e.rs` covers the Ollama sidecar and
  `port_conflict_recovery…` covers port collisions, but nothing kills the serving task mid-request.
  (Pairs with the §4 P0 frontend crash-recovery E2E.)
- **P2 — concurrent RPC cross-talk**: fire 20+ parallel mixed-method `/rpc` requests, assert each
  response `id` matches its request.
- **P2 — token rotation mid-session**: an open `/ws/dictation` or SSE stream outliving a bearer
  rotation should terminate, not silently continue.

### A.4 Ideas

- Reorganize `raw_coverage` sprint files into per-domain modules — the "round" layout is exactly
  why 20 uncovered domains went unnoticed.
- **CI check: every controller domain registered in `src/core/all.rs` must have ≥1 reference in
  `tests/`** — small script in the spirit of `generate-test-inventory.mjs`; would have caught all
  of A.3 automatically. (Add to Phase 0 alongside the orphan-test check.)
