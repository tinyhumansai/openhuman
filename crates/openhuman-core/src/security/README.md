# security

The trust boundary of the core. This folder decides whether an agent may run a
command or touch a path, parks tool calls that need a human yes or no, stores
credentials and secrets, scrubs secrets and PII out of anything persisted, and
guards the RPC listener when it binds beyond loopback. `security/mod.rs` calls
it the kernel security family. The `security-module` feature moves asynchronous
filesystem authorization into the separately loaded TinySecurity native module.

## Native filesystem policy

`security-module` is a default feature of the core, embed, CLI and TUI. Unit
tests (`cfg(test)`) keep `validate_path` on the in-process evaluator so they
need no downloaded module; the native route is covered by
`path_checks_native_tests.rs` and the released-archive fixtures
(`OPENHUMAN_TEST_SECURITY_RELEASE_HOST`). The scoped decision cache is flushed
by the module generation on every response and by the `PolicyChanged` signal
(`modules/security_watch.rs`).

With `security-module`, `SecurityPolicy::validate_path` and
`validate_parent_path` use the typed host client in `modules/security.rs`.
`policy/native_paths.rs` translates trusted roots, the acting directory,
internal-state reservations into an immutable scope. Scope registration deduplicates the whole policy; one agent
cannot replace another agent's settings through global module reinitialization.

The host links only `tinysecurity-bus`. The engine canonicalizes paths and
checks read/write grants in the native module. A missing module, an untrusted
recipient, a timeout, or a malformed response denies access without a local
fallback; with the feature off `validate_path` and `validate_parent_path` deny.
The host has no lexical path preflight of its own: the batched
`SecurityPolicy::check_paths` (module `CheckPaths2`, one call per batch) and the
scope-free `floor.rs` queries (`ProtectedPath2`) both go to the module, and
the `*_blocking` variants cross the shared sync-to-async seam for tool traits
that cannot be `async`. Unit tests install a scripted `GapPort`
(`modules/security/gap`, `StubPort`) for the current thread; there is no local
policy to fall back on.

Production admission pins TinySecurity v0.17.1 and its published archive checksums
in the module registry. `scripts/ci/security-native-fixture.sh` loads those
released archives and invokes their admission, path, and latency tests. Explicit
local fixtures remain available for development; their digests never become
production release pins. Shell policy, approvals and redaction run on the
module; crypto continues to use its existing host engines until its own
migration phase is implemented and verified.

Host latency budget (committed). The host path-policy gate runs
`OPENHUMAN_SECURITY_PERF_BUDGET=1 scripts/ci/security-native-fixture.sh` (a
`--release` build against the released v0.9.0 archive, warmed scope
registration, 200 turns of 5 checks: 3 reads, 1 parent-create, 1 denied probe).
Measured on Linux 7.0 x86_64, Rust 1.99.0, 2026-10-15, three release runs:
per call p50 81-90 us, p99 116-185 us; per turn (5 calls) p50 376-423 us, p99
493-1568 us (p99 varies run to run). The enforced budget is the owner's p50 <=
1 ms / p99 <= 10 ms per call (`security_native_tests.rs`), at least 11x and 54x
headroom over the worst run, for noisy hosted runners. It is deliberately not
tightened to the measurement; the numbers are committed here as the reference. The
`latency-budget` job in `.github/workflows/security-native.yml` records each
run's JSON as an artifact; tighten the constants if a controlled runner is added.

Crypto (contract 1.28): `modules/security_crypto.rs` calls the confidential
`PasswordCrypt2`, `KeyringAccess2` and `PairingControl2` members, and
`modules/security_host.rs` serves the `KeyringConsent2` callback from the
existing keychain consent policy (`keyring_consent::policy::check_secret_access`;
the prompt UX stays host-side). `modules/fixtures/legacy_password_payload.json`
pins the legacy Argon2id/AES-GCM payload the module must keep opening. The host
`encryption/core.rs` is deleted and password crypt is module-owned. Production
`CryptoConfig2` keeps keyring and pairing disabled because current production
callers still use the host stores and device-pairing implementation. TinySecurity
now provides `DevicePairing2`, `Device2` and `DeviceFrame2`; OpenHuman has not yet
migrated `security/devices/` to those contracts. The remaining host crypto and
keyring migration is tracked in issue #7328 phase 8.

