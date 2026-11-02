//! The store itself is tested upstream (`tinyflows-drivers`); these pin the
//! host mapping from `Config` to its options and the import of the `jobs.db`
//! tinyflows used to write.

use super::*;
use chrono::Duration as ChronoDuration;
use tempfile::TempDir;

/// Move one scheduled row into the past for scheduler tests. The document
/// representation belongs to the module store; this helper is test-only.
pub(crate) fn force_next_run_for_test(
    config: &Config,
    job_id: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    super::force_next_run_for_test(config, job_id, at)
}

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    crate::storage::local::use_persistent_default_for_test(&config.workspace_dir);
    config
}

#[test]
fn remove_job_does_not_write_to_stdout() {
    use std::process::Command;

    const CHILD: &str = "OPENHUMAN_CRON_REMOVE_STDOUT_CHILD";
    if let Ok(workspace) = std::env::var(CHILD) {
        let config = Config {
            workspace_dir: workspace.into(),
            action_dir: std::env::temp_dir(),
            ..Config::default()
        };
        crate::storage::local::use_persistent_default_for_test(&config.workspace_dir);
        println!("BEGIN_CRON_REMOVE_CAPTURE");
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        remove_job(
            &config,
            &std::env::var("OPENHUMAN_CRON_REMOVE_JOB").unwrap(),
        )
        .unwrap();
        println!("END_CRON_REMOVE_CAPTURE");
        std::io::stdout().flush().unwrap();
        return;
    }

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = add_job(&config, "*/5 * * * *", "echo remove").unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cron::store::tests::remove_job_does_not_write_to_stdout",
            "--nocapture",
        ])
        .env(CHILD, &config.workspace_dir)
        .env("OPENHUMAN_CRON_REMOVE_JOB", &job.id)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let captured = stdout.split("BEGIN_CRON_REMOVE_CAPTURE\n").nth(1).unwrap();
    let captured = captured.split("END_CRON_REMOVE_CAPTURE\n").next().unwrap();
    assert!(captured.is_empty(), "unexpected stdout: {captured}");
}

#[cfg(feature = "storage-sqlite")]
#[test]
fn jobs_live_in_the_workspace_default_database() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    add_job(&config, "*/5 * * * *", "echo ok").unwrap();
    assert!(tmp.path().join("workspace/storage/storage.db").is_file());
}

#[test]
fn run_history_cap_follows_cron_max_run_history() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.cron.max_run_history = 2;
    let job = add_job(&config, "*/15 * * * *", "echo run").unwrap();
    for i in 0..5 {
        let t = Utc::now() + ChronoDuration::seconds(i);
        record_run(&config, &job.id, t, t, "ok", Some("x"), 1).unwrap();
    }
    assert_eq!(list_runs(&config, &job.id, 10).unwrap().len(), 2);
}

