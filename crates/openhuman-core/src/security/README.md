# security

The trust boundary of the core. This folder decides whether an agent may run a
command or touch a path, parks tool calls that need a human yes or no, stores
credentials and secrets, scrubs secrets and PII out of anything persisted, and
guards the RPC listener when it binds beyond loopback. `security/mod.rs` calls
it the kernel security family. The `security-module` feature moves asynchronous
filesystem authorization into the separately loaded TinySecurity native module.

## Native filesystem policy

With `security-module`, `SecurityPolicy::validate_path` and
`validate_parent_path` use the typed host client in `modules/security.rs`.
`policy/native_paths.rs` translates trusted roots, the acting directory,
internal-state reservations, and the current turn's workspace grant into an
immutable scope. Scope registration deduplicates the whole policy; one agent
cannot replace another agent's settings through global module reinitialization.

The host links only `tinysecurity-bus`. The engine canonicalizes paths and
checks read/write grants in the native module. A missing module, an untrusted
recipient, a timeout, or a malformed response denies access without a local
fallback. Synchronous lexical checks remain host preflights during this phase;
they do not substitute for the asynchronous I/O authorization.

Production admission pins TinySecurity v0.2.2 and its published archive checksums
in the module registry. `scripts/ci/security-native-fixture.sh` loads those
released archives and invokes their admission, path, and latency tests. Explicit
local fixtures remain available for development; their digests never become
production release pins. Shell policy,
approvals, redaction, and crypto continue to use their existing host engines
until their own migration phases are implemented and verified.

Callers are spread across the core. Every acting tool consults
`SecurityPolicy` before it runs, the agent harness asks the approval gate
before a prompted call, the credentials domain owns the `auth.*` RPC
namespace, and the RPC server asks [`pairing.rs`](./pairing.rs) whether a bind needs a token.

## How it works

### The policy and its off switch

`SecurityPolicy` ([`policy/types.rs`](./policy/types.rs)) is the value every tool receives. It is
built once per agent session by `SecurityPolicy::from_config`
([`policy/enforcement.rs`](./policy/enforcement.rs)) from the `[autonomy]` config block, the workspace dir
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
| `gate_decision` | [`policy/command_checks.rs`](./policy/command_checks.rs) | `GateDecision::Allow` |
| `check_gated_command` | `policy/command_checks.rs` | `Ok(class)`, no structural guard |
| `validate_command_execution` | `policy/command_checks.rs` | `Ok(risk)`, no allowlist |
| `is_command_allowed` | `policy/command_checks.rs` | `true` |
| `can_act` | `policy/enforcement.rs` | `true` |
| `record_action`, `is_rate_limited` | `policy/enforcement.rs` | never limited |
| `is_path_string_allowed` | [`policy/path_checks.rs`](./policy/path_checks.rs) | `true` after the floor checks |
| `is_resolved_path_allowed_for` | `policy/path_checks.rs` | `true` after the floor check |
| `check_resolved_against_forbidden` | `policy/path_checks.rs` | `Ok(())` after the floor check |

So with the policy disabled, command classification still runs (callers get a
`CommandClass` back) but nothing is gated on it: no approval prompt, no
command allowlist, no hourly action budget, and no `workspace_only`,
`forbidden_paths`, trusted-root or workspace-internal containment.

A floor survives the off switch. These checks run before the `enabled` test
and hold in every configuration:

- `is_always_forbidden` (`policy/path_checks.rs`). Credential stores are
  matched by path segment, case-insensitively: `.ssh`, `.gnupg`, `.aws`,
  `.azure`, `.kube`, `keychains`, and the Windows pairs
  `Microsoft\{Protect,Credentials,Crypto,Vault}`. System roots are matched by
  absolute prefix: `/etc`, `/root`, `/boot`, `/proc`, `/sys`, `/system`,
  `C:\Windows`, `C:\Program Files`, `C:\Program Files (x86)`,
  `C:\ProgramData`. A trusted-root grant cannot reach these either, and the
  per-turn workspace grant is checked after this test, so it cannot either.
- In `is_path_string_allowed`: a null byte, a `..` path component, and
  URL-encoded traversal (`..%2f`, `%2f..`) are rejected. A bare relative name
  that matches one of the reserved workspace files (`WORKSPACE_INTERNAL_FILES`
  in `policy/types.rs`, such as `.env`) is also rejected.

`SecurityPolicy::default()` is the opposite of the shipped default: it sets
`enabled: true`. It is the fallback for a policy built with no config at all
(tests, examples, a few internal call sites), where failing closed is right.
Only `from_config` carries the shipped `false`.

### With the policy enabled

Set `[autonomy] enabled = true` and the full policy applies. Commands are
classified, then gated by tier:

