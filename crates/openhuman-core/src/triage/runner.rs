//! [`TriageRunner`]: the two triage steps a caller runs on a trigger, behind
//! one seam.
//!
//! Every inbound path classifies a [`TriggerEnvelope`] ([`run_triage`]) and,
//! on a decision, applies it ([`apply_decision`]) under the remote-trigger
//! turn origin, because a trigger body is attacker-influenceable (#5634). The
//! direct paths (the Composio trigger subscriber and agent webhooks) and the
//! inbound event queues' `Deliver` callback all go through this trait, so
//! their behavior is testable without a model: tests script the outcomes, and
//! production uses [`RemoteTriage`].
//!
//! [`RemoteTriage`] carries its provenance in its name: it applies every
//! decision under [`remote_trigger_origin`], so it is only for payloads an
//! outsider can influence. A locally initiated trigger keeps calling
//! [`apply_decision`] under `local_trigger_origin` itself. The dispatch-site
//! tripwire (`origin_tests::every_dispatch_site_scopes_an_origin`) counts a
//! file that constructs `RemoteTriage` as a remote dispatch site.

use async_trait::async_trait;

use super::{
    apply_decision, remote_trigger_origin, run_triage, TriageOutcome, TriageRun, TriggerEnvelope,
};
use crate::core::runtime::turn_origin::with_origin;
use crate::core::runtime::DispatchCtx;

/// Classify a trigger and apply a decision.
#[async_trait]
pub trait TriageRunner: Send + Sync {
    /// Classify `envelope` (the triage evaluator chain).
    ///
    /// # Errors
    ///
    /// The evaluator's own failure (provider, parse, budget).
    async fn run(
        &self,
        ctx: &DispatchCtx,
        envelope: &TriggerEnvelope,
    ) -> anyhow::Result<TriageOutcome>;

    /// Apply a decision for `envelope` under the remote-trigger turn origin.
    ///
    /// # Errors
    ///
    /// The escalation's failure (approval denied is not an error).
    async fn apply(
        &self,
        ctx: &DispatchCtx,
        run: TriageRun,
        envelope: &TriggerEnvelope,
    ) -> anyhow::Result<()>;
}

/// The production runner for remote payloads: [`run_triage`], and
/// [`apply_decision`] under [`remote_trigger_origin`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoteTriage;

#[async_trait]
impl TriageRunner for RemoteTriage {
    async fn run(
        &self,
        ctx: &DispatchCtx,
        envelope: &TriggerEnvelope,
    ) -> anyhow::Result<TriageOutcome> {
        run_triage(ctx, envelope).await
    }

    async fn apply(
        &self,
        ctx: &DispatchCtx,
        run: TriageRun,
        envelope: &TriggerEnvelope,
    ) -> anyhow::Result<()> {
        // `AGENT_TURN_ORIGIN` is a task-local: scope it here, in whichever
        // task applies the decision, so a caller that spawned cannot lose it.
        let origin = remote_trigger_origin(envelope);
        with_origin(origin, apply_decision(ctx, run, envelope)).await
    }
}
