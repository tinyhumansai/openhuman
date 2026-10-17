//! Typed calls to the `tinyflows` module's `Schedule` object.
//!
//! The scheduler runtime, the cron job and run store and the schedule maths
//! all run in the module (`tinyflows_bus::schedule`, contract v3). This file is
//! the only place core builds those requests: it turns a [`Config`] and a
//! [`StoreOperation`] into a `ScheduleStore` request bound to the acting
//! scope's document port ([`super::documents_port`]), and wraps every pure
//! member (`NextRun`, `ValidateSchedule`, `RunsCloserThan`, ...) in a typed
//! function. Members are named through the contract constants; nothing here
//! parses a cron expression or computes a next run.
//!
//! Every call logs `[cron:bus]` with the member (never a payload) so a failing
//! call is greppable.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tinyflows_bus::schedule::{
    members, CheckOriginRequest, CheckPatchRequest, DeliveryConfig, JobOrigin, NextRunRequest,
    RunsCloserThanRequest, Schedule, SessionTarget, StoreBackend, StoreConfig, StoreOperation,
    StoreRequest, TooFrequent, ValidateScheduleRequest,
};
use tinyflows_bus::schedule::{CronJob, CronJobPatch};

use super::documents_port::{self, DocScope};
use crate::config::Config;
use crate::modules::tinyflows as module;

/// The bus failure as the module's own message, so callers (and tests) that
/// match on `Cron job '<id>' not found` still see it.
fn wire(error: &tinybus::Error) -> anyhow::Error {
    anyhow!(error.wire_message())
}

async fn call<R: DeserializeOwned>(
    config: &Config,
    member: &'static str,
    args: impl Serialize,
) -> Result<R> {
    let proxy = module::proxy(config).await.map_err(|e| anyhow!(e))?;
    tracing::trace!(member, "[cron:bus] call");
    proxy.call(member, args).await.map_err(|error| {
        tracing::debug!(member, error = %error.wire_message(), "[cron:bus] call failed");
        wire(&error)
    })
}

/// The `StoreConfig` for `config`: the scope's document port and the host's
/// history and batch limits.
async fn store_config(config: &Config, scope: &DocScope) -> Result<StoreConfig> {
    Ok(StoreConfig {
        backend: StoreBackend::Documents {
            callback: documents_port::address(scope).await?,
        },
        max_run_history: config.cron.max_run_history,
        max_tasks: config.scheduler.max_tasks,
    })
}

/// Run one store operation in `scope` and decode its reply.
pub(crate) async fn store_in<R: DeserializeOwned>(
    config: &Config,
    scope: &DocScope,
    operation: StoreOperation,
) -> Result<R> {
    let request = StoreRequest {
        storage: store_config(config, scope).await?,
        operation: serde_json::to_value(&operation)?,
    };
    let value: Value = call(config, members::SCHEDULE_STORE, (request,)).await?;
    serde_json::from_value(value).map_err(|error| {
        tracing::warn!(%error, "[cron:bus] undecodable ScheduleStore reply");
        anyhow!("the tinyflows module answered a store request with an unexpected shape")
    })
}

/// Run one store operation in the acting scope (`current_scope()`).
pub(crate) async fn store<R: DeserializeOwned>(
    config: &Config,
    operation: StoreOperation,
) -> Result<R> {
    let scope = DocScope::current(config)?;
    store_in(config, &scope, operation).await
}

/// `ParseHumanDelay`: `"30m"`, `"2h"`, ... as a duration.
pub(crate) async fn parse_human_delay(config: &Config, delay: &str) -> Result<Duration> {
    let millis: i64 = call(config, members::PARSE_HUMAN_DELAY, (delay,)).await?;
    Duration::try_milliseconds(millis).ok_or_else(|| anyhow!("delay '{delay}' is out of range"))
}

/// `ValidateDelivery`: the module's rules plus the channel's `allowed_users`
/// the host passes in.
pub(crate) async fn validate_delivery(
    config: &Config,
    delivery: &DeliveryConfig,
    origin: Option<&JobOrigin>,
    allowed_users: Option<&[String]>,
) -> Result<()> {
    call(
        config,
        members::VALIDATE_DELIVERY,
        (delivery, origin, allowed_users),
    )
    .await
}

