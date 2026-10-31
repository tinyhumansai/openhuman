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

**Steering** (`approval.steer`) is a third answer beside approve and deny: the
operator refuses *this* call and hands the agent a directive to follow instead.
It is host-only by construction. To TinySecurity a steer is a denial — the tool
never runs, so the module records `Denied` and the call is released, never
reserved; reserving is irreversible and would consume an approval for something
that will not execute. The contract needs nothing new for it
(`ApprovalChoice2` is `Once | Tool | Flow | Deny`), and the host keeps the
distinction in its own decision label and in `ApprovalDecided`.

The directive never leaves the host: it is not persisted on the pending row,
not published on the bus, and never logged — only its length is. It reaches the
parked call through `Resolution`, which crosses an in-process `oneshot` only.
`ApprovalDecision::Steer` is deliberately fieldless so the enum stays `Copy`
and keeps serializing as a plain `"steer"`.

A `forced` request still accepts only a one-time approval or a denial, so it
cannot be steered: it was escalated precisely because it needs an explicit yes
or no. `approval.decide` rejects `"steer"` as well — a steer without a
directive has nothing to redirect to, so it must come through `approval.steer`.

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