Callers are spread across the core. Every acting tool consults
`SecurityPolicy` before it runs, the agent harness asks the approval gate
before a prompted call, the credentials domain owns the `auth.*` RPC
namespace, and the RPC server asks [`pairing.rs`](./pairing.rs) whether a bind needs a token.

## How it works

### The policy and its off switch

`SecurityPolicy` ([`policy/types.rs`](./policy/types.rs)) is the value every tool receives. It is
built once per agent session by `SecurityPolicy::from_config`
([`policy/enforcement.rs`](./policy/enforcement.rs)) from the `[security.autonomy]` config block, the workspace dir
and the action dir. That constructor is the single chokepoint between config
and policy, so it also injects the default trusted roots: the projects home
(`~/OpenHuman/projects`) as read-write, the configured `action_dir` as
read-write (skipped when the action dir sits at or above `workspace_dir`), the
tool-result artifacts directory as read-only, and the namespaced scratch dir
(`/tmp/openhuman`, never `/tmp` itself) as read-write.

The policy is off by default. `AutonomyConfig::enabled`
(`config/schema/autonomy.rs`) defaults to `false`, and `from_config` copies it
onto `SecurityPolicy::enabled`. Agents here run inside containers, platform
jails and Docker sandboxes that already give the isolation this in-process
policy approximated, and a shell tool that refuses ordinary shell syntax is
not a usable shell. `from_config` logs a line at info level when it builds a
disabled policy.

Every enforcement entry point checks the flag first and returns the permissive
answer when it is off:

| Function | File | Disabled result |
| --- | --- | --- |
| `check_command` | [`policy/native_commands.rs`](./policy/native_commands.rs) | the module answers with the policy disabled: `Allow`, no structural guard, no allowlist; the protected-path floor still refuses |
| `spend_action`, `tier_allows_acting_blocking` | [`policy/module_gate.rs`](./policy/module_gate.rs) | the module answers with the policy disabled: no tier refusal, no budget |
| `check_paths`, `path_allowed`, `validate_path` | `policy/module_gate.rs`, [`policy/path_checks.rs`](./policy/path_checks.rs) | the module answers with the policy disabled: allowed after the floor check |

So with the policy disabled, command classification still runs in the security
module (callers get a `CommandClass` back) but nothing is gated on it: no approval prompt, no
command allowlist, no hourly action budget, and no `workspace_only`,
`forbidden_paths`, trusted-root or workspace-internal containment.

A floor survives the off switch. The module applies it before the `enabled`
test and it holds in every configuration. The host asks it through
[`floor.rs`](./floor.rs) (`is_protected`, `any_protected`, `protected_paths`,
`is_protected_blocking`; module `ProtectedPath2`), which needs no
`SecurityPolicy` and answers "protected" when the module cannot be asked.
`floor()` (`ProtectedFloor2`) returns the vocabulary for components that apply
it themselves; `sandbox/local_grants.rs` uses it to filter TinyBox jail grants.

- The protected-path floor. Credential stores are
  matched by path segment, case-insensitively: `.ssh`, `.gnupg`, `.aws`,
  `.azure`, `.kube`, `keychains`, and the Windows pairs
  `Microsoft\{Protect,Credentials,Crypto,Vault}`. System roots are matched by
  absolute prefix: `/etc`, `/root`, `/boot`, `/proc`, `/sys`, `/system`,
  `C:\Windows`, `C:\Program Files`, `C:\Program Files (x86)`,
  `C:\ProgramData`. A trusted-root grant cannot reach these either, and the
  per-turn workspace grant is checked after this test, so it cannot either.
- In the module's string path check (`PathKind::StringAllowed`): a null byte, a
  `..` path component, and URL-encoded traversal (`..%2f`, `%2f..`) are
  rejected. A bare relative name that matches one of the reserved workspace
  files (`WORKSPACE_INTERNAL_FILES` in `policy/types.rs`, such as `.env`) is
  also rejected.

