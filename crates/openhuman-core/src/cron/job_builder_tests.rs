use super::*;
use crate::cron::list_jobs;
use tempfile::TempDir;

fn config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn input() -> AgentJobInput {
    AgentJobInput {
        name: Some("water".into()),
        schedule: Schedule::Every { every_ms: 300_000 },
        prompt: "drink water".into(),
        session_target: None,
        model: None,
        delivery: None,
        delete_after_run: false,
    }
}

fn web() -> JobOrigin {
    JobOrigin::Web {
        thread_id: "t-1".into(),
        agent_id: None,
    }
}

fn channel() -> JobOrigin {
    JobOrigin::Channel {
        channel: "telegram".into(),
        reply_target: "42".into(),
        history_key: "telegram_alice_42".into(),
        sender: Some("alice".into()),
        thread_id: None,
    }
}

#[test]
fn origin_defaults_to_current_target_and_origin_delivery() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let job = create_agent_job(&cfg, input(), Some(web())).unwrap();
    assert_eq!(job.session_target, SessionTarget::Current);
    assert_eq!(job.delivery.mode, "origin");
    assert_eq!(job.origin, Some(web()));
    // And it round-trips through the store.
    assert_eq!(list_jobs(&cfg).unwrap()[0].origin, Some(web()));
}

#[test]
fn no_origin_keeps_isolated_and_proactive() {
    let tmp = TempDir::new().unwrap();
    let job = create_agent_job(&config(&tmp), input(), None).unwrap();
    assert_eq!(job.session_target, SessionTarget::Isolated);
    assert_eq!(job.delivery.mode, "proactive");
    assert_eq!(job.origin, None);
}

#[test]
fn explicit_delivery_and_target_win_and_drop_the_unused_origin() {
    let tmp = TempDir::new().unwrap();
    let mut i = input();
    i.session_target = Some(SessionTarget::Isolated);
    i.delivery = Some(DeliveryConfig {
        mode: "none".into(),
        channel: None,
        to: None,
        best_effort: true,
    });
    let job = create_agent_job(&config(&tmp), i, Some(web())).unwrap();
    assert_eq!(job.session_target, SessionTarget::Isolated);
    assert_eq!(job.delivery.mode, "none");
    assert_eq!(
        job.origin, None,
        "nothing uses the origin, so it is not kept"
    );
}

#[test]
fn origin_delivery_or_current_target_without_an_origin_is_an_error() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let mut i = input();
    i.delivery = Some(
        crate::cron::client::blocking(&Config::default(), |config| async move {
            crate::cron::client::default_delivery(&config, Some(&web())).await
        })
        .unwrap(),
    );
    assert!(create_agent_job(&cfg, i, None)
        .unwrap_err()
        .contains("origin"));
    let mut i = input();
    i.session_target = Some(SessionTarget::Current);
    assert!(create_agent_job(&cfg, i, None)
        .unwrap_err()
        .contains("current"));
}

fn announce(channel_name: &str, to: &str) -> DeliveryConfig {
    DeliveryConfig {
        mode: "announce".into(),
        channel: Some(channel_name.into()),
        to: Some(to.into()),
        best_effort: true,
    }
}

#[test]
fn announce_to_origin_reply_target_skips_allowed_users() {
    let tmp = TempDir::new().unwrap();
    let mut cfg = config(&tmp);
    cfg.channels_config.telegram = Some(crate::config::TelegramConfig {
        bot_token: "t".into(),
        chat_id: None,
        allowed_users: vec!["alice".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    // Chat id 42 is not in allowed_users (a username), but it is the asker.
    assert!(
        validate_delivery_for_config(&cfg, &announce("telegram", "42"), Some(&channel())).is_ok()
    );
    // Without the origin, or for another target, the allowlist still applies.
    assert!(validate_delivery_for_config(&cfg, &announce("telegram", "42"), None).is_err());
    assert!(
        validate_delivery_for_config(&cfg, &announce("telegram", "99"), Some(&channel())).is_err()
    );
    assert!(
        validate_delivery_for_config(&cfg, &announce("discord", "42"), Some(&channel())).is_err()
    );
}