#[test]
fn a_stale_routing_patch_cannot_combine_with_a_newer_unvalidated_route() {
    use tinyflows_bus::schedule::AgentJobSpec;

    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.channels_config.telegram = Some(crate::config::TelegramConfig {
        bot_token: "test-token".into(),
        chat_id: None,
        allowed_users: vec!["42".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    let channel_origin = |target: &str| tinyflows_bus::schedule::types::JobOrigin::Channel {
        channel: "telegram".into(),
        reply_target: target.into(),
        history_key: format!("telegram-target-{target}"),
        sender: None,
        thread_id: None,
        channel_session: None,
    };
    let origin_42 = channel_origin("42");
    let origin_999 = channel_origin("999");
    let mut spec = AgentJobSpec::new(Schedule::Every { every_ms: 300_000 }, "remind me");
    spec.delivery = Some(DeliveryConfig::default());
    spec.origin = Some(origin_42.clone());
    let job = add_agent_job_from_spec(&config, spec).unwrap();
    let initial = get_job(&config, &job.id).unwrap();

    // Both callers validate against the same initial row. A routes through
    // the authorized chat 42; B changes only the origin while delivery is
    // `none`. Pause before either request reaches the module store.
    let stale_a = validate_and_complete_route_patch(
        &config,
        &initial,
        CronJobPatch {
            delivery: Some(DeliveryConfig {
                mode: "origin".into(),
                ..DeliveryConfig::default()
            }),
            ..CronJobPatch::default()
        },
        None,
    )
    .unwrap();
    let stale_b = validate_and_complete_route_patch(
        &config,
        &initial,
        CronJobPatch {
            origin: Some(Some(origin_999.clone())),
            ..CronJobPatch::default()
        },
        None,
    )
    .unwrap();

    store_blocking::<CronJob>(
        &config,
        StoreOperation::Update {
            job_id: job.id.clone(),
            patch: stale_a,
        },
    )
    .unwrap();
    store_blocking::<CronJob>(
        &config,
        StoreOperation::Update {
            job_id: job.id.clone(),
            patch: stale_b,
        },
    )
    .unwrap();

    let saved = get_job(&config, &job.id).unwrap();
    assert_eq!(saved.delivery.mode, "none");
    assert_eq!(saved.origin.as_ref(), Some(&origin_999));
}

fn agent_context(
    config: &Config,
    agent: &str,
) -> std::sync::Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .session_agent(agent),
    )
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn an_agents_jobs_live_in_its_own_scope() {
    use crate::core::runtime::CoreContext;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = CoreContext::scope(agent_context(&config, "alpha"), async {
        add_job(&config, "*/5 * * * *", "echo alpha").unwrap()
    })
    .await;

    let alpha_ids = CoreContext::scope(agent_context(&config, "alpha"), async {
        list_jobs(&config)
            .unwrap()
            .into_iter()
            .map(|job| job.id)
            .collect::<Vec<_>>()
    })
    .await;
    assert_eq!(alpha_ids, std::slice::from_ref(&job.id));
    let beta_jobs = CoreContext::scope(agent_context(&config, "beta"), async {
        list_jobs(&config).unwrap()
    })
    .await;
    assert!(beta_jobs.is_empty());
    assert!(
        list_jobs(&config).unwrap().is_empty(),
        "the workspace store stays empty"
    );
}

#[tokio::test]
async fn a_job_whose_agent_is_not_live_stays_dormant() {
    use crate::core::runtime::CoreContext;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ctx = agent_context(&config, "dormant-agent");
    let job = CoreContext::scope(std::sync::Arc::clone(&ctx), async {
        add_job(&config, "* * * * *", "echo dormant").unwrap()
    })
    .await;
    drop(ctx);

    let mut agents = crate::cron::scheduler::LiveAgentSchedulers::default();
    agents.tick().await;
    agents.drain().await;

    let after = CoreContext::scope(agent_context(&config, "dormant-agent"), async {
        get_job(&config, &job.id).unwrap()
    })
    .await;
    assert!(
        after.last_run.is_none(),
        "nothing ran the dormant agent's job"
    );
}

#[cfg(feature = "legacy-import")]
fn legacy_fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/storage/import/fixtures/tinyflows/workspace/cron/jobs.db")
}

#[cfg(all(feature = "storage-sqlite", feature = "legacy-import"))]
#[test]
fn a_legacy_jobs_db_is_imported_by_the_storage_importer_and_left_untouched() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // The importer binds its claim to atomic conditional writes, which the
    // in-memory test default does not support: use the workspace's sqlite.
    crate::storage::local::use_persistent_default_for_test(&config.workspace_dir);
    let legacy = crate::storage::import::tinyflows::legacy_jobs_db(
        &crate::core::runtime::agent_scope_dir(&config),
    );
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::copy(legacy_fixture(), &legacy).unwrap();
    let before = std::fs::read(&legacy).unwrap();

    let backend = crate::storage::local::backend_for(&config.workspace_dir).unwrap();
    for report in crate::storage::import::tinyflows::run_with(&backend, &config) {
        report.outcome.unwrap();
    }

    let jobs = list_jobs(&config).unwrap();
    assert_eq!(jobs.len(), 3);
    assert!(jobs.iter().any(|job| job.name.as_deref() == Some("inbox")));
    assert!(find_flow_schedule_job(&config, "flow-beta")
        .unwrap()
        .is_some());
    assert_eq!(
        std::fs::read(&legacy).unwrap(),
        before,
        "jobs.db was modified"
    );

    // Imported once: a job removed afterwards stays removed.
    let inbox = jobs
        .iter()
        .find(|job| job.name.as_deref() == Some("inbox"))
        .unwrap();
    remove_job(&config, &inbox.id).unwrap();
    for report in crate::storage::import::tinyflows::run_with(&backend, &config) {
        report.outcome.unwrap();
    }
    assert_eq!(list_jobs(&config).unwrap().len(), 2);
}