`SecurityPolicy::default()` is the opposite of the shipped default: it sets
`enabled: true`. It is the fallback for a policy built with no config at all
(tests, examples, a few internal call sites), where failing closed is right.
Only `from_config` carries the shipped `false`.

### With the policy enabled

Set `[security.autonomy] enabled = true` and the full policy applies. Commands are
classified, then gated by tier:

```text
 shell command string
        |
        v
 SecurityPolicy::check_command    one CheckCommand bus call to the TinySecurity
        |                         module, which blanks quoted heredoc bodies,
        |                         classifies (highest class across ; | && ||
        |                         segments, redirect or tee lifts to >= Write),
        |                         raises the class with the model's declared
        |                         category (never lowers it), and applies the
        |                         autonomy tier, hidden-execution defence and
        |                         protected-path floor
        v
 CommandGate
        |
   +----+--------------+-------------------+
   v                   v                   v
 Allow               Prompt              Refusal
 run the tool        ApprovalGate park   POLICY_BLOCKED_MARKER message
                     (10 min TTL)        (or "Rate limit exceeded")
```

The host asks twice and the module counts once. The harness gate asks before
the approval prompt as a `Check::HostShell` admission under the scope that carries
`max_actions_per_hour` (it reads the budget and never spends it), and the tool
asks again immediately before it spawns (`CommandStage::Execute`, the same
scope), so an allowed run spends one action of the module's hourly budget and a
prompt that is declined spends none. Acting tools spend through the same
ledger, once per acting call, at the tool gate (`Check::HostAction { spend }`, `ToolPolicyGateMiddleware` in
`agent/tinyagents/middleware/security_gate.rs`), so shell runs and acting tools
share one budget. The in-tool budget checks are gone; callers that are not
tools (MCP server dispatch, `mcp_ui`, the workflow `http_request` node) spend
through `SecurityPolicy::spend_action` (`SpendAction2`). Direct tool runs
(`tools::native_ops::execute_tool`: JS and RPC tool calls, workflow `tool_call`
nodes) go through the same gate, so a refused call never executes. The
scheduler asks under the legacy rule (`CommandFlow::Scheduled`: the curated
allowlist plus the configured risk restrictions) when a job is created or
edited, and again when it runs. A module that is missing, untrusted, faulted
or slow refuses the command (`Refusal::Unavailable`); there is no host
classifier to fall back on. The `CommandGateway` seam on the policy is sealed to
this crate and exists for unit tests of what a caller does with an answer.

One budget applies to a shell run or an acting tool: the module's hourly ledger
(keyed by the policy value, so sessions with identical settings share it),
spent by the `Execute`-stage check. The host keeps no counter of its own.

The tier matrix (the module's; flow nodes, the approval manifest and the
network and filesystem gates all ask it through `command_class_gate`, and a
module-backed test pins it to an inlined table): `ReadOnly` allows only `Read` and blocks
everything else; `Supervised` allows `Read` and prompts on every acting class;
`Full` runs `Read` and `Write` silently and prompts on `Network`, `Install` and
`Destructive`.

The invariants that hold with the policy enabled:

- `action_dir` is the agent's permitted read and write root. Tools resolve
  relative paths and default their working directory there, and `from_config`
  grants it as a `ReadWrite` trusted root so a non-default working folder is
  writable. The grant is skipped when the action dir is an ancestor of
  `workspace_dir`, because a trusted root overrides `forbidden_paths` and a
  working directory should not buy that over the whole workspace.
- `workspace_dir` stores internal state and is never an acting-tool target.
  The module's internal-state check (`is_internal`, which fails closed) covers
  `WORKSPACE_INTERNAL_DIRS`,
  `WORKSPACE_INTERNAL_FILES`, the `memory-`, `memory_tree-` and `session_raw-`
  prefixes, per-artifact directories under `artifacts/` (but not
  `artifacts/tool-results/`), and the account `config.toml` beside the
  workspace. A trusted-root grant does not weaken this.
