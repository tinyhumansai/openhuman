//! Reads the legacy `jobs.db` (the cron store before it moved to documents) into
//! plain bus records. It names only the transport vocabulary
//! (`tinyflows_bus::schedule::types`), so a host that reaches the scheduler
//! through the bus can replay the file through the module's
//! `import_job` / `import_run` store operations without linking the scheduler
//! implementation.
//!
//! Two tables, `cron_jobs` and `cron_runs`. `cron_jobs` gained most of its
//! columns over time, so a database from before a column existed reads as if
//! the column held its default. An older `cron_job_policies` side table, which
//! the old store folded into `cron_jobs.retries` / `single_flight` on its next
//! open, is read the same way here.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tinyflows_bus::schedule::types::{
    CronJob, CronRun, DeliveryConfig, DeliveryStatus, JobOrigin, JobPolicy, JobType, Schedule,
    SessionTarget,
};

use super::legacy_db::{column_or, LegacyDb};
use super::report::ImportReport;

/// What `jobs.db` held, decoded.
#[derive(Debug, Default)]
pub struct LegacyCron {
    /// Every decodable job, oldest first.
    pub jobs: Vec<CronJob>,
    /// Every decodable run, in row order. Its `job_id` is the job's id in the
    /// legacy file; a flow-schedule job is filed under another id by the
    /// target store, so a caller remaps it from the id the import returns.
    pub runs: Vec<CronRun>,
    /// Rows that could not be decoded; also counted in the report as invalid.
    pub report: ImportReport,
}

/// Reads `jobs_db` (the file the old store was opened with). A missing file or
/// one without a `cron_jobs` table reads as empty.
///
/// # Errors
///
/// When the file exists but cannot be read. A row that cannot be decoded is
/// counted in the report as invalid.
pub async fn read_legacy_cron(jobs_db: &Path) -> Result<LegacyCron> {
    let mut legacy = LegacyCron::default();
    let Some(db) = LegacyDb::open(jobs_db)? else {
        return Ok(legacy);
    };
    let tables = db.tables().await?;
    if !tables.contains("cron_jobs") {
        return Ok(legacy);
    }
    legacy.jobs = read_jobs(&db, &tables, &mut legacy.report).await?;
    if tables.contains("cron_runs") {
        legacy.runs = read_runs(&db, &mut legacy.report).await?;
    }
    Ok(legacy)
}

/// One `cron_jobs` row with the raw text columns still raw.
struct JobRow {
    id: String,
    expression: String,
    command: String,
    schedule: Option<String>,
    job_type: String,
    prompt: Option<String>,
    name: Option<String>,
    session_target: String,
    model: Option<String>,
    enabled: i64,
    delivery: Option<String>,
    delete_after_run: i64,
    created_at: String,
    next_run: String,
    last_run: Option<String>,
    last_status: Option<String>,
    last_output: Option<String>,
    agent_id: Option<String>,
    origin: Option<String>,
    retries: Option<i64>,
    single_flight: i64,
    claim_generation: i64,
}

async fn read_jobs(
    db: &LegacyDb,
    tables: &HashSet<String>,
    report: &mut ImportReport,
) -> Result<Vec<CronJob>> {
    let have = db.columns("cron_jobs").await?;
    let select = format!(
        "SELECT id, expression, command, {}, {}, {}, {}, {}, {}, {}, {}, {}, created_at, next_run, \
         last_run, last_status, last_output, {}, {}, {}, {}, {} FROM cron_jobs ORDER BY created_at, id",
        column_or(&have, "schedule", "NULL"),
        column_or(&have, "job_type", "'shell'"),
        column_or(&have, "prompt", "NULL"),
        column_or(&have, "name", "NULL"),
        column_or(&have, "session_target", "'isolated'"),
        column_or(&have, "model", "NULL"),
        column_or(&have, "enabled", "1"),
        column_or(&have, "delivery", "NULL"),
        column_or(&have, "delete_after_run", "0"),
        column_or(&have, "agent_id", "NULL"),
        column_or(&have, "origin", "NULL"),
        column_or(&have, "retries", "NULL"),
        column_or(&have, "single_flight", "0"),
        column_or(&have, "claim_generation", "0"),
    );
    let mut rows = db
        .read("cron jobs", move |conn| {
            let mut statement = conn.prepare(&select)?;
            let rows = statement
                .query_map([], |row| {
                    Ok(JobRow {
                        id: row.get(0)?,
                        expression: row.get(1)?,
                        command: row.get(2)?,
                        schedule: row.get(3)?,
                        job_type: row.get(4)?,
                        prompt: row.get(5)?,
                        name: row.get(6)?,
                        session_target: row.get(7)?,
                        model: row.get(8)?,
                        enabled: row.get(9)?,
                        delivery: row.get(10)?,
                        delete_after_run: row.get(11)?,
                        created_at: row.get(12)?,
                        next_run: row.get(13)?,
                        last_run: row.get(14)?,
                        last_status: row.get(15)?,
                        last_output: row.get(16)?,
                        agent_id: row.get(17)?,
                        origin: row.get(18)?,
                        retries: row.get(19)?,
                        single_flight: row.get(20)?,
                        claim_generation: row.get(21)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>();
            rows
        })
        .await?;
    if tables.contains("cron_job_policies") {
        let policies = read_policies(db).await?;
        for row in &mut rows {
            if let Some((retries, single_flight)) = policies.get(&row.id) {
                row.retries = *retries;
                row.single_flight = *single_flight;
            }
        }
    }
    let mut jobs = Vec::new();
    for row in rows {
        let id = row.id.clone();
        match decode_job(row) {
            Ok(job) => jobs.push(job),
            Err(error) => report.invalid("jobs", format!("job '{id}': {error:#}")),
        }
    }
    Ok(jobs)
}

/// `cron_job_policies`: `job_id -> (retries, single_flight)`.
async fn read_policies(db: &LegacyDb) -> Result<HashMap<String, (Option<i64>, i64)>> {
    db.read("cron job policies", |conn| {
        let mut statement =
            conn.prepare("SELECT job_id, retries, single_flight FROM cron_job_policies")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, (row.get(1)?, row.get(2)?)))
            })?
            .collect::<Result<HashMap<_, _>, _>>();
        rows
    })
    .await
}

