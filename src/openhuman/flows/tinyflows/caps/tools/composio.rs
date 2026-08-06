//! The default namespace — Composio actions.
//!
//! Claims every slug no earlier backend did, because a Composio action slug
//! (`GMAIL_SEND_EMAIL`) carries no distinguishing prefix. That makes this the
//! catch-all, and the reason [`BACKENDS`](super::BACKENDS) must keep it last.
//!
//! # Gate order is a security property, not a style choice
//!
//! The autonomy-tier gate runs **before** the curation check. A read-only tier
//! is rejected on the tier gate and never reaches curation, so it cannot use
//! the difference between "not curated" and "curated but denied" to probe which
//! slugs exist in the user's configured scope. These two steps move together or
//! not at all.
//!
//! Full order: tier gate → curation/scope → required-arg preflight → approval
//! gate → dispatch. Each rejects before the next does any work, so a call that
//! cannot succeed never reaches the provider and never asks the user to approve
//! it.

use async_trait::async_trait;
use serde_json::Value;
use tinyflows::error::{EngineError, Result};

use super::{ToolBackend, ToolCallCtx};
use crate::openhuman::config::ops as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::integrations::composio::client::{
    create_composio_client, direct_execute, ComposioClientKind,
};
use crate::openhuman::security::GateDecision;

/// Dispatches a Composio action slug through the tier, curation, preflight and
/// approval gates.
pub(crate) struct ComposioToolBackend;

