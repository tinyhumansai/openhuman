use super::*;
use crate::agent::turn_origin::{with_origin, TrustedAutomationSource};
use serde_json::json;
use tinyflows_schedule::types::Schedule;

fn channel_turn(history_key: Option<&str>) -> AgentTurnOrigin {
    AgentTurnOrigin::ExternalChannel {
        sender_name: None,
        channel: "telegram".into(),
        sender: Some("alice".into()),
        reply_target: "42".into(),
        message_id: "m1".into(),
        history_key: history_key.map(str::to_string),
    }
}

fn web_turn(thread_id: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: thread_id.into(),
        client_id: "c1".into(),
        request_id: None,
    }
}

#[test]
fn web_turn_maps_to_web_origin() {
    assert_eq!(
        job_origin_from_turn(&web_turn("t-1")),
        Some(JobOrigin::Web {
            thread_id: "t-1".into(),
            agent_id: None
        })
    );
    assert_eq!(job_origin_from_turn(&web_turn("  ")), None);
}

#[test]
fn channel_turn_maps_to_channel_origin_with_history_key() {
    assert_eq!(
        job_origin_from_turn(&channel_turn(Some("telegram_alice_42"))),
        Some(JobOrigin::Channel {
            channel: "telegram".into(),
            reply_target: "42".into(),
            history_key: "telegram_alice_42".into(),
            sender: Some("alice".into()),
            thread_id: None,
        })
    );
}

#[test]
fn channel_labelled_turn_without_history_key_has_no_origin() {
    // MCP server callers, triage and voice are ExternalChannel but not a chat.
    assert_eq!(job_origin_from_turn(&channel_turn(None)), None);
}

#[test]
fn other_origins_have_no_job_origin() {
    for origin in [
        AgentTurnOrigin::Cli,
        AgentTurnOrigin::DirectChat,
        AgentTurnOrigin::Unknown,
        AgentTurnOrigin::TrustedAutomation {
            job_id: "j".into(),
            source: TrustedAutomationSource::Cron,
        },
    ] {
        assert_eq!(job_origin_from_turn(&origin), None);
    }
}

#[tokio::test]
async fn current_job_origin_reads_the_scoped_turn() {
    assert_eq!(current_job_origin(), None);
    let got = with_origin(web_turn("t-9"), async { current_job_origin() }).await;
    assert!(matches!(got, Some(JobOrigin::Web { thread_id, .. }) if thread_id == "t-9"));
}

fn job_with_origin(origin: Option<JobOrigin>) -> CronJob {
    CronJob {
        id: "job-1".into(),
        expression: "* * * * *".into(),
        schedule: Schedule::Every { every_ms: 300_000 },
        command: String::new(),
        prompt: Some("p".into()),
        name: None,
        job_type: tinyflows_schedule::types::JobType::Agent,
        session_target: tinyflows_schedule::types::SessionTarget::Current,
        model: None,
        agent_id: None,
        enabled: true,
        delivery: tinyflows_schedule::types::DeliveryConfig::default(),
        delete_after_run: false,
        created_at: chrono::Utc::now(),
        next_run: chrono::Utc::now(),
        last_run: None,
        last_status: None,
        last_output: None,
        origin,
    }
}

#[test]
fn channel_origin_job_runs_as_external_channel_turn() {
    let job = job_with_origin(Some(JobOrigin::Channel {
        channel: "telegram".into(),
        reply_target: "42".into(),
        history_key: "k".into(),
        sender: Some("alice".into()),
        thread_id: None,
    }));
    match turn_origin_for_job_run(&job, "run-1") {
        AgentTurnOrigin::ExternalChannel {
            sender_name: _,
            channel,
            sender,
            reply_target,
            message_id,
            history_key,
        } => {
            assert_eq!(channel, "telegram");
            assert_eq!(sender.as_deref(), Some("alice"));
            assert_eq!(reply_target, "42");
            assert_eq!(message_id, format!("cron:{}:run-1", job.id));
            assert_eq!(history_key.as_deref(), Some("k"));
        }
        other => panic!("expected ExternalChannel, got {other:?}"),
    }
}

#[test]
fn web_and_origin_less_jobs_keep_trusted_cron() {
    for origin in [
        None,
        Some(JobOrigin::Web {
            thread_id: "t".into(),
            agent_id: None,
        }),
    ] {
        let job = job_with_origin(origin);
        assert!(matches!(
            turn_origin_for_job_run(&job, "r"),
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::Cron,
                ..
            }
        ));
    }
}

