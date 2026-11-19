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

**Requests an earlier launch left waiting.** The one-time legacy import
carries an undecided request that is still inside its window into TinySecurity
as a pending orphan: a pending row no host instance owns, bound to a synthetic
call nothing can run. `approval_list_pending` lists orphans for the default
tenant after the requests this process parked. A person can only deny one
(`gate_orphans.rs`, `native/engine_orphans.rs`): the host verifies that denial
against the listed row byte for byte, and TinySecurity accepts nothing else for
a row another instance owns. Approving one is refused, and it stays listed until
it is denied or its window closes.

**SaaS secret master grant.** A SaaS core has no person to ask when a headless
tenant request (a gateway call, a background or delivery job, the embed
`profiles` API) must unlock that tenant's config secrets, which needs
TinySecurity's one-time (HumanOnce) master grant. `gate_saas_grant.rs` lets the
host approve that one grant, and only it, before the ordinary SaaS allowlist
decision (`profiles::tools::gate_verdict`):

- only in SaaS, only for a forced `native_secret_grant` call, and only when no
  person will decide it on a routable WebChat turn;
- only when the dispatch is bound to an authenticated tenant profile, the host's
  profile layout places that tenant's `.secret_key`, and the live secret
  registration named by `registration_id` is that same key. Another tenant's
  scope, the operator's scope, an unknown registration or an unbound dispatch
  is refused (fail closed);
- the approval runs the engine's own `check` → `decide` round trip, so
  TinySecurity verifies and audits it on the tenant's secret scope stream under
  the label and actor `saas_host_authorised`. Any other engine answer denies,
  and a caller dropped mid-grant cancels the request it left pending.

Outside SaaS a person still decides the secret master grant.

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
