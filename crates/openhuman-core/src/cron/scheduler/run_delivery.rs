//! Host adapters for TinyFlows' send, terminal projection and trigger ports.

use super::delivery::deliver_run;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::cron::bus_host::{kind_refusal, Binding};
use crate::cron::handlers::{kind_handler, JobRunContext};
use anyhow::Result;
use tinyflows_bus::schedule::types::{CronJob, DeliveryStatus, JobType};
use tinyflows_bus::schedule::{Attempt, DeliveryProjection};

/// Perform the actual product delivery exactly once.
pub(crate) async fn send(
    binding: &Binding,
    job: &CronJob,
    run_id: &str,
    attempt: &Attempt,
) -> Result<DeliveryStatus> {
    binding
        .within(async {
            let config = binding.config();
            let ctx = binding.dispatch_ctx();
            deliver_run(&ctx, &config, job, run_id, &attempt.output, attempt.success).await
        })
        .await
}

/// Project the schedule module's authoritative terminal decision and event.
pub(crate) async fn deliver(binding: &Binding, projection: &DeliveryProjection) {
    binding
        .within(async {
            let ctx = binding.dispatch_ctx();
            binding.finish_triggered_run(&projection.run_id);
            publish_completion(
                &ctx,
                &projection.job,
                projection.success,
                &projection.attempt.output,
            );
        })
        .await;
}

/// Publish the final status shared by scheduled and manual runs.
pub(crate) fn publish_completion(
    ctx: &crate::core::runtime::DispatchCtx,
    job: &CronJob,
    success: bool,
    output: &str,
) {
    crate::cron::completion_owner::note(ctx, &job.id);
    BUS.publish(DomainEvent::CronJobCompleted {
        job_id: job.id.clone(),
        success,
        output: crate::util::truncate_with_ellipsis(output, 512),
        tenant: ctx.event_tenant(),
    });
    BUS.publish(DomainEvent::HealthChanged {
        component: "scheduler".to_string(),
        healthy: success,
        message: (!success).then(|| {
            if output.is_empty() {
                format!("job {} failed", job.id)
            } else {
                crate::util::truncate_with_ellipsis(output, 256)
            }
        }),
    });
}

/// Fire the captured flow definition through the registered kind handler.
pub(crate) async fn trigger(binding: &Binding, job: &CronJob, run_id: &str) -> Attempt {
    binding
        .within(async {
            binding.publish_triggered_once(job, run_id);
            let config = binding.config();
            if let Err(error) = crate::cron::limits::check_run(&config, job) {
                return Attempt {
                    success: false,
                    output: error.to_string(),
                    retryable: false,
                    raw_detail: None,
                };
            }
            if let Some(refusal) = kind_refusal(binding, job) {
                return Attempt {
                    success: false,
                    output: refusal,
                    retryable: false,
                    raw_detail: None,
                };
            }
            let Some(handler) = kind_handler(&JobType::Flow) else {
                return Attempt {
                    success: false,
                    output: "no handler is registered for flow jobs".to_string(),
                    retryable: false,
                    raw_detail: None,
                };
            };
            let outcome = handler
                .run(&JobRunContext::new(binding.clone()), job, run_id)
                .await;
            Attempt {
                success: outcome.success,
                output: outcome.output,
                retryable: !outcome.permanent,
                raw_detail: outcome.agent_error,
            }
        })
        .await
}
