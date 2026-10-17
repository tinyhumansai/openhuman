//! Scheduling: OpenHuman's own cron, driven from the library.
//!
//! [`Runtime::cron`](crate::Runtime::cron) returns a [`Cron`] facade over the
//! runtime's job store. A job is named, and [`Cron::upsert`] is idempotent by
//! that name, so a host can declare its schedule on every start:
//!
//! ```no_run
//! # async fn demo(runtime: &openhuman_embed::Runtime) -> Result<(), openhuman_embed::CronError> {
//! use openhuman_embed::{JobSchedule, JobSpec};
//!
//! let cron = runtime.cron();
//! // A turn of the runtime agent `teeny` every morning, with its host tools.
//! cron.upsert(
//!     JobSpec::agent("morning", "teeny", "Plan the day.", JobSchedule::Cron {
//!         expr: "0 8 * * *".into(),
//!         tz: Some("Europe/Berlin".into()),
//!     })
//!     .retries(0)
//!     .single_flight(true),
//! )?;
//! // A job whose work is the host's own code.
//! runtime.on_system_job("digest", |_ctx| async { Ok(()) })?;
//! cron.upsert(JobSpec::system("digest", "digest", JobSchedule::Every { ms: 15 * 60_000 }))?;
//! # Ok(()) }
//! ```
//!
//! # Targets
//!
//! - [`JobTarget::Agent`] runs one turn of the agent with that id. When the id
//!   names an agent alive on this runtime ([`Runtime::agent`](crate::Runtime::agent)),
//!   the turn runs *as that agent*: its definition and system prompt, its
//!   [`AgentSpec::tools`](crate::AgentSpec::tools) and attached tools, its
//!   provider and its context. Otherwise the id resolves through the core's
//!   agent registries. Either way the turn's origin is
//!   `TrustedAutomation { Cron }`, not the agent's own access origin.
//! - [`JobTarget::System`] runs the handler registered with
//!   [`Runtime::on_system_job`](crate::Runtime::on_system_job) and records
//!   its result. With no handler the run is announced on the bus
//!   (`CronSystemJobDue`) and recorded `ok`.
//!
//! # When jobs fire
//!
//! Only while the scheduler runs: build the runtime with
//! `ServiceSet { cron: true, .. }` (or call
//! [`Runtime::start_services`](crate::Runtime::start_services)). Without it
//! jobs are stored and [`Cron::run_now`] still works, but nothing fires on
//! its own.
//!
//! # Retries and overlap
//!
//! A failed run is retried `retries` times with backoff (`None` keeps the
//! runtime's `reliability.scheduler_retries`, two by default; `Some(0)` is one
//! attempt — choose it for a job whose turn has side effects). Two runs of one
//! job never overlap: [`Cron::run_now`] is refused while one is active, and a
//! slot that comes due mid-run is skipped — silently by default, recorded as
//! a `skipped` run with `single_flight`.

use std::future::Future;
use std::time::SystemTime;

use openhuman_core::config::Config;
use openhuman_core::cron::system_jobs::SYSTEM_COMMAND_PREFIX;
use openhuman_core::cron::JobPolicy;
use {
    openhuman_core::cron::CronJob, openhuman_core::cron::CronJobPatch,
    openhuman_core::cron::JobType, openhuman_core::cron::Schedule,
};

pub use openhuman_core::cron::handlers::{JobKindHandler, JobOutcome, JobRunContext};
pub use openhuman_core::cron::system_job_handlers::SystemJobContext;
/// The kind of a scheduled job (`shell`, `agent`, `flow`), which decides the
/// handler that runs it: see
/// [`RuntimeBuilder::cron_job_handler`](crate::RuntimeBuilder::cron_job_handler).
pub use openhuman_core::cron::JobType as JobKind;

/// When a job fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobSchedule {
    /// A five-field cron expression, in `tz` (an IANA name) or UTC.
    Cron {
        /// The expression, e.g. `"0 8 * * *"`.
        expr: String,
        /// IANA time zone; `None` is UTC.
        tz: Option<String>,
    },
    /// Every `ms` milliseconds. An agent job may not run more often than
    /// every five minutes.
    Every {
        /// The interval in milliseconds.
        ms: u64,
    },
    /// Once, at `at`. Disabled after it runs.
    At {
        /// The instant to run at; must be in the future.
        at: SystemTime,
    },
}

impl JobSchedule {
    pub(crate) fn to_core(&self) -> Schedule {
        match self {
            Self::Cron { expr, tz } => Schedule::Cron {
                expr: expr.clone(),
                tz: tz.clone(),
                active_hours: None,
            },
            Self::Every { ms } => Schedule::Every { every_ms: *ms },
            Self::At { at } => Schedule::At {
                at: chrono::DateTime::<chrono::Utc>::from(*at),
            },
        }
    }

