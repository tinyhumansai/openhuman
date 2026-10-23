---
description: >-
  OpenHuman's host adapters for TinySecurity policy, approvals, redaction,
  sandbox planning and cryptographic services.
icon: shield-halved
---

# Security

OpenHuman's security boundary is split between the host and the separately
loaded TinySecurity module. The host owns product configuration and identity,
RPC and UI behavior, persistence callbacks, and the actual execution of tools
and sandbox plans. TinySecurity owns the policy decisions and security state
that OpenHuman reaches through the `tinysecurity-bus` contract.

The production registry pins TinySecurity v0.17.1 (contract 1.28) and checks
each released native archive against its published SHA-256 before loading it.
OpenHuman links only `tinysecurity-bus`; it does not link the module's
implementation crates. A required call that cannot load, attest or receive a
valid response fails closed. There is no in-process policy fallback.

## Policy and tool admission

`modules/security_policy.rs` is the single translator from the host's
`Config` to TinySecurity policy types. It derives policy subjects from
authenticated dispatch context, registers immutable scopes, and keeps
credentials out of policy payloads. `modules/security.rs` owns module loading,
attestation, and the typed client.

The TinySecurity gate is installed in
`agent/tinyagents/middleware/tinysecurity_module.rs`. It checks the tool the
host is about to execute, not a model-supplied assertion about the tool. The
module decides access tiers, command classes, hourly action limits, protected
paths, privacy mode, and approval requirements. Denials take precedence over
approval, which takes precedence over allow.

The autonomy policy is disabled by default. With it disabled, its command
allowlist, action budget, workspace boundary and approval gate are inactive.
The protected-path floor remains active in every configuration and rejects
credential stores, system roots, traversal and malformed paths.

## Approvals and audit

TinySecurity owns approval request state, exact-action binding, expiry,
durable tool and flow grants, and the ordered audit stream. The host keeps the
parked future and user-facing prompt, emits product events, and serves the
module's persistence, prompt, audit and human-verification callbacks. A prompt
that expires or whose decision cannot be committed is denied. The normal
interactive approval window is ten minutes; copilot and sub-agent requests use
the shorter three-minute window.

Approval decisions, automatic approvals, judge verdicts and sandbox plans use
the module's audit stream. The host persists rows through the audit callback
and records execution results the module cannot observe. There is no separate
host approval log.

## Redaction and egress

Structured arguments, credential values, text redaction, prompt scans and
egress checks go through the TinySecurity adapters in
`modules/security_text.rs` and `modules/security_egress.rs`. The host retains a
bounded synchronous log floor in `core/log_redaction.rs`: logging starts
before the module loads, and synchronous sinks cannot await a bus call. Once
available, TinySecurity makes a second pass over that already scrubbed text.
A missing or faulty module never causes the original text to be emitted.

Network tools ask TinySecurity to validate destinations and the resolved
addresses before connecting. Browser navigation uses the module's browser
policy as well. Remaining duplicate redaction and URL helpers are being
removed as callers migrate; issue [#7328](https://github.com/tinyhumansai/openhuman/issues/7328)
tracks that work.

## Sandboxing

TinySecurity chooses and records a sandbox plan; TinyBox performs the local
isolation and process execution. The host converts approved plans into TinyBox
requests and rechecks the executor before spawn. SaaS always retains its
isolation requirement, even when `OPENHUMAN_SANDBOX=off` is set.

Some sandbox capabilities are still limited. Local macOS sandboxing is refused
when Seatbelt can only offer best-effort enforcement, and sandboxed commands
are denied network access until the host and TinyBox can enforce the requested
network policy. The remaining capability and spawner work is tracked by
[TinyBox issue #27](https://github.com/tinyhumansai/tinybox/issues/27) and
OpenHuman issue #7328. Do not infer that a process is sandboxed merely because
it was started by an agent.

## Cryptography and secret storage

Password encryption uses TinySecurity's `PasswordCrypt2` contract. The host
still owns its platform-independent secret-storage port, native OS keyring
providers and device-tunnel frame handling. TinySecurity publishes
`KeyringAccess2`, `PairingControl2`, `DevicePairing2`, `Device2` and
`DeviceFrame2`, but OpenHuman has not migrated all current callers to those
contracts. The production keyring and pairing module configurations remain
disabled until their host callers use them. This migration is part of issue
#7328 phase 8.

The public `encryption.*` RPC names remain host wire contracts. The host adapts
them to the module-backed password cipher without exposing module types through
the RPC surface.

## Source map

| Path | Responsibility |
| --- | --- |
| `modules/security.rs` | Load, attest and call the TinySecurity module. |
| `modules/security_policy.rs` | Translate trusted host config and identity into policy scopes. |
| `modules/security_text.rs`, `modules/security_egress.rs` | Adapt redaction, scanning, privacy and destination checks. |
| `modules/security_crypto.rs`, `modules/security_sandbox.rs` | Adapt password crypt and sandbox planning. |
| `security/approval/native/` | Host callbacks, persistence and prompt integration for the module-owned approval engine. |
| `security/devices/`, `security/secret_storage/` | Existing host-owned device tunnel and secret-storage implementations; migration remains open. |
| `core/log_redaction.rs` | Bounded synchronous log floor used before module load and when the module cannot answer. |
| `sandbox/` | Host-side plan validation and TinyBox execution. |

For user-facing behavior, see [Approval gate](../../features/approval-gate.md)
and [Privacy and security](../../features/privacy-and-security.md). For module
admission and release pins, see [Loadable modules](../loadable-modules.md).
