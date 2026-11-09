//! Bounded async execution, filtering, and report assembly.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::contract::{
    DoctorCheck, DoctorCtx, DoctorOptions, DoctorReport, DoctorSummary, Finding,
};
use super::registry;

const MAX_CONCURRENT_CHECKS: usize = 8;
static CHECK_SEMAPHORE: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_CHECKS)));

/// Runs registered checks using exact-id filters and stable report ordering.
pub(super) async fn run(ctx: &DoctorCtx, options: DoctorOptions) -> Result<DoctorReport, String> {
    let checks = registry::registered_checks_for(ctx)?;
    let check_ids = checks
        .iter()
        .map(|check| check.id().to_owned())
        .collect::<HashSet<_>>();
    let only = options.only.into_iter().collect::<HashSet<_>>();
    let skip = options.skip.into_iter().collect::<HashSet<_>>();
    let has_unknown_selector = only
        .iter()
        .chain(skip.iter())
        .any(|id| !check_ids.contains(id));
    let _reserved_profile_input = options.profile;

    let mut evaluated = Vec::new();
    let mut runnable = Vec::new();

    for (index, check) in checks.into_iter().enumerate() {
        let id = check.id().to_owned();
        if (!only.is_empty() && !only.contains(&id)) || skip.contains(&id) {
            continue;
        }

        if !options.tier.includes(check.tier()) {
            evaluated.push((
                index,
                id.clone(),
                vec![Finding::skipped(
                    &id,
                    format!(
                        "The {} check is above the requested tier.",
                        check.category()
                    ),
                    "Request a higher doctor tier and rerun the check.",
                )],
                true,
            ));
            continue;
        }

        if let Some(component) = check.component_gate() {
            if !component.enabled(&ctx.components()) {
                let name = component.as_str();
                evaluated.push((
                    index,
                    id.clone(),
                    vec![Finding::skipped(
                        &id,
                        format!("The required {name} component is disabled."),
                        format!("Enable the {name} component and rerun doctor."),
                    )],
                    true,
                ));
                continue;
            }
        }
        runnable.push((index, check));
    }

    let mut tasks = FuturesUnordered::new();
    for (index, check) in runnable {
        tasks.push(execute_one(index, check, ctx.clone()));
    }
    let mut outcomes = Vec::new();
    while let Some(outcome) = tasks.next().await {
        outcomes.push(outcome);
    }
    evaluated.append(&mut outcomes);
    evaluated.sort_by_key(|(index, _, _, _)| *index);

    let mut checks_run = Vec::new();
    let mut checks_skipped = Vec::new();
    let mut findings = Vec::new();
    for (_, id, mut check_findings, was_skipped) in evaluated {
        if was_skipped {
            checks_skipped.push(id);
        } else {
            checks_run.push(id);
        }
        findings.append(&mut check_findings);
    }

    if has_unknown_selector {
        findings.push(Finding::error(
            "doctor/selection",
            "One or more selected check ids are not registered.",
            "Use a registered exact check id from the doctor report.",
        ));
    }

    let summary = DoctorSummary::from_findings(&findings);
    let ok = summary.error == 0;
    Ok(DoctorReport {
        schema_version: 2,
        profile: ctx.profile().map(str::to_owned),
        config_source: ctx.config_source(),
        profile_layout_version: ctx.profile_layout_version().map(|version| match version {
            crate::config::schema::profile_layout::LayoutVersion::V1 => 1,
            crate::config::schema::profile_layout::LayoutVersion::V2 => 2,
        }),
        config_resolution_source: ctx.profile_resolution_source().map(str::to_owned),
        tier: options.tier,
        checks_run,
        checks_skipped,
        findings,
        summary,
        ok,
    })
}

async fn execute_one(
    index: usize,
    check: Arc<dyn DoctorCheck>,
    ctx: DoctorCtx,
) -> (usize, String, Vec<Finding>, bool) {
    let id = check.id().to_owned();
    let timeout = check.timeout();
    let started = Instant::now();
    let detection = detect(check, ctx);
    let result = tokio::time::timeout(timeout, detection).await;
    let duration_ms = elapsed_ms(started.elapsed());
    let mut findings = match result {
        Ok(Ok(mut findings)) => {
            if findings.is_empty() {
                findings.push(Finding::pass(&id, "The check completed without findings."));
            }
            findings
        }
        Ok(Err(_)) => vec![Finding::error(
            &id,
            "The check worker failed.",
            "Review the check's local dependencies and rerun doctor.",
        )],
        Err(_) => vec![Finding::error(
            &id,
            format!("The check timed out after {} ms.", timeout.as_millis()),
            "Review the check's dependencies and rerun doctor.",
        )],
    };
    for finding in &mut findings {
        finding.check_id.clone_from(&id);
        finding.duration_ms = duration_ms;
        if finding.fix_hint.trim().is_empty() {
            finding.fix_hint = "Review the check result and rerun doctor.".into();
        }
    }
    (index, id, findings, false)
}

async fn detect(check: Arc<dyn DoctorCheck>, ctx: DoctorCtx) -> Result<Vec<Finding>, ()> {
    // Include permit acquisition in the per-check budget, so queued work
    // cannot wait forever behind blocking probes that outlive a timeout.
    let permit: OwnedSemaphorePermit = Arc::clone(&CHECK_SEMAPHORE)
        .acquire_owned()
        .await
        .map_err(|_| ())?;
    if check.is_blocking() {
        tokio::task::spawn_blocking(move || {
            let result = check.detect_blocking(&ctx).ok_or(());
            // A blocking worker cannot be cancelled after it has started. It
            // retains the process-wide slot after its caller times out.
            drop(permit);
            result
        })
        .await
        .map_err(|_| ())?
    } else {
        let findings = check.detect(&ctx).await;
        drop(permit);
        Ok(findings)
    }
}

fn elapsed_ms(duration: std::time::Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}