```text
 shell command string
        |
        v
 strip_quoted_heredoc_bodies      quoted heredoc body is data, blanked
        |
        v
 classify_command                 highest class across ; | && || segments,
        |                         redirect or tee lifts to >= Write
        v
 class.max(declared)              model may raise the class, never lower it
        |
        v
 gate_decision(class)
        |
   +----+--------------+-------------------+
   v                   v                   v
 Allow               Prompt              Block
 run the tool        ApprovalGate park   refused with
                     (10 min TTL)        POLICY_BLOCKED_MARKER
```

The tier matrix in `gate_decision`: `ReadOnly` allows only `Read` and blocks
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
  `is_workspace_internal_path` covers `WORKSPACE_INTERNAL_DIRS`,
  `WORKSPACE_INTERNAL_FILES`, the `memory-`, `memory_tree-` and `session_raw-`
  prefixes, per-artifact directories under `artifacts/` (but not
  `artifacts/tool-results/`), and the account `config.toml` beside the
  workspace. A trusted-root grant does not weaken this.
- Unknown commands classify as writes. `classify_command` is fail-closed: a
  command that is not provably read-only is at least `CommandClass::Write`.
  The body of a quoted heredoc (`<< 'EOF' ... EOF`) is blanked first by
  `strip_quoted_heredoc_bodies`, since the shell expands nothing there. An
  unquoted delimiter (`<< EOF`) is expanded and is still scanned.
- Outside `Full`, `check_gated_command` rejects hidden execution (command or
  process substitution, backticks, background `&`) that could smuggle a
  command past the approval the human read.
- Interactive approval requests expire as denied after ten minutes
  (`DEFAULT_APPROVAL_TTL` in [`approval/gate.rs`](./approval/gate.rs); the Flow Canvas copilot path
  uses a three-minute window).

The shell-string scanning itself (segment splitting, quoting, heredoc
stripping, segment classification, hidden-execution detection) is
`tinybox_core::shell`, in `vendor/tinybox/crates/tinybox-core`.
[`policy/command_checks.rs`](./policy/command_checks.rs) composes it with the tier.

### Hot-swapping the policy

A session builds its policy once and shares it immutably with every tool, so a
settings change would otherwise wait for a new session. [`live_policy.rs`](./live_policy.rs) holds
the current policy in a process-global cell. `install` is called at bootstrap
(`core/runtime/bootstrap.rs`) and at channel startup, and `reload_from` is
called by the config save path (`config/ops/agent.rs`) so `current()` reflects
the new `[autonomy]` block immediately. The approval gate reads the user's
"Always allow" list (`autonomy.auto_approve`) through `current()`. Today tools
observe the swap at the next session boundary.

### Approval

[`approval/`](./approval/) is the human-in-the-loop gate for calls the policy prompts on.
The gate persists a pending row, publishes `DomainEvent::ApprovalRequested` so
the UI can show a card, parks the tool future on a oneshot, and wakes on the
`approval_decide` RPC or a typed chat reply. Timeouts and denials fail closed.
`approval_gate_boot_decision` (`core/types.rs`, applied in
`core/runtime/bootstrap.rs`) always installs the gate for
`HostKind::TauriShell` and ignores `OPENHUMAN_APPROVAL_GATE=0` there; CLI,
Docker and library hosts may opt out. When no gate is installed, the harness
bridge (`agent/tinyagents/host/security_gate.rs`) warns and allows a `Prompt`
decision. See [approval/README.md](approval/README.md).

## Layout

| Path | What it does |
| --- | --- |
| [`policy/`](policy/README.md) | `SecurityPolicy`, autonomy tiers, command classification and path checks. |
| [`approval/`](approval/README.md) | The approval gate, its SQLite store, argument redaction and the `approval_*` RPCs. |
| [`credentials/`](credentials/README.md) | The backend credential slots, auth profiles, provider OAuth and the `auth.*` RPC namespace. |
| [`devices/`](devices/README.md) | Mobile device pairing and the end-to-end encrypted tunnel to iOS clients. |
| [`egress/`](egress/README.md) | Descriptors for every external data transfer and the `LocalOnly` enforcement chokepoint. |
| [`encryption/`](encryption/README.md) | AES-256-GCM at-rest encryption for memory storage and the encrypt/decrypt RPCs. |
| [`keyring/`](keyring/README.md) | OS keychain backend with an encrypted-file fallback, and the `SecretStore` config-field codec. |
| [`keyring_consent/`](keyring_consent/README.md) | Consent gate for falling back from the OS keychain to local encrypted storage. |
| [`pii/`](pii/README.md) | Fully local PII and identification-risk scanner. |
| [`prompt_injection/`](prompt_injection/README.md) | Deterministic prompt-injection screening (`Allow`, `Review`, `Block`). |
| [`live_policy.rs`](./live_policy.rs) | Process-global, hot-swappable current `SecurityPolicy` and privacy mode. |
| [`scrub.rs`](./scrub.rs) | Secret and PII scrubbing with the host's policy (`Policy::corroborated`) over TinyMemory's scrubbers. |
| [`audit.rs`](./audit.rs) | Append-only JSON audit log of agent actions (`AuditLogger`). |
| [`pairing.rs`](./pairing.rs) | Non-loopback bind guard for the core RPC token; re-exports channel pairing helpers. |
| [`core.rs`](./core.rs) | `redact()`, the 4-character-prefix log redaction. |
| [`ops.rs`](./ops.rs) | `security_policy_info_for_config` and `load_and_get_security_policy_info`. |
| [`schemas.rs`](./schemas.rs) | The `security.policy_info` controller. |
| [`tools.rs`](./tools.rs) | `SecurityPolicyInfoTool`, the only agent-callable tool in this domain. |
| [`secrets.rs`](./secrets.rs) | One-line re-export of `keyring::encrypted_store` for older import paths. |