    pub(crate) fn from_core(schedule: &Schedule) -> Self {
        match schedule {
            Schedule::Cron { expr, tz, .. } => Self::Cron {
                expr: expr.clone(),
                tz: tz.clone(),
            },
            Schedule::Every { every_ms } => Self::Every { ms: *every_ms },
            Schedule::At { at } => Self::At {
                at: SystemTime::from(*at),
            },
        }
    }
}

/// What a job runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobTarget {
    /// One turn of agent `agent_id` with `prompt`.
    Agent {
        /// A runtime agent's id, or a registry agent's.
        agent_id: String,
        /// The message the turn is driven by.
        prompt: String,
    },
    /// The host's handler for system job `name`.
    System {
        /// The name handed to [`Runtime::on_system_job`](crate::Runtime::on_system_job).
        name: String,
    },
}

impl JobTarget {
    /// The target of a stored job; `None` for shell jobs and workflow
    /// schedule triggers, which this facade does not manage.
    pub(crate) fn from_job(job: &CronJob) -> Option<Self> {
        match job.job_type {
            JobType::Agent => Some(Self::Agent {
                agent_id: job
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "orchestrator".to_string()),
                prompt: job.prompt.clone().unwrap_or_default(),
            }),
            JobType::Flow => {
                openhuman_core::cron::system_jobs::system_job_name(job).map(|name| Self::System {
                    name: name.to_string(),
                })
            }
            JobType::Shell => None,
        }
    }
}

/// A job as declared by the host. Upserted by `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSpec {
    /// The job's identity in the store.
    pub name: String,
    /// When it fires.
    pub schedule: JobSchedule,
    /// What it runs.
    pub target: JobTarget,
    /// Retries after a failed attempt; `None` keeps the runtime default.
    pub retries: Option<u32>,
    /// Skip, and record as `skipped`, a slot that comes due while the
    /// previous run is still in flight.
    pub single_flight: bool,
    /// Whether the job fires at all.
    pub enabled: bool,
}

impl JobSpec {
    /// A turn of agent `agent_id` driven by `prompt`.
    pub fn agent(
        name: impl Into<String>,
        agent_id: impl Into<String>,
        prompt: impl Into<String>,
        schedule: JobSchedule,
    ) -> Self {
        Self::new(
            name,
            schedule,
            JobTarget::Agent {
                agent_id: agent_id.into(),
                prompt: prompt.into(),
            },
        )
    }

    /// The host's handler for system job `system_name`.
    pub fn system(
        name: impl Into<String>,
        system_name: impl Into<String>,
        schedule: JobSchedule,
    ) -> Self {
        Self::new(
            name,
            schedule,
            JobTarget::System {
                name: system_name.into(),
            },
        )
    }

    fn new(name: impl Into<String>, schedule: JobSchedule, target: JobTarget) -> Self {
        Self {
            name: name.into(),
            schedule,
            target,
            retries: None,
            single_flight: false,
            enabled: true,
        }
    }

    /// Retries after a failed attempt; `0` is exactly one attempt.
    #[must_use]
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = Some(retries);
        self
    }

    /// Skip slots that come due while a run is still in flight.
    #[must_use]
    pub fn single_flight(mut self, single_flight: bool) -> Self {
        self.single_flight = single_flight;
        self
    }

    /// Whether the job fires.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(crate) fn validate(&self) -> Result<(), CronError> {
        let blank = |value: &str, what: &str| {
            if value.trim().is_empty() {
                Err(CronError::Invalid(format!("{what} is blank")))
            } else {
                Ok(())
            }
        };
        blank(&self.name, "the job name")?;
        match &self.target {
            JobTarget::Agent { agent_id, prompt } => {
                blank(agent_id, "the agent id")?;
                blank(prompt, "the prompt")
            }
            JobTarget::System { name } => {
                blank(name, "the system job name")?;
                if name.contains(':') {
                    return Err(CronError::Invalid(
                        "a system job name may not contain ':'".to_string(),
                    ));
                }
                Ok(())
            }
        }
    }

    fn policy(&self) -> JobPolicy {
        JobPolicy {
            retries: self.retries,
            single_flight: self.single_flight,
        }
    }
}

/// A stored job, as [`Cron::list`] and [`Cron::upsert`] report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledJob {
    /// The store's id for the job.
    pub id: String,
    /// The job's name.
    pub name: String,
    /// When it fires.
    pub schedule: JobSchedule,
    /// What it runs.
    pub target: JobTarget,
    /// Retries after a failed attempt; `None` is the runtime default.
    pub retries: Option<u32>,
    /// Whether overlapping slots are skipped and recorded.
    pub single_flight: bool,
    /// Whether the job fires.
    pub enabled: bool,
    /// When it next fires.
    pub next_run: SystemTime,
    /// When it last ran.
    pub last_run: Option<SystemTime>,
    /// `ok` or `error` for the last run.
    pub last_status: Option<String>,
}

