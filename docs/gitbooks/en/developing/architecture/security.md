---
description: >-
  The trust boundary for the autonomous core: the autonomy and risk policy,
  sandbox selection, the audit log, the encrypted secret store, the pairing
  guard and the redact() helper.
icon: shield-halved
---

# Security

`crates/openhuman-core/src/security/` is the trust boundary for the autonomous core. Look here first when you ask "is this agent action allowed, and if so, how is it confined?"

It owns:

- The autonomy and risk policy that decides whether a tool call is allowed.
- The sandbox selection that confines calls when the host supports it.
- The append-only audit log of every agent action.
- The encrypted secret store.
- The pairing guard that gates public binding of the RPC server.
- The `redact()` helper every other domain uses to keep plaintext credentials out of logs.

It does not own the cross-domain `EncryptionEngine` (in `security/encryption/`) or per-channel credential storage (in `security/credentials/`).

## Public surface

| Item                                                                                                                          | File         | Purpose                                                                 |
| ----------------------------------------------------------------------------------------------------------------------------- | ------------ | ----------------------------------------------------------------------- |
| `SecurityPolicy`                                                                                                              | `policy/types.rs` (path checks in `policy/path_checks.rs`, command classification in `policy/command_checks.rs`, gating in `policy/enforcement.rs`) | Assembles runtime policy from `AutonomyConfig` + workspace dir.         |
| `AutonomyLevel` (`ReadOnly` / `Supervised` / `Full`)                                                                          | `policy/types.rs` | Three-step autonomy ladder.                                             |
| `CommandRiskLevel`, `ToolOperation`, `ActionTracker`                                                                          | `policy/types.rs` | Risk classification and per-session counting.                             |
| `SecretStore`                                                                                                                 | `secret_storage/encrypted_store.rs` (`secrets.rs` re-exports it) | OS-keychain / encrypted-file secret persistence with round-trip helpers. |
| `AuditLogger`, `AuditEventType`, `AuditEvent`, `Actor`, `Action`, `ExecutionResult`, `SecurityContext`, `CommandExecutionLog` | `audit.rs`   | Append-only audit trail.                                                |
| `PairingGuard`, `constant_time_eq`, `is_public_bind`                                                                          | `pairing.rs` (`PairingGuard` and `constant_time_eq` are re-exported from `tinychannels_bus::security`) | Pairing-token check before binding the RPC server publicly.             |
| `redact(value: &str) -> String`                                                                                               | `core.rs`    | Uniform 4-char-prefix redaction for logs.                               |
| `security_policy_info_for_config(&Config) -> Outcome<serde_json::Value>`                                                    | `ops.rs`     | RPC handler for the doctor / settings UI.                               |

## Sandboxing

This module carries no sandbox backends. Per-session sandbox selection lives in `crates/openhuman-core/src/sandbox/`. It delegates local OS confinement to `tinybox-jail` (vendored tinybox) and keeps its own `docker run` executor. Command classification scans shell strings with `tinybox_core::shell::scan`, and the classification rules stay in this module.

## Autonomy ladder

`AutonomyLevel` is a three-step ladder that sets how strictly the policy gates tool calls. It only takes effect while `[autonomy] enabled = true`.

- `ReadOnly` blocks every writing class outright. No approval can authorize one.
- `Supervised` is the default. It allows reads and parks everything else for an approval round trip.
- `Full` allows reads and writes. It still parks the network, install and destructive classes.

The wire spellings are lowercase (`readonly`, `supervised`, `full`). The user-facing table is in [Approval gate](../../features/approval-gate.md).

`CommandRiskLevel` and `ToolOperation` classify a tool call. `ActionTracker` keeps the per-session counts that the policy compares against its caps. The agent harness asks `SecurityPolicy` for a decision before every executable tool dispatch.

All of this applies only when the autonomy policy is on. `[autonomy] enabled = false` is the default, and `SecurityPolicy::from_config` carries that flag through every enforcement entry point. With the policy off, command classification, the approval gate, the command allowlist and the hourly action budget do nothing. `workspace_only`, `forbidden_paths` and the workspace-internal boundary are not enforced either.

One check never turns off. `is_always_forbidden` blocks credential stores (`~/.ssh`, `~/.gnupg`, `~/.aws`) and system roots. It also rejects `..` traversal and null bytes in a path. This is the floor the module keeps whatever the configuration.

## Audit log

`audit.rs` writes an append-only stream of `AuditEvent`s under the workspace dir. Every executable tool call lands here with its `Actor` (agent or user), `Action`, `ExecutionResult` and the `SecurityContext` (autonomy level, sandbox backend and so on) it ran under. The log tells you afterwards what the agent did and why it was allowed.

