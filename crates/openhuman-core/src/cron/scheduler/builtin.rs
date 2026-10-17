//! The default job-kind handlers: shell, agent and flow.
//!
//! These are what [`crate::cron::handlers`] holds until a host registers its
//! own. Each is the thin adapter between a handler call and the code that
//! already did the work: the sandboxed shell runner ([`super::shell_job`]), a
//! full agent turn ([`super::agent_run`]) and the flow / system-job dispatch
//! below.

use std::sync::Arc;

use async_trait::async_trait;
use tinyflows_bus::schedule::types::{CronJob, JobType};

use super::agent_run::run_agent_job_for_run;
use super::shell_job::run_job_command;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::cron::handlers::{JobKindHandler, JobOutcome, JobRunContext};

/// The built-in handler of every job kind.
pub(crate) fn defaults() -> Vec<(JobType, Arc<dyn JobKindHandler>)> {
    vec![
        (JobType::Shell, Arc::new(ShellHandler)),
        (JobType::Agent, Arc::new(AgentHandler)),
        (JobType::Flow, Arc::new(FlowHandler)),
    ]
}

/// `shell`: the command through the sandbox runner under the security policy.
struct ShellHandler;

#[async_trait]
impl JobKindHandler for ShellHandler {
    async fn run(&self, cx: &JobRunContext, job: &CronJob, _run_id: &str) -> JobOutcome {
        let (success, output) = run_job_command(
            &cx.dispatch_ctx(),
            &cx.config(),
            cx.security().as_ref(),
            job,
        )
        .await;
        JobOutcome {
            success,
            output,
            agent_error: None,
            permanent: false,
        }
    }
}

/// `agent`: a full agent turn; its own error text feeds failure classification.
struct AgentHandler;

#[async_trait]
impl JobKindHandler for AgentHandler {
    async fn run(&self, cx: &JobRunContext, job: &CronJob, run_id: &str) -> JobOutcome {
        let (success, output, agent_error) =
            run_agent_job_for_run(&cx.dispatch_ctx(), &cx.config(), job, run_id).await;
        JobOutcome {
            success,
            output,
            agent_error,
            permanent: false,
        }
    }
}

/// `flow`: a `system:<name>` job, or the schedule tick of the bound flow.
struct FlowHandler;

#[async_trait]
impl JobKindHandler for FlowHandler {
    async fn run(&self, cx: &JobRunContext, job: &CronJob, run_id: &str) -> JobOutcome {
        let flow_id = job.command.as_str();
        if let Some(name) = crate::cron::system_jobs::system_command_name(flow_id) {
            return run_system_job(&cx.dispatch_ctx(), &job.id, name, run_id).await;
        }
        // `[components] flows = false`: schedule-triggered flows stop while
        // agent, shell and system jobs keep running. The tick is answered, not
        // retried, so the row's recurrence moves on.
        if !cx.dispatch_ctx().components().flows {
            tracing::debug!(
                target: "flows",
                %flow_id,
                %run_id,
                "[cron] flow schedule trigger skipped: component 'flows' disabled"
            );
            return JobOutcome::ok(format!(
                "flow schedule tick skipped for flow {flow_id}: COMPONENT_DISABLED:flows"
            ));
        }
        tracing::info!(
            target: "flows",
            %flow_id,
            %run_id,
            "[cron] flow schedule trigger fired"
        );
        #[cfg(feature = "flows")]
        cx.binding()
            .flows()
            .dispatch_schedule_tick(&cx.dispatch_ctx(), flow_id)
            .await;
        #[cfg(not(feature = "flows"))]
        tracing::warn!(%flow_id, "[cron] built without `flows`; schedule trigger dropped");
        JobOutcome::ok(format!("flow schedule tick emitted for flow {flow_id}"))
    }
}

/// A host-owned system job (`system:<name>`): announced on the bus, then its
/// registered handler is awaited and its result is the run's result. Without a
/// handler the job is announce-only and `ok` on dispatch.
async fn run_system_job(
    ctx: &crate::core::runtime::DispatchCtx,
    job_id: &str,
    name: &str,
    run_id: &str,
) -> JobOutcome {
    tracing::info!(%job_id, job = %name, %run_id, "[cron] system job due — publishing CronSystemJobDue");
    BUS.publish(DomainEvent::CronSystemJobDue {
        job: name.to_string(),
        tenant: ctx.event_tenant(),
    });
    let ctx = crate::cron::system_job_handlers::SystemJobContext {
        job_id: job_id.to_string(),
        name: name.to_string(),
    };
    match crate::cron::system_job_handlers::dispatch(ctx).await {
        Some(Ok(())) => JobOutcome::ok(format!("system job {name} completed")),
        Some(Err(error)) => JobOutcome::failed(format!("system job {name} failed: {error}")),
        None => JobOutcome::ok(format!("system job {name} dispatched")),
    }
}