/// `DefaultDelivery`: origin delivery with an origin, proactive without.
pub(crate) async fn default_delivery(
    config: &Config,
    origin: Option<&JobOrigin>,
) -> Result<DeliveryConfig> {
    call(config, members::DEFAULT_DELIVERY, (origin,)).await
}

/// `CheckOriginRequirements`.
pub(crate) async fn check_origin_requirements(
    config: &Config,
    session_target: &SessionTarget,
    delivery: &DeliveryConfig,
    origin: Option<&JobOrigin>,
) -> Result<()> {
    let request = CheckOriginRequest {
        session_target: session_target.clone(),
        delivery: delivery.clone(),
        origin: origin.cloned(),
    };
    call(config, members::CHECK_ORIGIN_REQUIREMENTS, (request,)).await
}

/// `CheckPatch`: refuse a patch no store may apply to `job`.
pub(crate) async fn check_patch(
    config: &Config,
    job: &CronJob,
    patch: &CronJobPatch,
) -> Result<()> {
    let request = CheckPatchRequest {
        job: job.clone(),
        patch: patch.clone(),
    };
    call(config, members::CHECK_PATCH, (request,)).await
}

/// `NextRun`: the first occurrence of `schedule` strictly after `from`.
pub(crate) async fn next_run(
    config: &Config,
    schedule: &Schedule,
    from: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    let request = NextRunRequest {
        schedule: schedule.clone(),
        from,
    };
    call(config, members::NEXT_RUN, (request,)).await
}

/// `ValidateSchedule`; `agent` adds the agent-job cadence floor.
pub(crate) async fn validate_schedule(
    config: &Config,
    schedule: &Schedule,
    now: DateTime<Utc>,
    agent: bool,
) -> Result<()> {
    let request = ValidateScheduleRequest {
        schedule: schedule.clone(),
        now,
        agent,
    };
    call(config, members::VALIDATE_SCHEDULE, (request,)).await
}

/// `RunsCloserThan`: the tightest pair of consecutive runs under `min_gap`.
pub(crate) async fn runs_closer_than(
    config: &Config,
    schedule: &Schedule,
    from: DateTime<Utc>,
    min_gap: Duration,
) -> Result<Option<TooFrequent>> {
    let request = RunsCloserThanRequest {
        schedule: schedule.clone(),
        from,
        min_gap_ms: min_gap.num_milliseconds(),
    };
    call(config, members::RUNS_CLOSER_THAN, (request,)).await
}

/// `NormalizeExpression`: POSIX weekdays into the `cron` crate's numbering.
pub(crate) async fn normalize_expression(config: &Config, expression: &str) -> Result<String> {
    call(config, members::NORMALIZE_EXPRESSION, (expression,)).await
}

/// `ScheduleCronExpression`: the cron expression a schedule has, if any.
pub(crate) async fn schedule_cron_expression(
    config: &Config,
    schedule: &Schedule,
) -> Result<Option<String>> {
    call(config, members::SCHEDULE_CRON_EXPRESSION, (schedule,)).await
}

/// `TruncateCronOutput`: output bounded to `MAX_CRON_OUTPUT_BYTES`.
pub(crate) async fn truncate_cron_output(config: &Config, output: &str) -> Result<String> {
    call(config, members::TRUNCATE_CRON_OUTPUT, (output,)).await
}

/// [`store`] for a synchronous caller (the cron store API is synchronous). The
/// acting scope is resolved here, on the caller's task, because the bridge
/// thread the call hops to carries no task context.
pub(crate) fn store_blocking<R: DeserializeOwned + Send + 'static>(
    config: &Config,
    operation: StoreOperation,
) -> Result<R> {
    let scope = DocScope::current(config)?;
    blocking(config, move |config| async move {
        store_in(&config, &scope, operation).await
    })
}

/// A synchronous caller's bridge to the async calls above. Anything that needs
/// the acting scope must be captured before the hop.
pub(crate) fn blocking<T, F, Fut>(config: &Config, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(Config) -> Fut,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
{
    crate::storage::block_on_anyhow(f(config.clone()))
}

#[cfg(test)]
#[path = "client_testing.rs"]
pub(crate) mod testing;

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