/// One run's outcome from [`Cron::run_now`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRun {
    /// Whether the run (after its retries) succeeded.
    pub success: bool,
    /// The agent's reply, the handler's error, or a canned failure message.
    pub output: String,
}

/// One entry of a job's run history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRunRecord {
    /// `ok`, `error`, `skipped` or `queued`.
    pub status: String,
    /// What the run produced, truncated.
    pub output: Option<String>,
    /// When it started.
    pub started_at: SystemTime,
    /// When it finished.
    pub finished_at: SystemTime,
}

/// Error from the [`Cron`] facade.
#[derive(Debug, thiserror::Error)]
pub enum CronError {
    /// The runtime's config has `cron.enabled = false`.
    #[error("cron is disabled by config (cron.enabled=false)")]
    Disabled,
    /// No job of that name is stored.
    #[error("no scheduled job named {0:?}")]
    NotFound(String),
    /// The spec could not be honoured.
    #[error("invalid job: {0}")]
    Invalid(String),
    /// The run was refused or could not start (a single-flight job that is
    /// already running, for one).
    #[error("{0}")]
    Run(String),
    /// The job store failed.
    #[error("cron store: {0:#}")]
    Store(#[from] anyhow::Error),
}

/// Typed access to the runtime's scheduled jobs. Obtain with
/// [`Runtime::cron`](crate::Runtime::cron).
pub struct Cron<'a> {
    config: &'a Config,
    /// The runtime's own context: what a Run Now executes and delivers under.
    dispatch: openhuman_core::core::runtime::DispatchCtx,
}

impl<'a> Cron<'a> {
    pub(crate) fn new(
        config: &'a Config,
        dispatch: openhuman_core::core::runtime::DispatchCtx,
    ) -> Self {
        Self { config, dispatch }
    }

    fn enabled(&self) -> Result<(), CronError> {
        if self.config.cron.enabled {
            Ok(())
        } else {
            Err(CronError::Disabled)
        }
    }

    /// Create the job `spec.name`, or bring the stored one in line with
    /// `spec`. Unchanged fields are left alone, so upserting the same spec
    /// again does not move its next run.
    pub fn upsert(&self, spec: JobSpec) -> Result<ScheduledJob, CronError> {
        self.enabled()?;
        spec.validate()?;
        let config = self.config;
        let schedule = spec.schedule.to_core();
        let existing = self.find(&spec.name)?;
        log::debug!(
            "[embed][cron] upsert name={} exists={} target={:?}",
            spec.name,
            existing.is_some(),
            spec.target
        );
        let job = match (&spec.target, existing) {
            (JobTarget::Agent { agent_id, prompt }, Some(job))
                if matches!(job.job_type, JobType::Agent) =>
            {
                let patch = CronJobPatch {
                    schedule: (job.schedule != schedule).then(|| schedule.clone()),
                    prompt: (job.prompt.as_deref() != Some(prompt.as_str()))
                        .then(|| prompt.clone()),
                    agent_id: (job.agent_id.as_deref() != Some(agent_id.as_str()))
                        .then(|| Some(agent_id.clone())),
                    enabled: (job.enabled != spec.enabled).then_some(spec.enabled),
                    ..CronJobPatch::default()
                };
                patch_if_needed(config, job, patch)?
            }
            (JobTarget::System { name }, Some(job))
                if job.command == format!("{SYSTEM_COMMAND_PREFIX}{name}") =>
            {
                let patch = CronJobPatch {
                    schedule: (job.schedule != schedule).then(|| schedule.clone()),
                    enabled: (job.enabled != spec.enabled).then_some(spec.enabled),
                    ..CronJobPatch::default()
                };
                patch_if_needed(config, job, patch)?
            }
            (target, existing) => {
                if let Some(job) = existing {
                    log::debug!("[embed][cron] target kind changed; replacing {}", job.id);
                    openhuman_core::cron::remove_job(config, &job.id)?;
                }
                create(config, &spec.name, target, schedule, spec.enabled)?
            }
        };
        let job = openhuman_core::cron::update_job(
            config,
            &job.id,
            CronJobPatch {
                policy: Some(spec.policy()),
                ..CronJobPatch::default()
            },
        )?;
        self.describe(job)?.ok_or_else(|| {
            CronError::Invalid(format!("job {:?} is not an agent or system job", spec.name))
        })
    }

