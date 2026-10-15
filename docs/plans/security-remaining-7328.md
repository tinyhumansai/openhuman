# Complete TinySecurity migration: remaining implementation

Trackers: [OpenHuman #7328](https://github.com/tinyhumansai/openhuman/issues/7328),
[TinySecurity #1](https://github.com/tinyhumansai/tinysecurity/issues/1),
[TinyBox #27](https://github.com/tinyhumansai/tinybox/issues/27).
Binding owner design: `vendor/tinysecurity/docs/specs/security-module.md`,
`docs/specs/immutable-path-scopes.md`, `docs/plans/security-module.md`.

## Delivery boundary and starting evidence

Bootstrap plus immutable path scopes exist. TinySecurity #4 merged at
`f2b7ebd77c1f1109e08f6f38de87ad594841a0e3`. The module currently advertises
Evaluate, Check, PolicyInfo and four immutable path members. Its command engine
only allows argument-free diagnostics. `future.rs` contains reserved payloads,
not working approvals, scans, egress, audit or sandbox engines. The host's
`modules/security.rs::module_config` deliberately returns bootstrap defaults.
Do not treat any reserved type or passing bootstrap test as tracker completion.
Root is releasing that path milestone as v0.2.3 (run 38080694935). The next owner
implementer is already executing Task 3's immutable command registry. Finish and
review that task first; do not restart it or wait for the full migration plan.

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

**Host status (tinysecurity v0.4.0 repinned):** done: `[security]` table
(`config/schema/security.rs`, audit and approval TTL wired from it, folded
`[security.autonomy|sandbox|privacy]` overlay), migration 14->15
(`security_policy.rs`, `max_cost_per_day_cents` deleted with a notice), single
translator (`modules/security_policy.rs`), scoped client with generation-keyed
decision cache (`modules/security_scoped.rs`), `live_policy::reload_from_config`
driver, `SecurityPolicy::from_full_config` (privacy no longer hard-coded), the
trusted-host binding in `module_config`, and a released-module two-tenant test.
Open: routing shell/filesystem/network gates through the client and deleting the
`tinybox_core::shell` calls in `command_checks.rs` (needs `security-module`
default-on and a sync-to-async seam), a `PolicyChanged` signal subscriber,
scoped audited variants, and physically moving `[autonomy]/[sandbox]/[privacy]`
under `[security]` in `Config`.

## Task 3 — Command grammar, tool rules and action accounting

**In progress:** root has dispatched this task; resume its result/review instead
of dispatching a second implementer. The agreed contract is immutable
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
**Exit:** complete command/rule/rate parity through native Evaluate/Check.

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

## Task 14 — Every untrusted process uses approved TinyBox execution

**Host files:** sandbox/{ops,types,grants,docker}.rs,
`agent/host_runtime.rs`, `cron/scheduler/shell_job.rs`, `hooks/host.rs`,
`runtime/python_server/{server,kompress}.rs`, `runtime/pool/`,
`tools/impl/system/install_tool.rs`, MCP/runtime/flow host adapters.
**Owner seams:** TinyMCP transport/stdio/mod.rs; TinyRuntime pool/worker.rs and
tinyruntime-pyserver/src/server.rs; TinyFlows caps/host/script/runner.rs;
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
