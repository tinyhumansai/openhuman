# Complete TinySecurity migration: remaining implementation

Trackers: [OpenHuman #7328](https://github.com/tinyhumansai/openhuman/issues/7328),
[TinySecurity #1](https://github.com/tinyhumansai/tinysecurity/issues/1),
[TinyBox #27](https://github.com/tinyhumansai/tinybox/issues/27).
Binding owner design: `vendor/tinysecurity/docs/specs/security-module.md`,
`docs/specs/immutable-path-scopes.md`, `docs/plans/security-module.md`.

## Delivery boundary and starting evidence

TinySecurity v0.17.0 is published and pinned. It includes immutable command
policy registration, command classification and execution checks with atomic
hourly reservations, plus scoped policy and durable approval operations. OpenHuman
has host clients for those contracts. The remaining owner engines and host
consumers listed below are still required; a reserved bus type or a passing
bootstrap test is not evidence that its engine is implemented.

Deliver **one further cumulative TinySecurity PR**, against canonical upstream,
containing all owner work below. Continue the existing OpenHuman #7331 for host
work. Tasks are commit/review units, not separate TinySecurity PRs. Preserve all
checkpoint commits; never squash, reset, amend or bypass hooks. Root coordinates
release/pinning. Owner must merge and release before production host gitlinks,
registry versions or digests change. Copy every digest from release
`checksum.toml`, never from development builds.

Work inside this existing superproject worktree; no nested worktrees. For another
owner repository, use the same superproject branch and that owner's upstream PR.
Do not place missing TinyBox/TinyMCP/TinyRuntime/TinyFlows capabilities into the
host or TinySecurity as workarounds. Their upstream changes must merge/release
before dependent production pins. One TinySecurity PR does not prohibit required
PRs in those other owners.

**Base verification (2026-10-23):** fetched canonical `upstream/main`. The
`epic-7328-continue` worktree's base is exactly `74d2a79612`, the fetched
upstream head. The primary checkout's `main` has an extra local checkpoint
commit updating 21 submodule pointers and untracked submodule state, so it was
left untouched rather than force-reset or fast-forwarded over that commit.

## Invariants for every task

- Host normal dependencies name only `tinysecurity-bus`; its normal dependencies
  remain serde and thiserror, without runtime, transport, crypto or TinyTools.
  Internals and dependencies are compiled into the native module.
- Preserve `approval.*`, `security.*`, `sandbox.*`, `encryption.*` RPC names and
  existing payload compatibility. Add explicit migrations for stored/config data.
- Init/reinit carries secret credentials, endpoint and module service settings.
  Invocation args carry authenticated context and scoped policy references,
  never judge credentials, arbitrary tenant authority or model-provided tiers.
- Keep immutable PathPolicyId scopes. Expand to immutable full-policy scopes;
  never serialize tenants through process-global reinit or let one agent change
  another's policy. Policy activation increments the appropriate generation.
- Module absence, timeout, malformed reply or fault denies external effects.
  A fault latches; no reload/retry of that module in the same process. Missing
  caller configuration before a native call must not poison unrelated tenants.
- Disabled autonomy leaves discretionary classification/gates/allowlist/action
  budget inert, preserving credential/system/traversal/NUL floor and access-tier,
  origin, privacy and mandatory isolation boundaries.
- Write behavioral tests first, record the actual RED failure, implement, record
  GREEN and owning-suite results. Keep host characterization until its replacement
  tests exercise the same behavior through the released native module.
- Unit tests use explicit clocks/resolvers/storage seams. Unit files are sibling
  `*_tests.rs`, start `use super::*;`, and are declared with `#[cfg(test)]` plus
  `#[path = "…_tests.rs"] mod tests;`; never inline or legacy test filenames.
- No placeholders, empty crates, ignored failures or blanket lint allowances.
  Advertise a method only when all its engine, storage and failure paths work.
- Long checks use `scripts/ci-cancel-aware.sh` from the host root. Never export
  CARGO_TARGET_DIR or build under a temporary directory. Temp test data is fine.

## Test protocol and contract interfaces

Execution order: finish the in-progress command task, then remaining scoped
contracts/config, redaction, egress, callback/audit infrastructure, approvals,
judge, sandbox planning and crypto. Characterization/harness work precedes each
affected engine. Complete policy-widening human review after approval callbacks
exist; until then reject widening activation rather than install an approval
stub. Judge consumes completed redaction, audit and approvals. Sandbox/network
requirements remain hard denies until their engines work. Finish all owner gates
and the single owner release, then host migration tasks 12–16. This staged order
prevents intermediate methods from advertising incomplete authorization.

For each owner task, first run its named filter using
`cargo test --manifest-path vendor/tinysecurity/Cargo.toml -p <crate> <behavior>`;
save RED/GREEN output in the controller ledger. Then run the crate suite.
For host tests use `cargo test -p openhuman-cli --test <target>` or
`cargo test -p openhuman <filter>` through the cancellation-aware wrapper.
New root tests require explicit `[[test]]` entries in
`crates/openhuman-cli/Cargo.toml`. In-process backend tests call
`tests/support/tinyhumans_boot.rs::boot()` before use; network services are mocked.

Extend bus types in focused `policy.rs`, `approval.rs`, `redact.rs`, `egress.rs`,
`sandbox.rs`, `audit.rs`, `crypto.rs`, `callbacks.rs`; re-export from `lib.rs`.
Replace definitions in `future.rs` with compatibility re-exports rather than
duplicate types. Preserve existing method constants and fixtures. Define a
contract-version change and explicit old/new compatibility tests when adding
required fields or enum variants to strict serde payloads.

Shared interfaces to implement:

- `RegisterPolicy(RegisteredPolicy { path_policy_id, settings, tool_rules,
  command_policy_id, subject_scope }) -> PolicyId`; immutable typed ID that
  composes existing immutable path/command registries rather than duplicating
  them. Scope includes authenticated
  user/workspace/agent ceiling, not just agent name. Scoped Evaluate/Check bind
  PolicyId, verified CallerContext and canonical call fingerprint. Existing
  bootstrap calls retain safe behavior without a scope; they never gain effects.
- `PolicySettings`: enabled, allowed commands, rate limit, privacy, approvals,
  auto-approve origin rules, sandbox defaults and audit requirements. Path settings
  remain in PathPolicy. `PolicyInfo` returns all effective nonsensitive settings,
  current generation, implemented members and callback availability.
- `Decision` keeps generation/verdict/cacheable. Extend typed denial reasons for
  rate, privacy, URL, isolation, expired grant and required persistence failures.
  Unknown effects deny. Mutable approvals/rates/DNS results are never cacheable.
- Approval records bind owner, origin, thread/flow, policy scope/generation,
  call fingerprint, expiry, lifecycle, decision attribution and execution outcome.
  ApprovalStore supports scoped load and atomic compare-and-set transitions,
  plus durable flow/tool grants. Callback commits are explicit acknowledgements.
- Callback clients use SDK bus calls with bounded timeouts and typed errors.
  Never hold a module/host state mutex while awaiting host prompt/store callbacks.
  Reentrant Decide during a parked Evaluate must be supported without deadlock.

## Task 1 — Characterization inventory and native harness

**Host files:** `tests/security_policy_characterization.rs`,
`tests/security_approval_characterization.rs`,
`tests/security_redaction_characterization.rs`,
`tests/security_sandbox_characterization.rs`,
`tests/security_crypto_characterization.rs`, CLI manifest test tables;
`crates/openhuman-core/src/modules/security_native_tests.rs`.
**Owner files:** `crates/tinysecurity-module/tests/native_contract.rs`,
`crates/tinysecurity-bus/tests/fixtures/`, `docs/performance.md`.

1. Pin existing approval RPC/event shapes, TTLs (600s/180s), origin/flow/tool trust,
   disabled/enabled policy, command syntax, sandbox precedence and encrypted data.
2. Build one corpus from all host redactors, preserving missed-secret cases as
   explicit desired regressions rather than blessing their current omissions.
3. Native harness loads a real cdylib, serves typed callbacks, detects malformed
   args and exposes deterministic clock/resolver/stub-judge seams for later tasks.
4. Capture existing in-process and native Evaluate/Check latency p50/p99 and calls
   per turn; record runner/hardware/workload. Set a measured budget in
   `docs/performance.md` before host caller migration, then enforce it in CI.
**Exit:** characterization passes, native harness proves actual dispatch; desired
missing behaviors are RED, not removed or marked successful.

## Task 2 — Full scoped configuration and activation

**Owner files:** bus `policy.rs`, `names.rs`, `callbacks.rs`, policy
`src/policy_registry.rs`, `src/policy_registry_tests.rs`, module `adapter.rs`.
**Host after release:** `config/schema/security.rs`, schema `mod.rs`,
`config/migrations/security_policy.rs`, migration `mod.rs`,
`modules/security_config.rs`, `security/live_policy.rs`.

1. RED: concurrently register two agents/tenants with opposite command/privacy
   rules; scoped evaluations remain isolated during reload and invalid reinit.
2. Implement validated immutable policy registration and complete PolicyInfo.
   Config activation is atomic; failure leaves prior policy active. Review widening
   changes (roots/hosts/tools/classes/auto origins/judge) through human approval
   under old policy before publishing the new generation; cannot self-approve.
3. Choose and implement one `[security]` table. Migrate legacy autonomy/sandbox/
   privacy fields with deterministic precedence and preserve default policy off.
   Remove unreachable DaemonConfig.security/SecurityConfig duplication.
4. Remove unused `max_cost_per_day_cents` with a migration notice and fixture;
   do not advertise an unenforced cost budget. Keep actual judge budget separately.
5. Host translator derives authenticated scope and all fields once. No operator
   config/env fallback for SaaS. Secret settings never enter PolicyInfo or logs.
**Exit:** round-trip old config fixtures and isolated new effective policies pass.

**Host status (TinySecurity v0.17.0, published 2026-10-22):** done: `[security]` table
(`config/schema/security.rs`, audit and approval TTL wired from it, folded
`[security.autonomy|sandbox|privacy]` overlay), migration 14->15
(`security_policy.rs`, `max_cost_per_day_cents` deleted with a notice), single
translator (`modules/security_policy.rs`), scoped client with generation-keyed
decision cache (`modules/security_scoped.rs`), `live_policy::reload_from_config`
driver, `SecurityPolicy::from_full_config` (privacy no longer hard-coded), the
trusted-host binding in `module_config`, and a released-module two-tenant test.
Host follow-up is done: `Config.security` owns typed autonomy, sandbox and
privacy fields; the pre-decode rewrite folds legacy top-level tables into the
canonical nested shape with nested values taking precedence. Save emits only
`[security.*]`. Config consumers across core/embed/TinyHumans and profile-owned
config overlays use the nested fields. The config RPC snapshot now uses
TinySecurity endpoint-origin redaction for storage URLs as well as inbound queue
URLs, suppressing unsupported schemes and database names. Stale characterization
imports were updated to current security-owned turn-origin, approval-gate and
prompt-scan APIs. Regression coverage checks legacy decode, precedence, save
shape, sandbox retirement, profile refresh and endpoint suppression.
Filesystem, network and shell gates route through TinySecurity clients; the
former TinyBox shell calls are gone from `command_checks.rs`, which now only
parses the model's escalate-only class hint. The host subscribes to
`PolicyChanged`. Still open: scoped audited variants and completing the
redaction/egress consumers in Task 12. Task 7's released durable whole-tool
grant support is tracked below.

**Windows native redaction follow-up (2026-10-23):** OpenHuman's Windows
`moved_text_callers_redact_through_the_released_archive` test caught that
TinySecurity's redacted home path retained Windows backslashes while the host
display contract uses `~/...`. `normalize_redacted_path` now normalizes
separators only after the home marker; both the focused display test and the
released-archive caller test pass locally. The fix is on OpenHuman PR #7331;
the refreshed hosted native matrix is running.

## Task 3 — Command grammar, tool rules and action accounting

**Released and integrated (TinySecurity v0.17.0):** the owner implements immutable
`CommandPolicy { enabled, autonomy, allowed_commands, max_actions_per_hour,
require_approval_for_medium_risk, block_high_risk_commands, action_dir, home_dir,
execution_mode }`, opaque `CommandPolicyId`, RegisterCommandPolicy,
ClassifyCommand and CheckCommand. Mode is `HarnessGated | Allowlisted`, selected
only at trusted registration. Pure classification returns typed class/risk/gate/
denial and reserves nothing; Check atomically reserves allowed hourly actions.
Registry survives reinit, clocks are injected and stateful checks are uncached.
HarnessGated preserves legacy shell check_gated_command behavior without imposing
Allowlisted's allowlist/risk gate; validate_command_execution uses Allowlisted.
Later approval/middleware gates derive command-class decisions from verified
context. Keep these distinctions in characterization and native tests.

**Owner files:** policy `src/command.rs`, `src/command/{classify,scan,env_guard}.rs`,
`src/rules.rs`, `src/rate.rs`, `src/engine.rs`, sibling tests; bus policy types.
**Source parity:** host `security/policy/{command_checks,enforcement,types}.rs`,
`tools/rules/`; TinyBox shell classifier/scanner/environment rules.

1. RED: POSIX compounds/substitution/redirection, quoted heredoc data, expanded
   heredocs; PowerShell/cmd escaping, paths and PATHEXT executable resolution.
2. Port classification and scanning to TinySecurity. Known reads remain reads,
   unknown commands become writes when enabled. Never trust declared class.
   Floor scanning recognizes protected literals through supported syntax.
3. Reuse the existing vendored TinyTools ToolRules/ApprovalDirective vocabulary
   internally via the single TinyAgents-owned copy; translate serde bus records
   mechanically. Tool visibility/access ceiling and explicit denies dominate.
4. Deterministic reservation/commit/release accounting enforces hourly actions
   without duplicate charges on parked/resumed calls. Disabled policy does not
   reserve. Reinit/cache cannot replenish or bypass active reservations.
5. Preserve merged path registry tests, adding full-policy binding and real native
   Windows/APFS/Linux cases; no second path normalizer.
**Evidence:** the `command_registry` filter passes 12 tests, and the full
`tinysecurity-policy` package suite passes at v0.17.0. OpenHuman's
`modules/security_command.rs` registers immutable host-derived scopes and uses
`ClassifyCommand` for authorization and `CheckCommand` for execution; native
production tests exercise the released adapter. Remaining duplicate host
redaction and egress consumers are tracked under Task 12, not this command task.

## Task 4 — One redactor and integrated scans

**Owner files:** new `crates/tinysecurity-redact/{Cargo.toml,src/lib.rs}`, internal
`src/{patterns,structured,prompt}.rs` and sibling tests; bus `redact.rs`;
module dispatch; workspace members and internal umbrella wiring.

1. RED shared corpus: secrets, PII/identifiers, nested args, key names, escaped JSON,
   URL userinfo/query, Unix/macOS/Windows home paths, malformed/bounded inputs.
2. Implement one registry with explicit modes. Add idempotence/no-secret-survives
   properties and bounded-work fuzz cases; preserve useful nonsensitive errors.
3. Implement ScanPrompt/ScanToolDefinition verdicts and stable rule IDs, route
   blocked/suspicious results into policy/approval. Scanner verdict is context,
   never authorization. Document TinyMCP protocol sanitation and TinySkills
   package scanning as owner inputs; avoid reproducing those implementations.
4. Redact before storage, broadcasts, audit and judge. Sensitive log paths on
   module failure suppress content rather than return original text. Only a
   documented fixed boot diagnostic floor may exist before module readiness.
**Exit:** native Redact/scans and every shared-corpus/property test pass.

## Task 5 — URL guard, privacy and actual rebinding protection

**Owner files:** new `crates/tinysecurity-egress/{Cargo.toml,src/lib.rs}`, internal
`src/{url_guard,resolver,privacy}.rs`, sibling tests; bus `egress.rs`.
**Host after release:** `tools/impl/network/host.rs`, `modules/browser.rs`,
`security/egress/`, `web_chat/egress_surface.rs`, `inference/provider/factory.rs`,
Composio loopback gates and `util/url.rs` callers.

1. RED literals: metadata/link-local, CGNAT, private/mapped IPv6, unusual numeric
   IPv4 forms, zones, userinfo, unsupported schemes and mixed-address DNS answers.
2. Resolve in module, validate every candidate, return hostname plus exact approved
   IPs and bounded validity. Any forbidden candidate denies. Redirects are checked
   independently. Destination allowlist is scoped policy, not caller authority.
3. Host HTTP transport disables unchecked automatic redirects and connects only
   to approved IPs while preserving Host/SNI/TLS hostname. Test a resolver that
   changes after validation and a real local transport proving no second lookup.
4. Browser/TinyComputer or proxy routes require a checked egress proxy capable of
   per-hop pinning; if unsupported, deny protected network operation. URL string
   validation alone must never be reported as rebinding protection.
5. LocalOnly denies nonlocal egress; Standard enforces declared destination/data
   policy; Sensitive denies identifying/credential-bearing raw egress unless an
   explicit approved transformation removes it and descriptor is rechecked.
   All inference/embedding/memory/integration/browser paths supply descriptors.
**Exit:** native URL/privacy tests and real pinned-transport tests pass.

**Native DNS fixture progress (2026-10-23):** TinySecurity PR
[#46](https://github.com/tinyhumansai/tinysecurity/pull/46) adds a bounded,
debug-test-only resolver with per-host answer sequences for native module tests.
Its parser rejects missing, malformed, exhausted, case-colliding, and exact
duplicate hostname entries. The exact-duplicate regression was observed failing
before the duplicate-aware visitor was added. The focused resolver tests pass
(7), Clippy and format checks pass, and
`scripts/ci/security-browser-egress-fixture.sh` passes against the native module,
including the real host callback's public-to-loopback DNS-rebinding denial. PR
checks and reviewer re-review are still pending; this does not change the
production module pin.

**Coverage follow-up (2026-10-23):** PR #46's Linux per-file coverage gate
identified `native_test_resolver.rs` at 79.67%. The owner branch now exercises
fixture bounds, malformed shapes, hostname validation, resolver argument
limits, normalized-key collisions and valid/invalid configuration selection.
The resolver-specific suite passes 7/7; a focused `cargo llvm-cov` report shows
94.44% line coverage for that source file, and strict module Clippy plus format
checks pass. The update was pushed as `ed63e62`; subsequent Windows CI exposed
that coverage-instrumented cipher fixtures were using 50 ms backend and
5-second bus deadlines. Test-only timeout adjustments are now pushed to PR #46
through `d8707a0` and `c18b56f`. The focused resolver command
(`cargo test -p tinysecurity-module --features native-security-tests
native_test_resolver --lib`) passes 7/7 locally. A broad `--all-features`
invocation still hits the existing `needs_drop::<ghash::GHash>()` assertion in
`tinysecurity-crypto/src/password.rs`; it does not affect the focused resolver
build. This remains test-fixture work and does not change the production pin.

**Hosted Windows follow-up (2026-10-23):** one PR #46 Windows Rust job failed
in `secret_cipher_native_contract` at mode 13, where `ExecuteSecretCipher2`
exceeded the fixture's 5-second bus timeout while secret-cipher encryption was
disabled. A second Windows coverage run failed the confidential-cipher fixture
with a generic authority denial. Both failures are covered by the test-only
timeout adjustment pushed as `d8707a0`; its Windows and other platform checks
are pending, so it is not yet verified green. A follow-up at `c18b56f` gives the
fixture proxy a 35-second timeout so the duplicate-execution assertion observes
the module's replay rejection instead of a client timeout. This also addresses
CodeRabbit's specific review request on that assertion. The change is on PR #46;
the current CI runs are `39889136776` and `39889142841` and are still running.

## Task 6 — Audit engine and callback infrastructure

**Owner files:** new `crates/tinysecurity-audit/{Cargo.toml,src/lib.rs}`, internal
`src/{event,sink,rotation}.rs`, tests; bus `audit.rs`, `callbacks.rs`; module
`src/callbacks.rs`, callback and adapter tests.
**Host after release:** `modules/security_host.rs`, `modules/mod.rs`,
`security/approval/store*.rs`, existing host storage driver wiring.

1. RED: sink/store unavailable, timeout, wrong owner, malformed acknowledgement,
   callback reentrancy, duplicate events and rotation during Windows file locking.
2. Implement bounded typed callback clients. Native callback harness verifies
   bus interface identities, caller ownership and commit acknowledgements.
3. One ordered audit stream covers policy, approval, judge, shell/execution and
   sandbox decisions. Redact event summaries; use content-free applied rule IDs.
4. Required audit failure denies before execution. Optional failure reports
   sanitized degraded health, never falsely committed. JSONL rotation uses 0600
   on Unix and restricted Windows ACLs. Host callback sink is configurable.
**Exit:** persisted native audit and callback failure/security tests pass.

## Task 7 — Durable approvals, grants and expiry

**Owner files:** new `crates/tinysecurity-approval/{Cargo.toml,src/lib.rs}`, internal
`src/{state,store,grants,reply}.rs`, tests; bus `approval.rs`; module adapters.

1. RED pending→decided→executed, expiry at exact boundary, restart reload,
   duplicate/replayed Decide, concurrent decision, wrong tenant/context/fingerprint,
   changed policy, callback failure and terminal outcome acknowledgement.
2. Persist pending before RequireApproval. Store transitions are CAS/idempotent.
   Module reloads scoped durable records/grants; expired state never authorizes.
   TTL defaults 600s, copilot/sub-agent 180s; test injected-clock rollback safely.
3. Implement allow once/tool/flow and deny; tool grants persist/reload without
   silently granting different arguments/classes, and flow grants bind reviewed
   fingerprints. Policy widening invalidates/reviews affected grants.
4. ParseReply only parses intent; Decide requires authenticated human authority.
   Cron reads only; external effects deny. SaaS uses authenticated per-user prompt
   callback, denies if absent; remove unconditional SaaS approval bypass.
5. Host owns parked futures/cancellation and ApprovalRequested/ApprovalDecided
   events. Module owns state/TTL/log. Callbacks do not create recursive lock waits.
**Exit:** restart-surviving real native approval round trip, no unsafe grant replay.

**Release and host progress (2026-10-22):** TinySecurity v0.16.0 is published
and its release workflow passed. Its contract adds durable whole-tool
grant/revoke/list operations and immutable legacy-source import receipts.
OpenHuman now pins the release tag and all eleven supported platform checksums,
imports legacy `autonomy.auto_approve` entries through `ImportTrustSource2`, and
registers no transient whole-tool grants in each call's approval scope. A native
test against the published Linux archive verifies an imported grant still
authorizes a fresh engine with an empty transient grant list. Host list/revoke
surfaces were completed in the v0.17.0 follow-up below; the wider approval/SaaS
cases remain open. These changes are on the existing draft PR #7331; its
cross-platform native-policy and release latency jobs are running.

**Follow-up (2026-10-22):** TinySecurity v0.17.0 is now published and pinned
with all eleven host checksums from its release manifest. The v0.17 bus adds
`RevokeToolById2`; the host approval engine and `approval.list_tool_trust` /
`approval.revoke_tool_trust` controllers now expose the tenant-scoped durable
grant lifecycle. The new native regression passes against the published Linux
archive. The settings history panel now lists those grants and revokes the
selected opaque grant id, with translations in all supported locales and a
passing component/API test. A Windows native run also exposed a Unix-only
absolute-path fixture; it now uses the platform temp directory and its focused
test passes locally. That required TinyBus's already-merged secure-extraction
fix at `11b0a72`, because v0.17 release archives mark their root directory
group writable. A fresh three-OS released-artifact run remains open.

## Task 8 — Rules and optional Jev judge

**Owner files:** new `crates/tinysecurity-auto/{Cargo.toml,src/lib.rs}`, internal
`src/{rules,judge,budget}.rs`, tests; init JudgeConfig and approval wiring.

1. RED off-default/per-origin opt-in, allowlist/flow grants, auto_approve_all audit,
   judge allow/low-confidence/timeout/malformed/error/budget exhaustion and Deny.
2. Use tinyinference-decisions Jev API internally. Init supplies endpoint/credential;
   host resolves via resolve_backend_credential. No secret invocation fields.
3. Send redacted intent/reversibility/exfiltration questions; configurable class/
   origin thresholds and deterministic budget. Only RequireApproval may become
   Allow; hard denies, floor, privacy and isolation remain unchanged.
4. All errors/uncertainty fall back to human, unavailable human channel denies.
   Audit sanitized scores/thresholds/question IDs with auto:jev. Cache cannot
   issue reusable permission or bypass mutable checks.
**Exit:** native tests against local stub judge on all three OSes.

## Task 9 — Sandbox planning with complete capability policy

**Owner files:** new `crates/tinysecurity-sandbox/{Cargo.toml,src/lib.rs}`, internal
`src/{resolve,grants,capabilities}.rs`, tests; bus `sandbox.rs`, module Plan.
**Owner prerequisites:** TinyBox #27 real jail/namespace/microvm/Docker contracts.

1. RED SaaS + env off, source tiers, agent mode/config precedence, unsupported
   capabilities, Noop for untrusted code, credential grants, host networking.
2. Extend request with verified SaaS/env/mode facts and policy scope; resolve
   backend, resources, network and grants deterministically. Credentials never
   enter grants. Actual executor suitability still checked immediately at spawn.
3. SaaS always isolates; env off never overrides. Untrusted MCP/skills/downloads
   select microvm where available, otherwise an explicitly suitable real backend;
   unavailable suitable isolation denies. Host-network needs approval.
4. Implement Firejail/Bubblewrap config via supported namespace mapping or migrate
   to namespace and remove obsolete firejail_args/empty resource config. Every
   surviving resource field must map to an enforced TinyBox limit.
**Exit:** plan matrix and native Plan tests pass, no optimistic unsupported plan.

## Task 10 — Unified crypto, keyring and pairing

**Owner files:** new `crates/tinysecurity-crypto/{Cargo.toml,src/lib.rs}`, internal
`src/{password,keyring,device,pairing}.rs`, platform backend submodules, tests;
bus `crypto.rs` and constants; module dispatch and KeyringConsent callbacks.
**Host sources:** `security/encryption/core.rs`, `security/keyring/{crypto,
encrypted_store,encrypted_file_backend,backend}.rs`, `security/devices/crypto.rs`,
`security/pairing.rs` and existing encrypted fixture files.

1. RED decrypt existing Argon2id/AES-GCM and encrypted-file fixtures; malformed
   ciphertext/version/tag, wrong password, X25519/HKDF tunnel compatibility,
   pairing TTL/replay/rate limits, keychain consent deny/unavailable and restart.
2. Port implementation once; native backend owns OS keyring/encrypted fallback.
   Credential ownership/auth, device sockets, pairing UI and consent UX stay host.
   Opaque key/session handles bind authenticated owner; never return key material
   through info/log/audit. Define explicit binary payload size limits.
3. Validate native Secret Service/file, macOS Keychain and Windows Credential
   Manager round trips; remove test fixtures from OS keyrings after the test.
   Document genuinely unsupported runner consent setup, do not simulate OS tests.
**Exit:** existing data readable, bus crypto complete, real platform backends tested.

## Task 11 — Owner quality, review, single PR and release

**Files:** owner workflows `ci.yml`, `release.yml`, native test matrix, fuzz targets,
`deny.toml`, docs/performance, MODULE/specs/ADRs/ROADMAP; no manual version bump.

1. All new members must be native-dispatched, listed in PolicyInfo and covered by
   malformed/timeout/fault tests. Fuzz commands/paths/redactor/URL; per-file line
   coverage ≥90%; cargo deny, MSRV and warning-free rustdoc.
2. Full fmt/clippy/build/test all-features and feature-off checks. Benchmark
   Evaluate/Check p50/p99 against committed budget; batching/cache fixes remain
   bus-based and cannot cache rates/grants/DNS authorization.
3. Real Linux/macOS/Windows CI proves loading/digests, approvals/restart, policy,
   URL pinning, audit permissions, sandbox plans and keyring. Release matrix
   produces every supported artifact (11 registry platform keys) and checksum.
4. Broad spec/code review, address feedback, merge ONE remaining TinySecurity PR,
   dispatch semantic release workflow. Verify published artifacts by real loader
   allow/deny/callback flow. Only now can production host pins change.

**Upstream checkpoint (2026-10-23):** TinySecurity PR #42 (redaction) merged as
`0990d97159ddb12a214749d6aa03a4fbf022779f` and is included in the published
v0.17.1 release (`b62ad69ce28bc3178ad91e12f3074f21007055c4`). The host's
registry version and checksums already pin v0.17.1, so the short-key redaction
parity gap recorded under Task 12 is resolved by the released module. PR #43
([abandoned execution outcome](https://github.com/tinyhumansai/tinysecurity/pull/43))
adds the incompatible `ExecutionOutcome::Abandoned` bus variant and bumps the
contract to 2.0. Its CI exposed and fixed an old-major reinit fixture and a
Windows native verifier assumption. The currently open review requests are
addressed, but CodeRabbit's changes-requested verdict remains pending a fresh
review. PR #43 is a focused slice, not the cumulative owner migration required
by this task; Tasks 1–10 remain prerequisites before the next production host
pin changes.

**Runtime secret scopes (2026-10-23):** TinySecurity PR #47 is stacked on #46
and adds host-only runtime secret registration for live approval scopes. Review
exposed that a revoked native registration ID could be re-accepted after an
approval generation replacement when the policy reference stayed the same. A
regression first failed against that path; revoked IDs are now tombstoned for
the native registry lifetime, while the trusted static-scope replacement path
can reactivate configured scopes after fencing old handles. Native registry
tests (29), the module lifecycle regression, strict crypto/module Clippy and
format checks pass locally. The refreshed Linux/macOS/Windows owner checks are
running. OpenHuman remains pinned to released v0.17.1 until the owner change is
merged and released.

## Task 12 — Host pins and config/command/redaction migration

**Files:** registry security record, `scripts/ci/check-module-pins.mjs`,
`modules/security{,_config,_host}.rs`, `security/live_policy.rs`, schema/migrations,
`security/policy/`, network/fs/shell callers and redaction consumers.

1. Root pins released owner gitlink/version/checksums together and validates
   monotonic/pin/feature/bus-only checks; no local artifact digest admission.
2. Implement complete scoped translator and client; bind immutable policy IDs
   to host verified context. Tenant settings are never process-global init policy.
3. Route tools/in-tool checks to coarse Evaluate/batched Check and registered
   path scopes. Cache only explicitly cacheable static decisions by generation,
   complete fingerprint and verified caller scope; invalidate on activation.
4. Replace host redactors at util/redact, approval/redact, security/core/scrub/pii,
   core/log_redaction, registry/denials, Composio redact, credential_scrub and URL
   helpers. Port characterization tests, delete duplicated implementations only
   after native parity; suppress sensitive logging while module unavailable.
5. Install pinned URL transport/egress/scans at every Task 5 consumer and
   agent/bus.rs, session_host/runtime/run_loop.rs, mcp/registry. Verify secret
   suppression in Sentry/logs/approval/audit and real redirect/rebinding denial.

**Host follow-up (2026-10-22):** storage open and workspace-rebind logs use
TinySecurity `EndpointOrigin`, so database names and local SQLite paths do not
reach logs or the `Rebound` display fields. Inference URL migration logs use the
same mode; regressions assert paths and credentials stay out of captured logs.
Config snapshot URL suppression is covered under Task 2.
RPC dispatch parameter logs now use TinySecurity `StructuredLogArguments`;
unavailable or malformed responses suppress the complete parameter value.
`core::rpc_log` regression tests cover valid JSON projection and suppression.
Flow memory writes now route both agent-tool and TinyFlows node guards through
the released module's `CredentialsOnly` redactor and fail closed when it cannot
answer. The existing `api_key=abc123` characterization exposed a short-value
parity gap in v0.17.0. TinySecurity PR [#42](https://github.com/tinyhumansai/tinysecurity/pull/42)
merged upstream at `0990d97159ddb12a214749d6aa03a4fbf022779f`, adding the short-key
case while preserving source identifiers. A release containing that merge is
still required before the host integration is publishable. The host guard tests
and no-security-module RPC-log suppression test pass locally;
the flow-node short-key test remains red against the currently pinned archive.
Network destination gates also deny when `security-module` is compiled out;
regressions cover both direct and routed gates, and the enabled native gate
suite passes.
Composio result and error handling now sends every distinct string (including
object keys and strings without the configured Composio credential) through the
shared redactor. It withholds results on unavailable, incomplete, oversized or
credential-preserving answers; batches are bounded to 256 strings, 512 KiB and
15 seconds. Regressions cover unrelated bearer credentials, incomplete module
answers and the size limit.
Tool-result credential scrubbing also withholds when TinySecurity is not built
or armed; the diagnostic floor no longer authorizes unverified model-visible
tool output. A feature-off regression first failed by returning the unchanged
result, then passed after removing the harness scrubber fallback.
The synchronous `Text` redactor now suppresses the complete value when the
module cannot answer. Its consumers persist artifact bodies or put failure text
back into model context, so the log-only floor no longer substitutes for the
module there; the feature-off regression pins the suppression marker.
The skill installer now routes URL diagnostics through TinySecurity's
`Endpoint` mode, retaining safe paths while stripping credentials, queries and
fragments. Startup path logging also uses the shared `Text` facade instead of a
host-owned home-prefix redactor: it retains the conventional `~/...` display
when the released module answers and suppresses the full path while unavailable.
The unavailable-module regression failed against the old helper, then passed;
the released-archive redaction test passes for the module-backed path.

## Task 13 — Host approval adapter, middleware and frontend

**Files:** `security/approval/{gate,gate_intercept,store,schemas,rpc}.rs` and
related fragments; `agent/tinyagents/middleware/tinysecurity.rs`, middleware.rs,
`tools/agent_policy/`, `agent/tool_policy.rs`, middleware/{approval,tool_policy}.rs;
`app/src/components/settings/panels/ApprovalHistoryPanel.tsx`, locale resources,
`app/test/e2e/specs/security-approval.spec.ts`.

1. Keep approval RPC/event compatibility; store/prompt facades serve scoped
   callbacks. Module owns transitions/expiry/flow trust/log, host parks/resumes.
   Preserve cancellation and exactly-one execution outcome; no auto_approve bypass.
2. Register one TinySecurityMiddleware before effect-capable middleware. Map
   module decisions to tinyagents PolicyDecision, bind recorded tool/rule inputs.
   Delete duplicate agent_policy/ToolPolicyPosture wrappers and three disallowed
   checks. Keep declarative ToolRules and one scope/dispatch adapter.
3. UI displays auto:jev verdicts/scores and pending/expiry state with shadcn
   primitives, useT and real translations. Cover prompt/decide/history/badge via
   mocked desktop E2E helpers and existing approval component/API suites.
4. Run i18n:check, i18n:english:check and i18n coverage; no user text/IDs in analytics.

**SaaS approval progress (2026-10-22):** A routable, authenticated WebChat call
that falls outside the deployment's tool allowlist now continues through the
tenant-scoped approval engine and existing `ApprovalRequested` surface. Its
approval is one-time and cannot be auto-approved by profile posture or durable
grants. Allowlisted tools retain their audited automatic path; non-chat SaaS
origins remain denied. Durable tool-grant list/revoke, the localized history
panel, and judged-verdict scores/expiry are implemented; component tests cover
those states, and a Playwright browser test now exercises judged history and
grant revocation through mocked core RPC. The targeted SaaS gate/profile tests
pass. The existing interactive browser harness covers the approval prompt,
approve-once execution and denial without execution
(`app/test/e2e/specs/agent-harness-behaviors.spec.ts`). Task 13's
single-middleware consolidation remains open. The unused OpenHuman
`ToolPolicyMiddleware` wrapper and trait implementation are removed; the
security gate now calls the session policy adapters directly, and their tests
exercise the session host state without constructing a second middleware.
Builder policy decisions are now folded into `TinySecurityPolicy` as another
source, removing the separate `CombinedPolicy` wrapper while retaining the
shared approval resolver. The SDK's `ToolPolicyMiddleware(require_sandbox)`
has now been removed from both the normal and live harnesses. Its declared
`SandboxMode::Required` check is folded into the shared TinySecurity gate as a
host adapter, with tests for denial without a sandboxed workspace, allowance
with one, and tools that do not require a sandbox. The actual process plan and
execution remain owned by TinySecurity's released `Plan2` contract and TinyBox;
the adapter only enforces the tool declaration against the run workspace.
Remaining Task 13 work is the audit of the remaining posture and deny-list
surfaces plus the full approval UI and cross-path verification.

## Task 14 — Every untrusted process uses approved TinyBox execution

**Host files:** sandbox/{ops,types,grants,docker}.rs,
`cron/scheduler/shell_job.rs`, `hooks/host.rs`,
`tools/impl/system/install_tool.rs`, MCP and flow host adapters.
**Owner seams:** TinyMCP transport/stdio/mod.rs; TinyFlows caps/host/script/runner.rs;
TinyComputer browser process startup/egress contracts.

1. Replace resolve_* with module Plan→TinyBox BoxSpec/Placement conversion.
   Real `SandboxCapabilities::is_suitable_for_untrusted_code()` gates spawn;
   unsupported or Inactive is denial for untrusted tiers. Replace handwritten
   DockerRuntime docker run with tinybox_docker::OneShot. Keep execution host-side.
2. Maintain a committed spawner inventory from Command::new/process spawn search.
   Each cron/MCP stdio/runtime worker+pyserver+compression/command hook/flow script/
   installer/downloaded skill/browser process row names source trust, plan,
   executor and test. Launch probes must not become execution bypasses.
3. Where owner lacks process-launch injection, add owner seam and upstream tests,
   merge/release/pin first. No direct stdio/worker spawn bypass in composed product.
4. Real tests attempt protected file access/network from each untrusted family;
   confirm denial inside the actual OS jail/namespace/AppContainer/Seatbelt.
   KVM job tests microvm; Docker integration tests verify grants and network.
   SaaS+env off ordering/boot_guard stays pinned. Explicit unsupported platforms
   deny instead of silently skipping product behavior.

**TinyBox dependency status (2026-10-22):** upstream PR
[#100](https://github.com/tinyhumansai/tinybox/pull/100) merged at
`83ca264fbc41d603119e58e162cd990a961c5621`, removing TinyBox-owned command
classification from contract 2.0. OpenHuman no longer imports that removed
command-policy API, but TinyBox v0.2.0 has not been released and OpenHuman still
pins the released v0.1.42 / contract 1.22 module. Do not update the host pin or
registry until the owner publishes the new artifact and checksum.
[#103](https://github.com/tinyhumansai/tinybox/pull/103) remains draft and adds
Linux native resource limits; it is only part of the TinyBox #27 work and does
not complete cross-platform jail, microVM, Docker, or process-launch coverage.
TinyBox PR [#112](https://github.com/tinyhumansai/tinybox/pull/112) has since
merged as `b0b5969ab6d04b64c478e1f012b6af2ac53fc171`, bringing canonical TSD v2
storage to main. PR #103 was refreshed by merging current TinyBox main into its
branch; its current head is `98b541c7fb737ee546c6943f524f39823941a058`. The
refresh also updates its TinyBus dependency and preserves the PR history. The
exact-head CI run `39889111817` now passes formatting, Clippy, docs, MSRV,
supply-chain, tests, coverage, all three platform feature/module suites, and
`native-resource-live`. The full workspace test suite, strict Clippy, formatting,
locked metadata and diff checks also pass locally. Keep the PR in draft until a
compatible TinyBox release is available and OpenHuman's host adapter is updated.
The first post-merge CI run passed all 51 native jail unit tests but failed the
separate resource cancellation proof because it constructed production storage
without a served host callback. The test now uses TinyBox's testkit-backed
`Resources::test_resources()` fixture (`586e883bf81bda850c5358de4c2755cc77413736`).
That run exposed that the test's current-thread Tokio runtime cannot service the
testkit's synchronous storage calls. Commit `5110ce7693f718441a652f3ffd28d90ffb9b3232`
uses a two-worker runtime for this storage-backed test; the focused test passes
locally. The current exact-head run passes `native-resource-live`, module tests,
coverage, Clippy, formatting, MSRV, supply-chain and Linux/macOS/Windows module
E2E checks. Keep PR #103 in draft until a compatible TinyBox release is available
and OpenHuman's host adapter is updated.
The spawner inventory distinguishes `sandbox/ops.rs`'s module-planned operator
host-run mode from an untrusted direct bypass: the module registers that source
`TrustedHost`, while scheduled/hook text has a separate `UntrustedSkill` source
that cannot receive a host-run plan. `install_tool` currently returns an
unavailable result after its access and approval gates and contains no process
spawn; the sandbox README now records that accurately. Installing packages
remains an unimplemented capability until a safe scoped installer can run through
the TinySecurity plan and TinyBox executor.

**Process inventory scope check (2026-10-23):** rechecked the current tracked
tree and Cargo manifests. This checkout has no `tinyruntime` submodule or
dependency, `runtime/python_server/{server,kompress}.rs`, `runtime/pool/`, or
`agent/host_runtime.rs`; those paths from the original task list are not current
OpenHuman spawn sites. TinyFlows' direct `ProcessCodeRunner` and
`ProcessShellRunner` are behind its `host-caps` feature, which OpenHuman does not
enable. The linked module's `CapsHostPorts` instead delegates code execution to
OpenHuman's `OpenHumanCode`, which runs through `sandbox::untrusted::run` and is
covered by the committed spawner inventory. TinyComputer's worker and browser
processes remain module-owned and require the owner-side acceptance tracked below.

## Task 15 — Host audit, scan/consequence gates and crypto facades

**Files:** shell audit consumers, `mcp/audit/`, security encryption/keyring/device/
pairing adapters, web3/seams.rs, x402/seams.rs, TinyComputer/TinySkills adapters.

1. Route policy/approval/judge/shell/sandbox changes into one module audit. MCP
   protocol telemetry remains TinyMCP-owned; policy events forward to same audit
   stream. Record this ownership in ADR, remove duplicate policy logs.
2. Feed TinyComputer consequence/payment gates and TinySkills scan verdicts into
   policy/approval before execution. Add upstream callback contracts where needed;
   no documented bypass can substitute for a missing enforcement seam.
3. Replace crypto/keyring/pairing implementations with bus facades; preserve
   encryption.* and web3/x402 consumers. Port all fixture/migration tests before
   deleting old code; device networking/credentials/consent surface remain host.
4. Remove obsolete aliases, unused redaction/PII helpers, unused_import allowances,
   config fields and old tests only together with passing replacement coverage.

**Keyring and legacy import audit (2026-10-23):** `OsMasterKeySource` treats
only `NoEntry` as absence; denied and platform failures return errors and do not
mint or fall back to a workspace key. The OS availability probe fails closed
when a write is denied and retries after its backend is replaced. Legacy
`dev-keychain.json` data is retained when invalid, and is removed only after
the encrypted copy is durable and verified. Focused suites pass: keyring
backend 9/9, master-key source 7/7, and encrypted-file migration 29/29. The
generic OS backend's historical `Unavailable` read/delete mapping remains
unchanged; this audit found no master-key replacement or legacy-import data-loss
path through it.

**TinySkills scan policy contract (2026-10-23):** TinySecurity PR
[#48](https://github.com/tinyhumansai/tinysecurity/pull/48) adds the trusted
host-derived `SkillScan` verdict and bundle digest to contract 1.29. It binds
the scan identity into approval evaluation, makes `Warn` require a fresh human
approval even under broad grants or auto-approval, and makes `Block` a hard
denial. The owner PR's local bus/policy/approval/module checks pass; Linux,
macOS, Windows and release-latency CI are running. OpenHuman's runtime adapter
is in progress on draft PR #7331: the workflow registry now returns the
discovered bundle beside its runnable definition, the host hashes and scans the
prompt plus bounded resources, and `run_workflow` sends a host-derived
`SkillScan` check with an Execute effect check through TinySecurity. Scan
permits are one-use and bound to the workflow id, inputs, tool call id and
bundle digest; the spawn boundary re-scans after preflight and rejects changed
or blocked content. Both direct skills controllers dispatch `run_workflow`
through the native policy and approval middleware. The released registry pin
remains unchanged until the TinySecurity 1.29 owner change is merged and
released. The broader Task 15 audit and final cross-platform/owner-release
verification remain open.

**TinyComputer progress (2026-10-22):** upstream PR [#152](https://github.com/tinyhumansai/tinycomputer/pull/152)
merged at `82d0aa6`. Its live Chromium test exercises the TinyComputer module's
host authorization callback and verifies that changed policy produces no new
socket to either the old allowed listener or the denied rebound listener. This
does not supply TinyComputer consequence/payment gates or prove native OS process
confinement; both remain open under Tasks 14–15.

**TinyComputer transport follow-up (2026-10-23):** PR [#158](https://github.com/tinyhumansai/tinycomputer/pull/158)
merged at `6b0a3c8`. Its live Chromium test confirms the browser transport dials
only the exact host-authorized socket, even when a denied host shares the same
port. All module CI checks passed. This is transport evidence only; TinyComputer
#111 remains open for controlled DNS rebinding, TinySecurity-backed authority,
worker/direct-socket confinement and cross-platform acceptance.

**TinyComputer handshake follow-up (2026-10-23):** upstream PR
[#159](https://github.com/tinyhumansai/tinycomputer/pull/159) adds focused
transport-class coverage for the `/socket` WebSocket upgrade headers. The
current head passes the module test, coverage, formatting, Clippy, docs, MSRV,
supply-chain and Ubuntu feature-test jobs. Windows/macOS feature tests and
cross-platform module E2E jobs are still running. This test improvement does
not close TinyComputer #111 or replace its controlled-DNS and native
confinement acceptance.

The consequence gate still lacks a trusted host caller context: the module
callback knows the paused task and action, but not the tenant-bound dispatch
context that initiated `ContinueTask`. TinyComputer PR
[#153](https://github.com/tinyhumansai/tinycomputer/pull/153) adds an opaque,
redacted `HostContext` field to `ContinueTask`, relays it to
`ConsequenceHost.Review`, and bumps the wire contract to 2.27. It merged to
canonical `main` at `e52203cc`, but the latest published release is still
v0.15.0 and predates the contract change. OpenHuman must wait for a release
containing it before adding the host adapter; the adapter must validate and
consume the short-lived host-issued context, bind the exact task/action, and
fail closed on missing, stale, mismatched or reused context.

**TinySecurity runtime secret-scope lifecycle review (2026-10-23):** TinySecurity
PR [#47](https://github.com/tinyhumansai/tinysecurity/pull/47) now serializes
`RegisterSecretScope2`, approval-scope replacement, and retirement through one
module lifecycle lock. Secret registrations are retired only after the approval
engine accepts the replacement or retirement, so rejected changes preserve the
live resolver. Regression coverage exercises invalid replacement, initialized
scope retirement followed by valid replacement, and queued registration during
replacement. The owner branch is pushed at `3eb7e6a`; focused module tests and
all-target Clippy pass locally. Cross-platform CI and a fresh review are
pending. OpenHuman remains pinned to released TinySecurity `c18b56f` until the
owner change is merged and released.

## Task 16 — Final matrix, documentation and completion evidence

**Files:** `.github/workflows/{security-native,ci-full,test-reusable}.yml`,
`scripts/ci/security-native-fixture.sh`, dependency/pin checks; AGENTS ownership
(CLAUDE symlink), `gitbooks/developing/loadable-modules.md`,
`gitbooks/features/{approval-gate,privacy-and-security}.md`, security/approval/
sandbox README files, `platform/about_app/`; generated docs via scripts only.

1. Native host matrix loads RELEASED module, tests tampered digest, fault/timeout
   latched deny with zero retry, two-tenant isolation, approval RPC/event/restart,
   judge stub, OS command/path semantics, pinned URL transport, audit permissions,
   crypto/keyring and actual sandbox effects. No mocked native module or isolation.
2. Run pnpm rust:layout; feature forwarding, module pin/monotonic and bus-only
   checks; product-feature enabled/disabled builds; targeted Rust/frontend/E2E
   suites, changed-line coverage ≥80%. Owner per-file requirement remains ≥90%.
3. Write GitBook/docs accurately, including real privacy/SaaS/judge/fault behavior,
   approved release count/builds, callback ownership and isolation limitations.
   Run pnpm docs:generate and pnpm docs:check. Review entire branch, then independent
   completion verification of all claimed checks. Keep PR #7331 ready for review.
4. Trackers close only when all engines, released artifacts, migrated consumers,
   every spawner row, deleted duplicates, cross-OS tests and docs are evidenced.
   Any absent backend/test/release remains an explicit open task, never DONE.