// ── approval-gate decision for creating a job ──────────────────────

fn ch() -> AgentTurnOrigin {
    channel_turn(Some("telegram_alice_42"))
}

#[test]
fn gate_skipped_for_default_origin_delivery() {
    let args = json!({"name": "n", "prompt": "drink water", "schedule": {"kind": "every", "every_ms": 300000}});
    assert!(is_self_scoped_agent_job(&args, Some(&ch())));
}

#[test]
fn gate_skipped_for_explicit_origin_mode_and_matching_announce() {
    let origin_mode = json!({"job_type": "agent", "prompt": "p", "delivery": {"mode": "origin"}});
    assert!(is_self_scoped_agent_job(&origin_mode, Some(&ch())));
    let announce =
        json!({"prompt": "p", "delivery": {"mode": "announce", "channel": "Telegram", "to": "42"}});
    assert!(is_self_scoped_agent_job(&announce, Some(&ch())));
}

#[test]
fn gate_kept_for_every_other_case() {
    let cases = [
        // shell job
        json!({"job_type": "shell", "command": "ls"}),
        // agent job carrying a shell command
        json!({"job_type": "agent", "prompt": "p", "command": "rm -rf /"}),
        // no prompt and no job_type resolves to shell
        json!({"schedule": {"kind": "every", "every_ms": 300000}}),
        // other recipient on the same channel
        json!({"prompt": "p", "delivery": {"mode": "announce", "channel": "telegram", "to": "99"}}),
        // same target on another channel
        json!({"prompt": "p", "delivery": {"mode": "announce", "channel": "discord", "to": "42"}}),
        // not origin-scoped delivery modes
        json!({"prompt": "p", "delivery": {"mode": "proactive"}}),
        json!({"prompt": "p", "delivery": {"mode": "none"}}),
        json!({"prompt": "p", "delivery": {}}),
    ];
    for args in cases {
        assert!(!is_self_scoped_agent_job(&args, Some(&ch())), "{args}");
    }
}

#[test]
fn gate_kept_for_non_channel_turns() {
    let args = json!({"prompt": "p"});
    assert!(!is_self_scoped_agent_job(&args, None));
    assert!(!is_self_scoped_agent_job(&args, Some(&web_turn("t"))));
    assert!(!is_self_scoped_agent_job(&args, Some(&channel_turn(None))));
    assert!(!is_self_scoped_agent_job(
        &args,
        Some(&AgentTurnOrigin::Cli)
    ));
}

#[tokio::test]
async fn tools_report_external_effect_from_the_live_turn() {
    use crate::config::Config;
    use crate::security::SecurityPolicy;
    use std::sync::Arc;
    use tinytools::Tool;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = Arc::new(Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    });
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    ));
    let add = crate::cron::tools::CronAddTool::new(Arc::clone(&config), Arc::clone(&security));
    let collapsed = crate::cron::tools::CronTool::new(config, security);
    let agent = json!({"prompt": "p", "schedule": {"kind": "every", "every_ms": 300000}});
    let shell = json!({"job_type": "shell", "command": "ls", "schedule": {"kind": "every", "every_ms": 300000}});

    // Outside any turn nothing is exempt.
    assert!(add.external_effect_with_args(&agent));

    let (add_agent, add_shell, via_cron) = with_origin(ch(), async {
        let mut with_action = agent.clone();
        with_action["action"] = json!("add");
        (
            add.external_effect_with_args(&agent),
            add.external_effect_with_args(&shell),
            collapsed.external_effect_with_args(&with_action),
        )
    })
    .await;
    assert!(!add_agent, "self-scoped agent job skips the gate");
    assert!(add_shell, "shell job stays gated");
    assert!(!via_cron, "the collapsed `cron` tool agrees");
}

#[test]
fn announce_to_own_chat_keeps_the_gate_unless_the_origin_is_retained() {
    // `create_agent_job` drops the origin for a non-`current` session target
    // with an announce delivery, so that job would run as TrustedAutomation.
    for target in ["isolated", "main", "session:x"] {
        let args = json!({
            "prompt": "p",
            "session_target": target,
            "delivery": {"mode": "announce", "channel": "telegram", "to": "42"}
        });
        assert!(
            !is_self_scoped_agent_job(&args, Some(&ch())),
            "{target} must stay gated"
        );
    }
    let current = json!({
        "prompt": "p",
        "session_target": "current",
        "delivery": {"mode": "announce", "channel": "telegram", "to": "42"}
    });
    assert!(is_self_scoped_agent_job(&current, Some(&ch())));
}
