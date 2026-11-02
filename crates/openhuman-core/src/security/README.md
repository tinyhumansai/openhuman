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