## Pairing guard

`PairingGuard` (in `pairing.rs`) stands between the RPC server and any attempt to bind to a non-loopback address. `is_public_bind` detects that case. `PairingGuard` then requires a pairing token, compared in constant time with `constant_time_eq`, before the bind is allowed. The iOS and LAN-companion pairing flow relies on this to keep an unpaired peer from attaching to the desktop core.

## Secret store

`SecretStore` (implemented in `security/secret_storage/encrypted_store.rs`, re-exported through `secrets.rs`) encrypts config-field secrets with ChaCha20-Poly1305 (`enc2:` prefix). It migrates the legacy XOR `enc:` format when it decrypts. Core owns the platform-independent secret-storage contracts, an in-memory default, and an optional encrypted-file backend. `openhuman-rpc::keyring` owns the native OS providers and installs them through those contracts before host boot. RPC product hosts explicitly select persistent storage; embed uses memory unless a provider or backend override is supplied. Backend selection and migration are described in `crates/openhuman-core/src/security/secret_storage/README.md`.

## `redact()`

`redact(value)` returns a uniform 4-character-prefix string (for example `"sk-a…"`) for use in logs and error messages. Use it whenever a secret, credential, token, or PII string is about to be formatted into a `log::` / `tracing::` call. Other domains call it directly: `credentials/`, `webhooks/`, `composio/`, the integration adapters.

## Layout

| Path                                                          | Role                                                                      |
| ------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `policy/` (`mod.rs`, `types.rs`, `path_checks.rs`, `command_checks.rs`, `enforcement.rs`, `policy_*_tests.rs`, `proptest_tests.rs`) | `SecurityPolicy`, `AutonomyLevel`, risk classification, path and command checks, action tracking. |
| `core.rs`, `core_tests.rs`                                    | `redact()` + small shared helpers.                                        |
| `audit.rs`                                                    | Append-only audit log types.                                              |
| `secrets.rs`, `secret_storage/`                                      | `SecretStore` (implemented in `secret_storage/encrypted_store.rs`) + round-trip tests. |
| `pairing.rs`, `pairing_tests.rs`                              | `PairingGuard` + constant-time helpers.                                   |
| `ops.rs`                                                      | RPC handler (`security_policy_info_for_config`).                          |
| `schemas.rs`                                                  | Controller schemas + handler dispatch.                                    |
| `mod.rs`                                                      | Re-exports of the public surface above.                                   |
| `live_policy.rs`, `scrub.rs`, `tools.rs`                      | Live-policy lookup, the host scrubbing policy (`scrub::host_policy`), and the domain's own agent tools. |
| `approval/`, `credentials/`, `devices/`, `egress/`, `encryption/`, `keyring_consent/`, `prompt_injection/` | Sibling kernel-security domains under the same module. Each owns its own surface. This page covers the policy core only. |

Local PII detection belongs to [`tinymemory_api::safety`](https://github.com/tinyhumansai/tinymemory/blob/d1b06334faf7f8abb62bd1bd4ec4c4d4862f53a4/crates/tinymemory-api/src/safety/README.md) behind its `local-safety` feature. OpenHuman keeps privacy policy and redaction decisions.

## Calls into

- `crates/openhuman-core/src/config/`: `SecurityConfig`, `AutonomyConfig` for policy + sandbox selection.
- Workspace filesystem, for the audit log and secret store.

## Called by

- `crates/openhuman-core/src/cron/ops.rs`: wraps shell jobs in `SecurityPolicy::from_config`.
- `crates/openhuman-core/src/tools/ops.rs` and `tools/impl/{browser,document,filesystem,network,presentation,system}/`: every executable tool consults `SecurityPolicy`.
- `crates/openhuman-core/src/tools/impl/network/{gate,mcp,mcp_server_tools}.rs`: risk-classify outbound calls.
- `crates/openhuman-core/src/memory/guard.rs`: wraps every memory write in `security::scrub::host_policy()`.
- `crates/openhuman-core/src/agent/tools/delegate.rs`: sub-agent dispatch goes through the autonomy gate.
- `crates/openhuman-core/src/security/credentials/`: uses `SecretStore` and `redact`.

## Tests

- Unit: `pairing_tests.rs`, `policy/policy_tests*.rs`, `policy/proptest_tests.rs`, `secret_storage/encrypted_store_tests*.rs`.
- `core_tests.rs` covers `redact()`.

## Related

- [`security/README.md`](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-core/src/security/README.md): the in-repo overview this page mirrors.
- [Architecture](../architecture.md): the wider system context.
- [Agent harness](agent-harness.md): where `SecurityPolicy` is consulted on every tool dispatch.
