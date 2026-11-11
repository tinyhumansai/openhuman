# Approval host adapter

TinySecurity owns approval decisions and durable state: pending requests,
expiry, once-only decisions, reusable tool and flow grants, and the audit
stream. OpenHuman owns the live tool future and the ways a person reaches the
decision: prompt delivery, web and chat events, the approval RPC surface, and
host callbacks for storage and audit.

The host registers trusted caller facts and scope with TinySecurity, parks the
live call when the module returns a pending approval, and resumes it only after
the module reports a durable decision. Timeout, lost channel, failed callback,
or unavailable module denies the action. The host reports the terminal tool
outcome back to TinySecurity for the audit record.

**Always allow** is a user choice. The `approval_decide` handler saves a
human-approved tool name to `[autonomy].auto_approve` and reloads the live host
policy. Agent-specific access is not copied to the operator's list. An optional
Jev score, when explicitly configured by a host, can approve only the exact
pending request once after an audited decision; it cannot create a reusable
grant or override a hard denial. OpenHuman's current TinySecurity config leaves
Jev scoring disabled.

The `approval.*` RPC names and approval event names remain host wire contracts.
The host may change how it stores callback documents or presents a prompt, but
must not bypass TinySecurity's decision or durable-state checks.

## Host-owned files

- `gate*.rs` parks and resumes the process-local tool future.
- `native/` registers host callbacks and adapts TinySecurity's approval and
  audit interfaces to OpenHuman storage and prompt delivery.
- `rpc.rs`, `schemas.rs` and `types.rs` preserve the host-facing RPC and event
  contracts.
- `summary.rs` builds the safe human-readable prompt summary.

See [Approval gate](../../../../../docs/gitbooks/en/features/approval-gate.md)
and [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md).