## Key types and entry points

- `SecurityPolicy` ([`policy/types.rs`](./policy/types.rs)): the policy value. Build it with
  `SecurityPolicy::from_config`. Its methods are the enforcement entry points
  listed above, plus `enforce_tool_operation` (tier plus hourly budget for an
  acting tool) and `validate_path`-style helpers in [`policy/path_checks.rs`](./policy/path_checks.rs).
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
- `scrub::{sanitize_text, sanitize_json, has_likely_secret}` ([`scrub.rs`](./scrub.rs)):
  used by the approval store, memory writes, flow memory tools and artifact
  offload, so they all redact the same way.
- `AuditLogger` and `get_or_create_workspace_audit_logger` ([`audit.rs`](./audit.rs)).
- `is_public_bind` and `ensure_core_rpc_token_for_bind` (`pairing.rs`): refuse
  a non-loopback RPC bind without an `OPENHUMAN_CORE_TOKEN`
  (`CORE_TOKEN_ENV_VAR`). `PairingGuard`, `constant_time_eq` and the token
  helpers are re-exported from `tinychannels_bus::security`.
- `SecretStore` (re-exported from [`keyring/`](./keyring/)): encrypted-on-disk secrets whose
  master key lives in keychain-backed storage.
- The [`egress`](./egress) and [`pii`](./pii) re-exports at the crate root: `enforce_egress`,
  `emit_external_transfer`, `local_only_blocks`, `local_only_tool_block`,
  `EgressDescriptor`, and `scan_pii` with its result types.

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
  picks a backend per session and delegates local confinement to
  `tinybox-jail` (`vendor/tinybox`).
- Shell parsing and command classification primitives are owned by
  `tinybox` (`tinybox_core::shell`). Policy over them stays here.
- Secret and PII scrubbers are TinyMemory's
  (`tinymemory_integrations::safety`, `vendor/tinymemory`); this folder only
  pins the policy.
- Channel pairing primitives (`PairingGuard`, token hashing) belong to
  `tinychannels` (`vendor/tinychannels`).
- The core never obtains, validates, exchanges or refreshes a credential.
  Login-token exchange and `/auth/me` belong to the host's session owner
  (`openhuman_tinyhumans::session`). [`credentials/`](./credentials/) only accepts one through
  `auth.set_credential`.
- The tinyagents runtime holds no authority. The bridge that answers its
  "may this tool run?" question is `agent/tinyagents/host/security_gate.rs`.

## Gotchas

- Tests that exercise the allowlist, the tier or containment must build their
  config with `enabled: true`. With the shipped default they pass vacuously.
  [`policy/policy_disabled_tests.rs`](./policy/policy_disabled_tests.rs) pins the contract of the disabled path.
- Do not move a floor check below the `if !self.enabled` return. The floor is
  the part a future edit is most likely to take with it.
- `SecurityPolicy::default()` is enabled, `AutonomyConfig::default()` is not.
  Code that builds a policy without `from_config` gets the strict one.
- `enforce_tool_operation` records an action as a side effect. The harness
  gate reads `is_rate_limited` instead, so a call is not counted twice.
- A user's "Always allow" entry only suppresses the human prompt. The tier,
  the class and the floor still run before the gate.

## Tests

Each file has a sibling `<module>_tests.rs`; the policy suites live in
[`policy/`](./policy/) (`policy_tests.rs`, `policy_disabled_tests.rs`,
`policy_trusted_roots_tests.rs`, `policy_workspace_internal_tests.rs`,
`proptest_tests.rs` and others). Run them with
`cargo test -p openhuman security::` or `pnpm debug rust security`.

## Further reading

- [Security architecture](../../../../gitbooks/developing/architecture/security.md)
- [Privacy and security](../../../../gitbooks/features/privacy-and-security.md)
- [Approval gate](../../../../gitbooks/features/approval-gate.md)
- [OS keyring and secret storage](../../../../gitbooks/features/os-keyring-and-secret-storage.md)