- Unknown commands classify as writes. The module's classifier is fail-closed:
  a command that is not provably read-only is at least `CommandClass::Write`.
  The body of a quoted heredoc (`<< 'EOF' ... EOF`) is blanked first, since the
  shell expands nothing there. An unquoted delimiter (`<< EOF`) is expanded
  and is still scanned.
- Outside `Full`, the module rejects hidden execution (command or process
  substitution, backticks, background `&`) that could smuggle a command past
  the approval the human read (`Refusal::Hidden`).
- Interactive approval requests expire as denied after ten minutes
  (`DEFAULT_APPROVAL_TTL` in [`approval/gate.rs`](./approval/gate.rs); the Flow Canvas copilot path
  uses a three-minute window).

The TinySecurity module owns shell-string scanning, classification, the
allowlist, the risk score and the protected-path floor. [`policy/native_commands.rs`](./policy/native_commands.rs)
translates the host's settings into the module's immutable `CommandPolicy` and
projects its content-free answer back into `GateDecision`, `CommandClass` and
the existing policy-blocked messages. Refusal text is fixed per reason; it
never repeats the command, whose arguments can carry secrets.

### Hot-swapping the policy

A session builds its policy once and shares it immutably with every tool, so a
settings change would otherwise wait for a new session. [`live_policy.rs`](./live_policy.rs) holds
the current policy in a process-global cell. `install` is called at bootstrap
(`core/runtime/bootstrap.rs`) and at channel startup, and `reload_from` is
called by the config save path (`config/ops/agent.rs`) so `current()` reflects
the new `[security.autonomy]` block immediately. The approval gate reads the user's
"Always allow" list (`autonomy.auto_approve`) through `current()`. Today tools
observe the swap at the next session boundary.

### Approval

[`approval/`](./approval/) is the human-in-the-loop gate for calls the policy prompts on.
The approval itself is the TinySecurity module's (its durable lifecycle, exact
binding, grants, expiry and audit trail); the host serves its store, audit sink,
prompt and human verification. The gate asks the module, publishes
`DomainEvent::ApprovalRequested` when the module asks for the prompt so the UI
can show a card, parks the tool future on a oneshot, and wakes on the
`approval_decide` RPC or a typed chat reply. Timeouts and denials fail closed.
`approval_gate_boot_decision` (`core/types.rs`, applied in
`core/runtime/bootstrap.rs`) always installs the gate for
`HostKind::TauriShell` and ignores `OPENHUMAN_APPROVAL_GATE=0` there; CLI,
Docker and library hosts may opt out. When no gate is installed, the harness
approval resolver (`agent/tinyagents/middleware/approval.rs`) warns and allows a
tool the module says needs a human; a channel policy that demands approval is
denied instead (`middleware/tinysecurity_channel.rs`). See [approval/README.md](approval/README.md).

## Layout