    /// Every agent and system job in the store. Shell jobs and workflow
    /// schedule triggers are not listed.
    pub fn list(&self) -> Result<Vec<ScheduledJob>, CronError> {
        self.enabled()?;
        let mut jobs = Vec::new();
        for job in openhuman_core::cron::list_jobs(self.config)? {
            if let Some(described) = self.describe(job)? {
                jobs.push(described);
            }
        }
        Ok(jobs)
    }

    /// Remove job `name`. `false` when there was none.
    pub fn remove(&self, name: &str) -> Result<bool, CronError> {
        self.enabled()?;
        match self.find(name)? {
            Some(job) => {
                log::debug!("[embed][cron] remove name={name} id={}", job.id);
                openhuman_core::cron::remove_job(self.config, &job.id)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Run job `name` now and wait for it: the same execution, retry budget,
    /// run record and delivery as a scheduled run.
    pub async fn run_now(&self, name: &str) -> Result<JobRun, CronError> {
        self.enabled()?;
        let job = self
            .find(name)?
            .ok_or_else(|| CronError::NotFound(name.to_string()))?;
        log::debug!("[embed][cron] run_now name={name} id={}", job.id);
        let (success, output) =
            openhuman_core::cron::ops::run_job_now(&self.dispatch, self.config, &job.id)
                .await
                .map_err(CronError::Run)?;
        Ok(JobRun { success, output })
    }

    /// The newest `limit` runs of job `name`, newest first.
    pub fn runs(&self, name: &str, limit: usize) -> Result<Vec<JobRunRecord>, CronError> {
        self.enabled()?;
        let job = self
            .find(name)?
            .ok_or_else(|| CronError::NotFound(name.to_string()))?;
        Ok(
            openhuman_core::cron::list_runs(self.config, &job.id, limit.max(1))?
                .into_iter()
                .map(|run| JobRunRecord {
                    status: run.status,
                    output: run.output,
                    started_at: SystemTime::from(run.started_at),
                    finished_at: SystemTime::from(run.finished_at),
                })
                .collect(),
        )
    }

    fn find(&self, name: &str) -> Result<Option<CronJob>, CronError> {
        Ok(openhuman_core::cron::list_jobs(self.config)?
            .into_iter()
            .find(|job| job.name.as_deref() == Some(name) && JobTarget::from_job(job).is_some()))
    }

    fn describe(&self, job: CronJob) -> Result<Option<ScheduledJob>, CronError> {
        let Some(target) = JobTarget::from_job(&job) else {
            return Ok(None);
        };
        let policy = job.policy;
        Ok(Some(ScheduledJob {
            name: job.name.clone().unwrap_or_default(),
            schedule: JobSchedule::from_core(&job.schedule),
            target,
            retries: policy.retries,
            single_flight: policy.single_flight,
            enabled: job.enabled,
            next_run: SystemTime::from(job.next_run),
            last_run: job.last_run.map(SystemTime::from),
            last_status: job.last_status,
            id: job.id,
        }))
    }
}

fn patch_if_needed(
    config: &Config,
    job: CronJob,
    patch: CronJobPatch,
) -> Result<CronJob, CronError> {
    let unchanged = patch.schedule.is_none()
        && patch.prompt.is_none()
        && patch.agent_id.is_none()
        && patch.enabled.is_none();
    if unchanged {
        return Ok(job);
    }
    Ok(openhuman_core::cron::update_job(config, &job.id, patch)?)
}

fn create(
    config: &Config,
    name: &str,
    target: &JobTarget,
    schedule: Schedule,
    enabled: bool,
) -> Result<CronJob, CronError> {
    match target {
        JobTarget::Agent { agent_id, prompt } => {
            Ok(openhuman_core::cron::add_agent_job_with_definition(
                config,
                Some(name.to_string()),
                schedule,
                prompt,
                openhuman_core::cron::SessionTarget::Isolated,
                None,
                None,
                false,
                Some(agent_id.clone()),
                enabled,
            )?)
        }
        JobTarget::System { name: system } => {
            // One row per system job: the store keys it by its command.
            let command = format!("{SYSTEM_COMMAND_PREFIX}{system}");
            let job =
                openhuman_core::cron::add_flow_schedule_job(config, &command, schedule.clone())?;
            Ok(openhuman_core::cron::update_job(
                config,
                &job.id,
                CronJobPatch {
                    name: Some(name.to_string()),
                    enabled: Some(enabled),
                    schedule: (job.schedule != schedule).then_some(schedule),
                    ..CronJobPatch::default()
                },
            )?)
        }
    }
}

/// Box a host's system-job handler into the core's handler type.
pub(crate) fn boxed_handler<F, Fut>(
    handler: F,
) -> openhuman_core::cron::system_job_handlers::SystemJobHandler
where
    F: Fn(SystemJobContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    std::sync::Arc::new(move |ctx| Box::pin(handler(ctx)))
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