#[async_trait]
impl ToolBackend for ComposioToolBackend {
    fn name(&self) -> &'static str {
        "composio"
    }

    /// Empty: Composio slugs carry no prefix, so this backend is the fallback.
    fn prefix(&self) -> &'static str {
        ""
    }

    /// The required-arg check the real path runs, exposed so a dry run performs
    /// it too. Previously this lived in a second, parallel `ToolInvoker` that
    /// called the Composio preflight directly — which meant the dispatch seam
    /// did not actually contain all Composio knowledge, and a build without
    /// this backend would still have referenced it.
    async fn preflight(&self, config: &Config, slug: &str, args: &Value) -> Result<()> {
        super::super::preflight_composio_args(config, slug, args).await
    }

    async fn invoke(
        &self,
        ctx: &ToolCallCtx<'_>,
        slug: &str,
        args: Value,
        conn: Option<&str>,
    ) -> Result<Value> {
        // A capability object can outlive a Settings change. Reload once and
        // use the same snapshot for curation, contract preflight, client mode,
        // credentials, account resolution, and direct dispatch.
        let live_config = config_rpc::reload_config_snapshot_with_timeout(ctx.config)
            .await
            .map_err(|e| {
                EngineError::Capability(format!(
                    "tool_call node: failed to load live Composio config: {e}"
                ))
            })?;
        // Autonomy-tier gate (Phase 2, made effect-aware): the node's
        // [`CommandClass`] is derived from the action's curated
        // [`ToolScope`](crate::openhuman::memory::sync::composio::providers::ToolScope)
        // via [`classify_composio_action_for_tier`] — a curated Read action
        // (e.g. `TWITTER_RECENT_SEARCH`) is `CommandClass::Read`, which every
        // tier `Allow`s; a curated Write/Admin action, or anything not
        // curated, is `CommandClass::Network` (same class `http_request`
        // uses — fail-safe: only a curated Read skips the prompt). A
        // read-only run `Block`s on a non-Read class here and never reaches
        // curation, the preflight, or the approval gate; Supervised/Full
        // fall through to `gate_call_for_tier` below. Runs BEFORE the
        // curation check (unlike the pre-fix behavior) so a read-only tier
        // can never even probe which slugs are curated.
        let composio_class = super::super::classify_composio_action_for_tier(slug).await;
        let tier_decision =
            super::super::enforce_node_tier_gate(ctx.security, composio_class, "tool_call")?;
        tracing::debug!(
            target: "flows",
            %slug,
            ?composio_class,
            ?tier_decision,
            tier = ?ctx.security.autonomy,
            "[flows] tool_call: composio node tier gate evaluated"
        );

        // Curation + scope gate — hard allowlist (see [`is_curated_flow_tool`]'s
        // doc for why this differs from the general agent tool-call path).
        // Runs before anything else — a rejected slug never reaches the
        // composio client at all.
        if !super::super::is_curated_flow_tool(&live_config, slug).await {
            tracing::warn!(
                target: "flows",
                %slug,
                "[flows] tool_call: rejected — not a recognized curated toolkit action, or out \
                 of the user's configured scope"
            );
            return Err(EngineError::Capability(format!(
                "tool not permitted: {slug}"
            )));
        }

        // Required-arg preflight — fail with an actionable, field-naming error
        // BEFORE the approval gate and the Composio dispatch, so a mis-wired
        // arg (`=`-expression that resolved to null) never reaches the
        // provider or asks the user to approve a call that cannot succeed.
        super::super::preflight_composio_args(&live_config, slug, &args).await?;

        // Approval gate (see the struct doc). Mirrors `OpenHumanHttp::request`'s
        // shape exactly: `gate_call_for_tier` is what actually performs the
        // `Prompt` round-trip — it escalates a Supervised `Prompt` decision
        // into a forced approval regardless of the flow's own
        // `require_approval` toggle (Codex P1), same as the `http_request` and
        // `code` node paths. Deny short-circuits before any composio call;
        // Allow records an audit id to close out after the call resolves.
        //
        // Effect-aware short-circuit: when the tier decision is already
        // `Allow` (a curated Read action — the only `CommandClass` this
        // classifier maps to `Read`), skip `gate_call_for_tier`/
        // `intercept_audited` entirely. This matters
        // because `flows/ops.rs`'s Rule 2 force-sets `require_approval: true`
        // on any saved flow that contains a `tool_call` node, regardless of
        // which actions it calls — without this short-circuit, that forced
        // `Workflow { require_approval: true }` origin would still route a
        // curated-Read `Allow` decision through `intercept_audited`'s normal
        // parking flow, re-introducing the "reads wait for approval" bug via
        // a different path than the one this fix closes at the tier gate.
        // Refining Rule 2 itself to be scope-aware is a deferred follow-up.
        let summary = crate::openhuman::security::approval::summarize_action(slug, &args);
        let redacted = crate::openhuman::security::approval::redact_args(&args);
        let (outcome, audit_id) = if tier_decision == GateDecision::Allow {
            (
                crate::openhuman::security::approval::GateOutcome::Allow,
                None,
            )
        } else {
            super::super::gate_call_for_tier(tier_decision, slug, &summary, redacted).await
        };
        if let crate::openhuman::security::approval::GateOutcome::Deny { reason } = outcome {
            tracing::warn!(
                target: "flows",
                %slug,
                ?tier_decision,
                "[flows] tool_call: approval gate denied before Composio dispatch"
            );
            return Err(EngineError::Capability(reason));
        }

        let kind = create_composio_client(&live_config)
            .map_err(|e| EngineError::Capability(e.to_string()))?;
        let args_opt = if args.is_null() { None } else { Some(args) };
        let connection_id = conn.and_then(super::super::composio_connection_id);

        // Resolve the connection_ref to the SPECIFIC connected account it names,
        // so we can log which account executes and validate it against the
        // user's live connected set. Ambient-session fallback is used ONLY when
        // no connection_ref was supplied.
        let resolved_account = match connection_id {
            Some(id) => Some((
                id,
                super::super::resolve_composio_account(&live_config, id).await,
            )),
            None => None,
        };

        tracing::debug!(
            target: "flows",
            %slug,
            mode = kind.mode(),
            has_connection_ref = connection_id.is_some(),
            "[flows] tool_call: invoking composio tool"
        );

        let response = match kind {
            ComposioClientKind::Backend(client) => {
                if let Some((id, resolved)) = &resolved_account {
                    match resolved {
                        Some((toolkit, _label)) => tracing::warn!(
                            target: "flows",
                            %slug,
                            connection_id = %id,
                            %toolkit,
                            "[flows] tool_call: EXECUTING ON THE WRONG ACCOUNT — connection_ref \
                             names a specific connected account, but backend mode has no per-call \
                             account-scoping path yet, so this call runs against the AMBIENT \
                             signed-in session account instead of the one requested (E-m3, \
                             documented backend-API-gap stub, see caps.rs's OpenHumanTools doc). \
                             Proceeds rather than failing closed."
                        ),
                        None => tracing::warn!(
                            target: "flows",
                            %slug,
                            connection_id = %id,
                            "[flows] tool_call: POSSIBLY EXECUTING ON THE WRONG ACCOUNT — \
                             connection_ref names a connection id not found among the user's live \
                             connected accounts, and backend mode has no per-call account-scoping \
                             path to validate it against anyway — this call runs against the \
                             AMBIENT signed-in session account regardless (E-m3, documented \
                             backend-API-gap stub, see caps.rs's OpenHumanTools doc). Proceeds \
                             rather than failing closed."
                        ),
                    }
                }
                client
                    .execute_tool(slug, args_opt)
                    .await
                    .map_err(|e| EngineError::Capability(e.to_string()))
            }
            ComposioClientKind::Direct(tool) => {
                match &resolved_account {
                    Some((id, Some((toolkit, _label)))) => tracing::info!(
                        target: "flows",
                        %slug,
                        connection_id = %id,
                        %toolkit,
                        "[flows] tool_call: executing against the resolved connected account"
                    ),
                    Some((id, None)) => tracing::warn!(
                        target: "flows",
                        %slug,
                        connection_id = %id,
                        "[flows] tool_call: connection_ref connection_id not found among the user's \
                         live connected accounts (stale cache or foreign id) — forwarding to \
                         Composio Direct mode as-is"
                    ),
                    None => tracing::debug!(
                        target: "flows",
                        %slug,
                        "[flows] tool_call: no connection_ref — using the ambient signed-in account"
                    ),
                }
                direct_execute(
                    &tool,
                    slug,
                    args_opt,
                    &live_config.composio.entity_id,
                    connection_id,
                )
                .await
                .map_err(|e| EngineError::Capability(e.to_string()))
            }
        };

        // A successful HTTP round-trip can still carry a provider-side failure
        // (`{successful: false, error: "..."}`, e.g. a Slack 400 on
        // `SLACK_SEND_MESSAGE`) — reject it into a real capability error, see
        // `reject_unsuccessful_composio_response`'s doc.
        let response = response
            .and_then(|resp| super::super::reject_unsuccessful_composio_response(slug, resp));

        if let Some(id) = audit_id {
            if let Some(gate) = crate::openhuman::security::approval::ApprovalGate::try_global() {
                let exec = if response.is_ok() {
                    crate::openhuman::security::approval::ExecutionOutcome::Success
                } else {
                    crate::openhuman::security::approval::ExecutionOutcome::Failure
                };
                gate.record_execution(
                    &id,
                    exec,
                    response.as_ref().err().map(ToString::to_string).as_deref(),
                );
            }
        }

        serde_json::to_value(response?).map_err(|e| EngineError::Capability(e.to_string()))
    }
}
