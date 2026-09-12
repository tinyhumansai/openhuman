use super::*;
use crate::openhuman::agent::error::AgentError;
use crate::openhuman::config::Config;
use crate::openhuman::cron::{self, ActiveHours, DeliveryConfig};
use crate::openhuman::security::SecurityPolicy;
use chrono::{Duration as ChronoDuration, Timelike, Utc};
#[cfg(not(windows))]
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use tempfile::TempDir;

async fn test_config(tmp: &TempDir) -> Config {
    let ws = tmp.path().join("workspace");
    let config = Config {
        workspace_dir: ws.clone(),
        action_dir: ws.clone(),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    tokio::fs::create_dir_all(&config.workspace_dir)
        .await
        .unwrap();
    config
}

fn test_job(command: &str) -> CronJob {
    CronJob {
        id: "test-job".into(),
        expression: "* * * * *".into(),
        schedule: crate::openhuman::cron::Schedule::Cron {
            expr: "* * * * *".into(),
            tz: None,
            active_hours: None,
        },
        command: command.into(),
        prompt: None,
        name: None,
        job_type: JobType::Shell,
        session_target: SessionTarget::Isolated,
        model: None,
        agent_id: None,
        profile_id: None,
        enabled: true,
        delivery: DeliveryConfig::default(),
        delete_after_run: false,
        created_at: Utc::now(),
        next_run: Utc::now(),
        last_run: None,
        last_status: None,
        last_output: None,
    }
}

fn proactive_job() -> CronJob {
    let mut job = test_job("");
    job.delivery = DeliveryConfig {
        mode: "proactive".into(),
        channel: None,
        to: None,
        best_effort: true,
    };
    job
}

async fn cron_alerts(config: &Config) -> usize {
    crate::openhuman::desktop::notifications::store::list(config, 10, 0, Some("cron"), None)
        .unwrap()
        .len()
}

/// Receive the next `user_error` broadcast on `rx` carrying `kind`, skipping any
/// unrelated events. The web-channel bus is a process-global broadcast, so a
/// sibling test running concurrently may interleave its own `user_error` (a
/// different kind) onto the same channel — filtering on `kind` keeps each test
/// deterministic regardless of ordering.
///
/// A concurrent flood can also push our event past the channel capacity before
/// we read it, surfacing as `Lagged` (the receiver fell behind, not a real
/// absence). We treat `Lagged` as recoverable and keep scanning (CodeRabbit
/// #4169); only a terminal `Empty`/`Closed` — the matching event genuinely was
/// not published — panics.
fn next_user_error(
    rx: &mut tokio::sync::broadcast::Receiver<crate::core::socketio::WebChannelEvent>,
    kind: &str,
) -> crate::core::socketio::WebChannelEvent {
    use tokio::sync::broadcast::error::TryRecvError;
    loop {
        match rx.try_recv() {
            Ok(ev) if ev.event == "user_error" && ev.error_type.as_deref() == Some(kind) => {
                break ev
            }
            Ok(_) => continue,
            // Receiver fell behind a concurrent flood — the dropped slots can't
            // have held *our* just-published event before this point, so skip
            // ahead and keep scanning rather than failing spuriously.
            Err(TryRecvError::Lagged(_)) => continue,
            Err(e) => panic!("expected a user_error broadcast for kind={kind}, bus said: {e:?}"),
        }
    }
}

#[path = "scheduler_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "scheduler_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "scheduler_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "scheduler_tests_part_04_tests.rs"]
mod part_04_tests;