#[test]
fn a_patch_over_the_bus_keeps_the_agent_definition_it_does_not_mention() {
    use tinyflows_bus::schedule::types::{CronJobPatch, SessionTarget};
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = add_agent_job_with_definition(
        &config,
        Some("patched".into()),
        Schedule::Every {
            every_ms: 3_600_000,
        },
        "say hi",
        SessionTarget::Isolated,
        None,
        None,
        false,
        Some("teeny".into()),
        true,
    )
    .unwrap();

    assert_eq!(
        job.agent_id.as_deref(),
        Some("teeny"),
        "the definition id is stored"
    );
    let renamed = update_job(
        &config,
        &job.id,
        CronJobPatch {
            prompt: Some("check in".into()),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    assert_eq!(renamed.prompt.as_deref(), Some("check in"));

    let policy = update_job(
        &config,
        &job.id,
        CronJobPatch {
            policy: Some(Default::default()),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    assert_eq!(
        policy.prompt.as_deref(),
        Some("check in"),
        "a policy-only patch must not undo the earlier prompt patch"
    );
    assert_eq!(
        get_job(&config, &job.id).unwrap().prompt.as_deref(),
        Some("check in")
    );
    // Regression: `CronJobPatch.agent_id` is `Option<Option<_>>` and a present
    // `null` means "clear it". A patch that never mentioned the agent used to
    // serialize `agent_id: null` across the bus, so every update (pausing a
    // job, a policy change) silently dropped the job's agent definition.
    assert_eq!(
        policy.agent_id.as_deref(),
        Some("teeny"),
        "an unrelated patch cleared the agent definition"
    );
    assert_eq!(
        get_job(&config, &job.id).unwrap().agent_id.as_deref(),
        Some("teeny")
    );
}

fn assert_force_next_run_updates_collection(
    collection: &'static str,
    other_collection: &'static str,
) {
    use serde_json::json;
    use tinystoragedrivers::{CollectionSpec, Precondition};

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let opened = crate::storage::local::open("cron::store-test", &config.workspace_dir).unwrap();
    let docs = std::sync::Arc::clone(opened.scoped().unwrap().documents());
    let seed_docs = std::sync::Arc::clone(&docs);
    crate::storage::block_on_anyhow(async move {
        seed_docs
            .ensure_collection(&CollectionSpec::new(collection))
            .await?;
        let seeded_job = if collection == "cron_jobs" {
            json!({
                "next_run": "2030-01-01T00:00:00Z",
                "next_run_ms": 1_893_456_000_000_i64,
                "next_run_ns": 1_893_456_000_000_000_000_i64,
                "marker": "keep-me",
            })
        } else {
            json!({ "next_run_ms": 123, "marker": "keep-me" })
        };
        seed_docs
            .put(collection, "seeded-job", seeded_job, Precondition::Absent)
            .await?;
        anyhow::Ok(())
    })
    .unwrap();

    let before_docs = std::sync::Arc::clone(&docs);
    let collections_before =
        crate::storage::block_on_anyhow(async move { Ok(before_docs.collections("").await?) })
            .unwrap();
    assert!(collections_before.contains(&collection.to_string()));

    let next = chrono::DateTime::<Utc>::from_timestamp_millis(456).unwrap();
    force_next_run_for_test(&config, "seeded-job", next).unwrap();

    let read_docs = std::sync::Arc::clone(&docs);
    let (updated, other, collections) = crate::storage::block_on_anyhow(async move {
        Ok((
            read_docs.get(collection, "seeded-job").await?,
            read_docs.get(other_collection, "seeded-job").await?,
            read_docs.collections("").await?,
        ))
    })
    .unwrap();
    let updated = updated.expect("seeded document remains present").doc;
    assert_eq!(updated["next_run_ms"], json!(456));
    assert_eq!(updated["marker"], json!("keep-me"));
    if collection == "cron_jobs" {
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(updated["next_run"].as_str().unwrap())
                .unwrap()
                .timestamp_millis(),
            456
        );
        assert_eq!(updated["next_run_ns"], json!(456_000_000));
    } else {
        assert!(updated.get("next_run").is_none());
        assert!(updated.get("next_run_ns").is_none());
    }
    assert!(
        other.is_none(),
        "helper must not create or write {other_collection}"
    );
    assert_eq!(collections, collections_before);
}

#[test]
fn force_next_run_updates_current_cron_jobs_collection() {
    assert_force_next_run_updates_collection("cron.jobs", "cron_jobs");
}

#[test]
fn force_next_run_updates_legacy_cron_jobs_collection() {
    assert_force_next_run_updates_collection("cron_jobs", "cron.jobs");
}