fn decode_job(row: JobRow) -> Result<CronJob> {
    let schedule = match row.schedule.as_deref().map(str::trim) {
        Some(raw) if !raw.is_empty() => serde_json::from_str(raw)
            .with_context(|| format!("cron schedule JSON does not parse: {raw}"))?,
        _ if row.expression.trim().is_empty() => {
            return Err(anyhow!("missing schedule and legacy expression"));
        }
        // A job written before `schedule` existed carried only its expression.
        _ => Schedule::Cron {
            expr: row.expression.clone(),
            tz: None,
            active_hours: None,
        },
    };
    let delivery = match row.delivery.as_deref().map(str::trim) {
        Some(raw) if !raw.is_empty() => serde_json::from_str(raw)
            .with_context(|| format!("cron delivery JSON does not parse: {raw}"))?,
        _ => DeliveryConfig::default(),
    };
    let origin = match row.origin.as_deref().map(str::trim) {
        Some(raw) if !raw.is_empty() => Some(
            serde_json::from_str::<JobOrigin>(raw)
                .with_context(|| format!("cron origin JSON does not parse: {raw}"))?,
        ),
        _ => None,
    };
    Ok(CronJob {
        claim_generation: u64::try_from(row.claim_generation)
            .context("claim generation is negative")?,
        policy: JobPolicy {
            retries: row
                .retries
                .map(|n| u32::try_from(n.max(0)).unwrap_or(u32::MAX)),
            single_flight: row.single_flight != 0,
        },
        id: row.id,
        expression: row.expression,
        schedule,
        command: row.command,
        job_type: JobType::parse(&row.job_type),
        prompt: row.prompt,
        name: row.name,
        session_target: SessionTarget::parse(&row.session_target),
        model: row.model,
        agent_id: row.agent_id,
        enabled: row.enabled != 0,
        delivery,
        delete_after_run: row.delete_after_run != 0,
        created_at: instant(&row.created_at)?,
        next_run: instant(&row.next_run)?,
        last_run: row.last_run.as_deref().map(instant).transpose()?,
        last_status: row.last_status,
        last_output: row.last_output,
        origin,
    })
}

fn instant(raw: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("invalid RFC3339 timestamp: {raw}"))?
        .with_timezone(&Utc))
}

async fn read_runs(db: &LegacyDb, report: &mut ImportReport) -> Result<Vec<CronRun>> {
    let have = db.columns("cron_runs").await?;
    let select = format!(
        "SELECT id, job_id, started_at, finished_at, status, output, duration_ms, {} \
         FROM cron_runs ORDER BY id",
        column_or(&have, "delivery_status", "NULL"),
    );
    let rows = db
        .read("cron runs", move |conn| {
            let mut statement = conn.prepare(&select)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>();
            rows
        })
        .await?;
    let mut runs = Vec::new();
    for (id, job_id, started_at, finished_at, status, output, duration_ms, delivery_status) in rows
    {
        match (instant(&started_at), instant(&finished_at)) {
            (Ok(started_at), Ok(finished_at)) => runs.push(CronRun {
                id,
                job_id,
                run_id: None,
                started_at,
                finished_at,
                status,
                output,
                duration_ms,
                delivery_status: delivery_status.as_deref().and_then(DeliveryStatus::parse),
            }),
            (Err(error), _) | (_, Err(error)) => {
                report.invalid("runs", format!("run {id}: {error:#}"));
            }
        }
    }
    Ok(runs)
}
