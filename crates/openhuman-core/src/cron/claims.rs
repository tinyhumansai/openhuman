//! Occurrence claims: at most one process runs a job's scheduled occurrence.
//!
//! Several nodes can sweep the same shared store. Each (job, `next_run`) pair
//! is claimed by creating one document with an absent-only write, so exactly
//! one sweeper wins it. A claim carries a lease: if its holder dies, another
//! sweeper may take the occurrence over once the lease has lapsed, by a
//! compare-and-swap on the claim's version.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tinystoragedrivers::{CollectionSpec, DocumentStore, ErrorKind, Filter, Precondition};

use crate::cron::CronJob;

/// Collection holding one claim per claimed occurrence.
pub const CLAIMS: &str = "cron_occurrence_claims";

/// How long a claim holds an occurrence before another sweeper may retake it.
pub const CLAIM_LEASE_MS: i64 = 10 * 60 * 1000;

/// Claims older than this are pruned.
const KEEP_MS: i64 = 24 * 60 * 60 * 1000;

/// The claim id of `job`'s current occurrence.
fn claim_id(job: &CronJob) -> String {
    format!("{}:{}", job.id, job.next_run.timestamp_millis())
}

/// Claim `job`'s occurrence for `holder`. `Ok(true)` when this caller now owns
/// it; `Ok(false)` when another holder has it.
///
/// # Errors
///
/// When the store fails.
pub async fn claim_occurrence(
    docs: &dyn DocumentStore,
    job: &CronJob,
    holder: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    docs.ensure_collection(&CollectionSpec::new(CLAIMS))
        .await
        .context("declaring the occurrence claims")?;
    let now_ms = now.timestamp_millis();
    let id = claim_id(job);
    let doc = json!({
        "job_id": job.id,
        "holder": holder,
        "claimed_at_ms": now_ms,
        "lease_until_ms": now_ms + CLAIM_LEASE_MS,
    });
    match docs
        .put(CLAIMS, &id, doc.clone(), Precondition::Absent)
        .await
    {
        Ok(_) => return Ok(true),
        Err(error) if error.kind() == ErrorKind::Conflict => {}
        Err(error) => return Err(error).context("claiming the occurrence"),
    }
    let Some(held) = docs.get(CLAIMS, &id).await.context("reading the claim")? else {
        return Ok(false);
    };
    let lapsed = held.doc.get("lease_until_ms").and_then(Value::as_i64) < Some(now_ms);
    if !lapsed {
        return Ok(false);
    }
    match docs.put(CLAIMS, &id, doc, held.unchanged()).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::Conflict => Ok(false),
        Err(error) => Err(error).context("retaking the lapsed claim"),
    }
}

/// Drop claims whose lease ended more than a day ago.
///
/// # Errors
///
/// When the store fails.
pub async fn prune(docs: &dyn DocumentStore, now: DateTime<Utc>) -> Result<u64> {
    let cutoff = now.timestamp_millis() - KEEP_MS;
    docs.delete_where(CLAIMS, &Filter::lt("lease_until_ms", cutoff))
        .await
        .context("pruning old claims")
}

#[cfg(test)]
#[path = "claims_tests.rs"]
mod tests;
