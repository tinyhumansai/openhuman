//! OpenHuman policy and event adapters for TinyAgents' plan-review store.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tinyagents_harness::middleware::RunMode;
use tinyagents_harness_tools::plan_review::{
    InteractiveReviewSurface, ParkedReview, PlanReviewHost, PlanReviewNotifier, PlanReviewReason,
    PlanReviewRequest, PlanReviewResolution, PlanReviewStore,
};
use tinytools::ToolRunContext;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::runtime::turn_origin::AgentTurnOrigin;
use crate::core::runtime::{DispatchCtx, Tenant};

struct DomainEventNotifier;

impl PlanReviewNotifier<Tenant> for DomainEventNotifier {
    fn requested(&self, review: &ParkedReview<Tenant>) {
        BUS.publish(DomainEvent::PlanReviewRequested {
            request_id: review.request_id.clone(),
            thread_id: Some(review.thread_id.clone()),
            client_id: review.client_id.clone(),
            summary: review.summary.clone(),
            steps: review.steps.clone(),
            tool_call_id: review.tool_call_id.clone(),
            expires_at: Some(review.expires_at.clone()),
        });
    }

    fn decided(
        &self,
        review: &ParkedReview<Tenant>,
        resolution: &PlanReviewResolution,
        reason: PlanReviewReason,
    ) {
        let terminal_reason = match reason {
            PlanReviewReason::Expired => Some("expired"),
            PlanReviewReason::Cancelled => Some("cancelled"),
            PlanReviewReason::Approved | PlanReviewReason::Rejected | PlanReviewReason::Revised => {
                None
            }
        };
        BUS.publish(DomainEvent::PlanReviewDecided {
            request_id: review.request_id.clone(),
            decision: resolution.as_str().to_owned(),
            thread_id: Some(review.thread_id.clone()),
            client_id: review.client_id.clone(),
            tool_call_id: review.tool_call_id.clone(),
            resolution: terminal_reason.map(str::to_owned),
        });
    }
}

/// Process-wide review store. Its keys are normalized to a profile so that
/// the profile UI can decide or replay a request created by any of its agents.
#[doc(hidden)]
pub fn global() -> &'static PlanReviewStore<Tenant> {
    static STORE: OnceLock<PlanReviewStore<Tenant>> = OnceLock::new();
    STORE.get_or_init(|| PlanReviewStore::new(Arc::new(DomainEventNotifier)))
}

struct OpenHumanPlanReviewHost;

#[async_trait]
impl PlanReviewHost for OpenHumanPlanReviewHost {
    fn interactive_surface(
        &self,
        context: Option<&dyn ToolRunContext>,
    ) -> Option<InteractiveReviewSurface> {
        let origin = crate::agent::tinyagents::host::run_context::turn_origin_for_tool(context)?;
        matches!(origin, AgentTurnOrigin::WebChat { .. }).then_some(InteractiveReviewSurface)
    }

    fn thread_id(&self, context: Option<&dyn ToolRunContext>) -> Option<String> {
        crate::agent::tinyagents::host::run_context::approval_for_tool(context)
            .chat
            .map(|chat| chat.thread_id)
    }

    fn run_mode(&self, context: Option<&dyn ToolRunContext>, thread_id: Option<&str>) -> RunMode {
        thread_id.map_or(RunMode::Build, |thread_id| {
            crate::threads::run_mode::get_mode(
                &crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context),
                thread_id,
            )
        })
    }

    fn tool_call_id(&self, context: Option<&dyn ToolRunContext>) -> Option<String> {
        crate::tools::host_extensions::tool_call_id(context)
    }

    async fn request_review(
        &self,
        request: PlanReviewRequest,
        context: Option<&dyn ToolRunContext>,
    ) -> Result<PlanReviewResolution, String> {
        let chat = crate::agent::tinyagents::host::run_context::approval_for_tool(context)
            .chat
            .ok_or_else(|| "plan review has no originating chat".to_owned())?;
        let dispatch = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        let tenant = dispatch
            .tenant()
            .map_err(|error| error.to_string())?
            .profile_only();
        Ok(global()
            .request_review(tenant, chat.thread_id, Some(chat.client_id), request)
            .await)
    }
}

/// The harness-owned tool configured with OpenHuman's turn, tenant and event policy.
type RequestPlanReviewTool = tinyagents_harness_tools::plan_review::RequestPlanReviewTool;

pub(crate) fn request_plan_review_tool() -> RequestPlanReviewTool {
    tinyagents_harness_tools::plan_review::RequestPlanReviewTool::new(Some(Arc::new(
        OpenHumanPlanReviewHost,
    )))
}

/// Server-captured tenant for replay to a socket that joins a thread room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketReviewScope(Tenant);

/// Captures replay scope from trusted request state, never from socket payloads.
pub fn socket_scope_for_dispatch(dispatch: &DispatchCtx) -> Option<SocketReviewScope> {
    match dispatch.tenant() {
        Ok(tenant) => Some(SocketReviewScope(tenant)),
        Err(_) if crate::core::runtime::is_saas() => None,
        Err(_) => Some(SocketReviewScope(Tenant::default())),
    }
}

/// Returns the current review for a socket's captured profile scope.
pub fn parked_review_for_socket(
    scope: &SocketReviewScope,
    thread_id: &str,
) -> Option<ParkedReview<Tenant>> {
    global().parked_review_for_thread(&scope.0.profile_only(), thread_id)
}

/// Resolves a request in the profile belonging to the dispatch context.
pub(crate) fn decide(
    dispatch: &DispatchCtx,
    request_id: &str,
    resolution: PlanReviewResolution,
) -> Result<bool, String> {
    let tenant = dispatch
        .tenant()
        .map_err(|error| error.to_string())?
        .profile_only();
    Ok(global().decide(&tenant, request_id, resolution))
}

/// Checks whether this profile has a pending review for the thread.
pub(crate) fn has_parked_review(tenant: &Tenant, thread_id: &str) -> bool {
    global().has_parked_review(&tenant.profile_only(), thread_id)
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
