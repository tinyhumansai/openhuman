//! The host's answers to the module's `Deliver` and `ScheduleTrigger`
//! callbacks.
//!
//! Delivery sends a finished run's output to its chat, channel or origin and
//! raises the alert (`delivery.rs`), then announces the completion on the bus.
//! The trigger fires a `flow` row through the job-kind registry
//! ([`crate::cron::handlers`]): the default `flow` handler starts the flow's
//! schedule tick directly in the flows domain, or runs a host-owned system
//! job, with no domain event in between.

use super::delivery::deliver_run;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::cron::bus_host::{kind_refusal, Binding};
use crate::cron::handlers::{kind_handler, JobRunContext};
use anyhow::Result;
use chrono::Utc;
use tinyflows_bus::schedule::types::{
    CronJob, DeliveryConfig, DeliveryStatus, JobPolicy, JobType, Schedule, SessionTarget,
};
use tinyflows_bus::schedule::{Attempt, StoreOperation};

/// Deliver `attempt`'s output for `job` and publish the run's completion.
pub(crate) async fn deliver(
    binding: &Binding,
    job: &CronJob,
    run_id: &str,
    attempt: &Attempt,
) -> Result<DeliveryStatus> {
    binding
        .within(async {
            let config = binding.config();
            let ctx = binding.dispatch_ctx();
            let result =
                deliver_run(&ctx, &config, job, run_id, &attempt.output, attempt.success).await;
            // A failed required delivery fails the run, as the module
            // records it.
            let delivered = match &result {
                Ok(status) => *status != DeliveryStatus::Failed || job.delivery.best_effort,
                Err(_) => job.delivery.best_effort,
            };
            let success = attempt.success && delivered;
            // The job ran as its agent (when it had one); a one-shot job
            // is deleted once the run completes, so subscribers learn the
            // owner here, not from the store.
            crate::cron::completion_owner::note(&ctx, &job.id);
            BUS.publish(DomainEvent::CronJobCompleted {
                job_id: job.id.clone(),
                success,
                output: crate::util::truncate_with_ellipsis(&attempt.output, 512),
            });
            BUS.publish(DomainEvent::HealthChanged {
                component: "scheduler".to_string(),
                healthy: success,
                message: (!success).then(|| {
                    if attempt.output.is_empty() {
                        format!("job {} failed", job.id)
                    } else {
                        crate::util::truncate_with_ellipsis(&attempt.output, 256)
                    }
                }),
            });
            result
        })
        .await
}

/// A `flow` row standing in for a trigger whose stored row cannot be read (it
/// was removed between the claim and the tick). The handler needs only the
/// bound flow id, which is the row's `command`.
fn stand_in(flow_id: &str) -> CronJob {
    let now = Utc::now();
    CronJob {
        claim_generation: 0,
        policy: JobPolicy::default(),
        id: flow_id.to_string(),
        expression: String::new(),
        schedule: Schedule::Every { every_ms: 60_000 },
        command: flow_id.to_string(),
        prompt: None,
        name: None,
        job_type: JobType::Flow,
        session_target: SessionTarget::default(),
        model: None,
        agent_id: None,
        enabled: true,
        delivery: DeliveryConfig::default(),
        delete_after_run: false,
        created_at: now,
        next_run: now,
        last_run: None,
        last_status: None,
        last_output: None,
        origin: None,
    }
}

/// A stand-in `flow` row with `command`, for tests of handlers.
#[cfg(test)]
pub(crate) fn stand_in_for_test(command: &str) -> CronJob {
    stand_in(command)
}

/// Fire the `flow` row bound to `flow_id` through the registered `flow`
/// handler.
pub(crate) async fn trigger(binding: &Binding, flow_id: &str, run_id: &str) -> Attempt {
    binding
        .within(async {
            if let Some(refusal) = kind_refusal(binding, &JobType::Flow) {
                return Attempt {
                    success: false,
                    output: refusal,
                    retryable: false,
                };
            }
            let config = binding.config();
            let job = match crate::cron::client::store::<Option<CronJob>>(
                &config,
                StoreOperation::FindFlow {
                    flow_id: flow_id.to_string(),
                },
            )
            .await
            {
                Ok(Some(job)) => job,
                Ok(None) => stand_in(flow_id),
                Err(error) => {
                    tracing::debug!(%flow_id, %error, "[cron:trigger] flow row unreadable; using a stand-in");
                    stand_in(flow_id)
                }
            };
            let Some(handler) = kind_handler(&JobType::Flow) else {
                return Attempt {
                    success: false,
                    output: "no handler is registered for flow jobs".to_string(),
                    retryable: false,
                };
            };
            let outcome = handler
                .run(&JobRunContext::new(binding.clone()), &job, run_id)
                .await;
            Attempt {
                success: outcome.success,
                output: outcome.output,
                retryable: !outcome.permanent,
            }
        })
        .await
}