| Path | What it does |
| --- | --- |
| [`policy/`](policy/README.md) | `SecurityPolicy`, autonomy tiers, and the host's questions to the module about commands, paths and the hourly budget. |
| [`floor.rs`](./floor.rs) | The unconditional protected-path floor, asked of the module without a policy. |
| [`approval/`](approval/README.md) | The approval gate, the module-backed approval engine and its host callbacks, argument redaction and the `approval_*` RPCs. |
| [`credentials/`](credentials/README.md) | The backend credential slots, auth profiles, provider OAuth and the `auth.*` RPC namespace. |
| [`devices/`](devices/README.md) | Mobile device pairing and the end-to-end encrypted tunnel to iOS clients. |
| [`egress/`](egress/README.md) | Descriptors for every external data transfer and the privacy-mode enforcement chokepoint, decided by the module. |
| [`encryption/`](encryption/README.md) | AES-256-GCM at-rest encryption for memory storage and the encrypt/decrypt RPCs. |
| [`keyring/`](keyring/README.md) | OS keychain backend with an encrypted-file fallback, and the `SecretStore` config-field codec. |
| [`keyring_consent/`](keyring_consent/README.md) | Consent gate for falling back from the OS keychain to local encrypted storage. |
| [`tinymemory_api::safety`](https://github.com/tinyhumansai/tinymemory/blob/d1b06334faf7f8abb62bd1bd4ec4c4d4862f53a4/crates/tinymemory-api/src/safety/README.md) (upstream contract) | Fully local PII and identification-risk detection. `scan` and `redact_identifiers` require the API's opt-in `local-safety` feature; detection results and policy decisions remain separate. |
| [`prompt_injection/`](prompt_injection/README.md) | Deterministic prompt-injection screening (`Allow`, `Review`, `Block`). |
| [`live_policy.rs`](./live_policy.rs) | Process-global, hot-swappable current `SecurityPolicy` and privacy mode. |
| [`scrub.rs`](./scrub.rs) | Secret and PII scrubbing with the host's policy (`Policy::corroborated`) over TinyMemory's scrubbers. |
| [`audit_record.rs`](./audit_record.rs) | The host's own security audit rows (`HostAuditEvent`), recorded on the TinySecurity module's audit stream through `RecordAudit2`. |
| [`pairing.rs`](./pairing.rs) | Non-loopback bind guard for the core RPC token; re-exports channel pairing helpers. |
| [`core.rs`](./core.rs) | `redact()`, the 4-character-prefix log redaction. |
| [`ops.rs`](./ops.rs) | `security_policy_info_for_config` and `load_and_get_security_policy_info`. |
| [`schemas.rs`](./schemas.rs) | The `security.policy_info` controller. |
| [`tools.rs`](./tools.rs) | `SecurityPolicyInfoTool`, the only agent-callable tool in this domain. |
| [`secrets.rs`](./secrets.rs) | One-line re-export of `keyring::encrypted_store` for older import paths. |

## Key types and entry points

- `SecurityPolicy` ([`policy/types.rs`](./policy/types.rs)): the policy value. Build it with
  `SecurityPolicy::from_config`. Its methods are the enforcement entry points
  listed above, plus `spend_action` (tier plus hourly budget, for callers that
  are not tools), `check_paths` and `is_internal` in
  [`policy/module_gate.rs`](./policy/module_gate.rs), and `validate_path` in
  [`policy/path_checks.rs`](./policy/path_checks.rs).
- `AutonomyLevel` (`ReadOnly`, `Supervised`, `Full`), `CommandClass` (`Read`,
  `Write`, `Network`, `Install`, `Destructive`) and `GateDecision` (`Allow`,
  `Prompt`, `Block`), all in `policy/types.rs`.
- `TrustedRoot` and `TrustedAccess` (`Read`, `ReadWrite`): user-granted
  subtrees that override `workspace_only` and `forbidden_paths`, never the
  floor or the workspace-internal boundary.
- `POLICY_BLOCKED_MARKER` and `POLICY_DENIED_MARKER`: prefixes on refusal
  strings so the harness and model can tell a policy refusal from a tool
  failure.
- `validate_path_within_root`, `openhuman_scratch_dir`,
  `ensure_openhuman_scratch_dir` ([`policy/enforcement.rs`](./policy/enforcement.rs)).
- `live_policy::{install, current, reload_from, reload_privacy,
  set_action_dir}` (`live_policy.rs`).
- `redact::{text, endpoint_origin, endpoint_host}` ([`redact.rs`](./redact.rs)):
  the synchronous redaction entry points, answered by the TinySecurity module
  (`Text`, `EndpointOrigin`, `EndpointHost`). `text` serves the approval audit
  rows, halt summaries, failure lines and artifact offload; without the module
  it applies the fixed `core::log_redaction` floor, never the original text.
  The endpoint forms answer `[REDACTED:suppressed]` instead of a URL when the
  module cannot. Whole-URL log fields use `core::log_redaction::scrub_secrets`.
  `redact::arguments` answers structured tool-call arguments (approval prompts and
  rows, flow audit records) in the module's `StructuredArguments` mode and
  withholds them (a shape-only marker) when the module cannot answer; there is no
  host-side structured redactor to fall back to.

  Behaviour notes (decisions recorded for #7328 phase 3):
  - The module answers only after its bridge is armed by the first successful
    module call, and the bridge is process-global. Endpoint log fields written
    before that read `[REDACTED:suppressed]`; this fails closed and is accepted.
  - `redact::text` waits up to 5 s (artifact bodies) and the log bridge 250 ms per
    call. A runtime worker uses `block_in_place`; a current-thread runtime that
    owns the module is refused rather than deadlocked. `security::redact::text`
    is the one place production can take the host boot floor
    (`core::log_redaction::floor_scrub_text`) instead of the module: it never
    returns the original text and is not gated on `cfg(test)`.
  - A scheme-less `user:pass@host` DSN passes the floor but is removed by the
    module pass (pinned on the released archive by
    `whole_url_log_fields_lose_their_credentials_with_the_module_armed`).
  - Lexical host classification (Langfuse environment, turn policy, the
    TinyHumans backend URL guard) uses `tinytools_std::url_guard::
    is_private_or_local_host`. These callers label a configured endpoint for
    routing and authorize no request, so the module's scoped `ScopedUrlGuard`
    (destination policy, DNS pinning, one-shot grant) is not the right tool for
    them.
  - Agent network tools (`http_request`, `web_fetch`, `curl`) ask the module for
    every destination: the gate (`tools/impl/network/gate.rs`) implements
    `NetGate::authorize_destination` over `modules::security::egress::
    authorize_tool_destination`, which registers an open-public egress policy
    for the authenticated subject, runs `ScopedUrlGuard` on the real URL,
    method, headers and body, spends the grant and hands the tool the approved
    socket addresses. The tool connects only to those (`resolve_to_addrs`, URL
    host kept for `Host`/SNI/certificate checks). A refused, missing, faulted or
    slow module refuses the call. A build without `security-module` gets no
    module-checked resolved-address floor: `module_destination` answers `None`
    and the tool's own DNS check is the only one in force.
  - Browser navigation (`modules/browser.rs`) keeps the synchronous lexical
    pre-filter and then asks the module for a verdict on the `Browser`
    transport (`authorize_browser_navigation`): the address floor on every
    resolved address and the privacy mode. The browser module performs the
    connection, so this is a preflight verdict, not a grant, and a redirect or
    subresource the page causes later is the browser module's to enforce.
  - Tool-result credential scrub (`agent/tinyagents/middleware/
    credential_scrub.rs`) is the module's `CredentialsOnly` mode
    (`redact::credentials`): provider tokens, JWTs, PEM keys, cookie headers
    and labelled secrets go, while source code that mentions tokens and PII
    stay (#6954). Results above the module's 64 KiB request bound are answered
    in 48 KiB line-aligned chunks. Once the module serves, a failed answer
    withholds the result; where no module serves (slim build, library host,
    unit test) the harness's own credential scrub is the documented floor, as
    `core::log_redaction` is for logs. The browser confirmation token is taken
    out of the document before the scrub and put back afterwards.
  - Composio exact-key removal (`integrations/composio/tools/redact.rs`) uses
    the module's `RedactWith`: the keys a call resolves to are sent as that
    call's exact credentials, so a tenant's key is removed from that tenant's
    output without a process-wide reinitialization. The file only resolves the
    keys (host configuration), walks the outcome and writes answers back; a
    result the module could not check is withheld. Without `security-module`
    nothing can check a string that mentions a key, so every such string is
    withheld (fail closed).
  - Chat inference (`agent/tinyagents/egress_model.rs`) assesses the outgoing
    messages on every route that leaves the device (the primary, workload-tier
    routes and fallbacks), not only the primary: a local primary does not exempt
    an external fallback. A refused hop fails that hop only.
  - Hot logging paths keep a documented floor instead of a bus call per line:
    `core::log_redaction` (boot floor plus the module second pass; Sentry
    `before_send` runs through it) and `core::rpc_log::redact_params_for_log`
    (key-name masking of the `[rpc:dispatch] enter` trace, which fires on every
    dispatch and must not cross the module bus). Neither is a policy decision.
  - Redactors deliberately left, with owners: the memory write guard
    (`tinymemory_api::safety`, tinymemory-owned), the `denials` log
    (`tinytools`, upstream), and the rpc client / app deep-link
    `redact_url_for_log` copies (they run where no module exists: a remote-core
    client and the deep-link forwarding process).
- `audit_record::{HostAuditEvent, record}` ([`audit_record.rs`](./audit_record.rs)): a host-observed row (today a shell command's execution result) and its append to the calling tenant's subject stream.
- `is_public_bind` and `ensure_core_rpc_token_for_bind` (`pairing.rs`): refuse
  a non-loopback RPC bind without an `OPENHUMAN_CORE_TOKEN`
  (`CORE_TOKEN_ENV_VAR`). `PairingGuard`, `constant_time_eq` and the token
  helpers are re-exported from `tinychannels_bus::security`.
- `SecretStore` (re-exported from [`keyring/`](./keyring/)): encrypted-on-disk secrets whose
  master key lives in keychain-backed storage.
- The [`egress`](./egress) re-exports at the crate root: `enforce_egress`
  (async), `emit_external_transfer`, `local_only_tool_block[_blocking]`,
  `egress_blocks_message[_blocking]`, `EgressDescriptor`. OpenHuman's agent stop-summary code directly consumes
  `tinymemory_api::safety::redact_identifiers`; this security module does not
  re-export the safety API.

## RPC surface

This file registers one controller through `core/all.rs`:

- `security.policy_info`: the effective policy (autonomy level,
  `workspace_only`, allowed commands, hourly budget, approval and high-risk
  flags), computed from the loaded config.

`SecurityPolicyInfoTool` (`security_policy_info`, deferred exposure) returns
the same payload to the agent. Command and path gating is enforced inside the
engine and is never an agent-callable tool.

The subfolders register their own namespaces: `approval.*` ([`approval/`](./approval/)),
`auth.*` ([`credentials/`](./credentials/)), `devices.*` ([`devices/`](./devices/)), `encrypt.secret` and
`decrypt.secret` ([`encryption/`](./encryption/)), and
`keyring_consent.*` ([`keyring_consent/`](./keyring_consent/)). Their READMEs list the methods.

## Boundaries

- Sandbox backends do not live here. `crates/openhuman-core/src/sandbox/`
  picks a backend per session and delegates native confinement and process
  ownership to the loaded TinyBox module (`vendor/tinybox`).
- Shell parsing, command classification and gating run inside the loaded
  TinySecurity module (`tinysecurity-bus` `CheckCommand` / `ClassifyCommand`).
  The host sends the command directly to TinySecurity; TinyBox has no shell
  classification or authorization surface. Which settings the module is given,
  and what a refusal looks like to the agent, stay here.
- Secret and PII scrubbers are TinyMemory's
  (`tinymemory_integrations::safety`, `vendor/tinymemory`); this folder only
  pins the policy.
- Channel pairing primitives (`PairingGuard`, token hashing) belong to
  `tinychannels` (`vendor/tinychannels`).
- The core never obtains, validates, exchanges or refreshes a credential.
  Login-token exchange and `/auth/me` belong to the host's session owner
  (`openhuman_tinyhumans::session`). [`credentials/`](./credentials/) only accepts one through
  `auth.set_credential`.
- The tinyagents runtime holds no authority. Its "may this tool run?" question is
  answered by the security middleware (`agent/tinyagents/middleware/security_gate.rs`),
  which asks the module; `agent/tinyagents/host/security_gate.rs` only refuses a
  nested call and screens untrusted input.

## Gotchas

- Tests that exercise the allowlist, the tier or containment must build their
  config with `enabled: true`. With the shipped default they pass vacuously.
  [`policy/policy_disabled_tests.rs`](./policy/policy_disabled_tests.rs) pins the contract of the disabled path.
- Do not move a floor check below the `if !self.enabled` return. The floor is
  the part a future edit is most likely to take with it.
- `SecurityPolicy::default()` is enabled, `AutonomyConfig::default()` is not.
  Code that builds a policy without `from_config` gets the strict one.
- The tool gate reserves the hourly action once per acting call. Tools must not
  spend again: `FsGate` and `NetGate` report `is_rate_limited` as false and
  `record_action` as true because the gate already reserved it, and their
  `can_act` asks the module for the tier only.
- A user's "Always allow" entry only suppresses the human prompt. The tier,
  the class and the floor still run before the gate.

## Tests

Each file has a sibling `<module>_tests.rs`; the policy suites live in
[`policy/`](./policy/) (`policy_tests.rs`, `policy_disabled_tests.rs`,
`policy_trusted_roots_tests.rs`, `module_gate_tests.rs` and others). Run them with
`cargo test -p openhuman security::` or `pnpm debug rust security`.

## Further reading

- [Security architecture](../../../../docs/gitbooks/en/developing/architecture/security.md)
- [Privacy and security](../../../../docs/gitbooks/en/features/privacy-and-security.md)
- [Approval gate](../../../../docs/gitbooks/en/features/approval-gate.md)
- [OS keyring and secret storage](../../../../docs/gitbooks/en/features/os-keyring-and-secret-storage.md)

## Audit ownership

There is one security audit trail, and it is the module's: an ordered, contiguous
`Audit2` stream per subject that the module sanitizes and the host persists behind
`AuditSink2` (`approval/native/callbacks.rs::append`, `approval/native/rows.rs`).

- Approval, judge, approval-engine and sandbox-plan rows are appended by the module
  itself. Approvals fail closed when a row cannot be committed.
- A call the policy lets through without a prompt (`auto_approve_all`, the agent's
  approvals off, an allowlist or flow-grant match) is an audited automatic approval
  in the same stream (`auto:host-policy`, `auto:tool-grant`, `auto:flow-grant`), written
  by the module's approval engine; the host writes no separate row for it.
- What only the host observes is recorded through the module's `RecordAudit2`
  (contract 1.21; `audit_record.rs`, `modules/security_audit.rs`): a shell command's
  execution result (`tool:shell`, kind `execution`). The host reserves the subject's next
  stream position under the stream lock, the module sanitizes the row with its own
  redactor and appends it through the same sink, and the answer must carry a committed
  receipt for exactly that event. `AuditKind2::PolicyDecision` exists in the contract
  for a host decision that lets a call through unprompted; the host has no such row to
  write today.
- A row the module cannot record (no module, no tenant, a refused or uncommitted
  append) is logged at warn level by the caller and is not retried elsewhere: there is
  no second sink, and the former owner-only JSONL logger (`[security.audit]`,
  `AuditLogger`) and its config table are gone.
- MCP protocol telemetry (`mcp/audit`, the `mcp_audit` stream and `AuditStore`)
  stays tinymcp-owned: see `vendor/tinymcp/docs/adr/0002-mcp-audit-ownership.md`.
  Host policy events go through tinysecurity and are joined with MCP rows at
  read time. `mcp/audit` is not made a facade.
- `pii::scan` and the `security/secrets.rs` alias no longer exist; the former
  `security/core.rs` (only `redact`) is now `security/redact.rs`. No
  `#[allow(unused_imports)]` remains under `security/`.
- Tinycomputer consequence/payment gates are not wired. tinycomputer v0.14.0
  (contract 2.19) asks the host through `ConsequenceHost.Review` before an
  approved irreversible or paying action; a host that does not serve the
  interface leaves the module's own pause as the only gate
  (`Verdict::Unreviewed`), and a host that serves it and fails denies. This
  host does not serve it, so the existing pause plus the approval engine's
  `ContinueTask` decision are the gate. Serving it means a bus object that maps
  a `Review` onto an approval request; that is a feature of its own and is
  left for a follow-up rather than approximated here.
- Tinyskills `ScanReport::verdict()` (`Block`/`Warn`/`Pass`) is likewise not
  consumed by the host: skill trust and execution policy stay with the host's
  approval path, and no tinyskills change is needed to map the verdict when a
  caller is added.

Production-path coverage: `modules/security_production_tests.rs` (ignored; run by
`security-native-fixture.sh`) drives `register_path_policy` / `validate_path`
through the real module runtime and `ensure_loaded`, so the trusted-host
binding comes from `module_config` itself. `proxy` builds that configuration
only after the runtime is started and refuses to refresh without it.
`web_chat/event_bus_tests.rs` tests of the debug-only
`fresh_approval_surface_subscription` are `cfg(debug_assertions)` and therefore
not run by release lib-test profiles.
