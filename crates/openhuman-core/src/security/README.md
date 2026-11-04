# Security host adapters

TinySecurity owns security decisions and their durable state: command and path
policy, the protected-path floor, action budgets, approval lifecycle and trust,
redaction and scans, privacy and egress, sandbox planning, audit, and secret and
device crypto. OpenHuman calls the released module through `tinysecurity-bus`;
it does not keep a second policy implementation.

OpenHuman keeps product configuration and the host-side work around those
decisions:

- `modules/security/` translates trusted config and caller facts, hosts the
  storage, audit, keyring and prompt callbacks, and maps module outcomes to the
  existing host contracts. Missing or faulted module answers fail closed.
- `switch.rs` decides whether those checks run at all. Security is disabled in a
  build without the `security-module` feature, or when the booted config says
  `[security] mode = "off"` (`RuntimeBuilder::security` for an embedder). The
  checks are then permissive: the prompt screen, tool classification and
  approvals, path checks (no path policy at all, protected floor included),
  the action budget and the audit trail allow, egress and network shell
  commands are permitted unless `[privacy] mode = "local_only"`
  (`policy/local_only_commands.rs` classifies shell commands then), and
  redaction passes tool
  output, diagnostics and log URLs through unredacted. The approval gate is
  unchanged: approvals a channel, flow or origin demands still need a person
  (and an engine), while the module's own approval prompts are skipped. The
  encrypted secret store, the keychain and device pairing still need the
  module. Enabled security is unchanged: it fails closed. The core logs one line
  at boot when security is disabled, and why.
- `approval/` parks the live tool future, publishes the existing approval
  events, serves the UI and chat callbacks, and applies user choices to host
  configuration.
- `sandbox/` passes TinySecurity plans to TinyBox and starts or reports the
  selected executor.
- RPC namespaces, config schemas and user-facing consent surfaces stay in
  OpenHuman.

See [Security architecture](../../../../docs/gitbooks/en/developing/architecture/security.md)
for the full boundary and [policy adapters](./policy/README.md) for the host
configuration translation.
