use super::*;
use crate::openhuman::orchestration::store;
use std::sync::Arc;

/// The factory test override is process-global, so tests that install a scripted
/// provider must not run concurrently. Poison-tolerant serialization lock.
static PROVIDER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A scripted provider so `create_chat_provider` returns a canned steering
/// synthesis without any network (the factory test override).
struct ScriptedProvider {
    reply: String,
}
#[async_trait]
impl crate::openhuman::inference::provider::Provider for ScriptedProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(self.reply.clone())
    }
}

fn test_config(dir: &std::path::Path) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = dir.to_path_buf();
    cfg.orchestration.enabled = true;
    // A BYO provider signature so any downstream provider gate is deterministic.
    cfg.subconscious_provider = Some("groq".to_string());
    cfg
}

/// Seed one compressed-history row + one world-diff entry so a review has data.
fn seed_activity(config: &Config, tag: &str) {
    store::with_connection(&config.workspace_dir, |conn| {
        store::insert_compressed(
            conn,
            &format!("h1#{tag}"),
            "h1",
            "@me",
            400,
            20,
            &format!("did work {tag}"),
            &format!("2026-07-02T00:0{tag}:00Z"),
        )?;
        store::append_world_diff(
            conn,
            &format!("h1#{tag}"),
            "h1",
            "@me",
            "sig",
            &format!("world moved {tag}"),
            "delta",
            &format!("2026-07-02T00:0{tag}:00Z"),
        )?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn tinyplace_profile_metadata() {
    let profile = TinyPlaceProfile;
    assert_eq!(profile.id(), "tinyplace");
    // Always tainted — steering reacts to third-party harness content.
    let tainted = profile.origin(&Observation::default());
    assert!(matches!(
        tainted,
        TrustedAutomationSource::SubconsciousTainted
    ));
}

#[test]
fn cadence_defaults_to_heartbeat_interval_then_honours_override() {
    let mut cfg = Config::default();
    cfg.heartbeat.interval_minutes = 7;
    // No explicit review interval → follows the heartbeat.
    assert_eq!(
        TinyPlaceProfile.cadence(&cfg),
        std::time::Duration::from_secs(7 * 60)
    );
    // Explicit override wins.
    cfg.orchestration.review_interval_minutes = Some(3);
    assert_eq!(
        TinyPlaceProfile.cadence(&cfg),
        std::time::Duration::from_secs(3 * 60)
    );
}

#[tokio::test]
async fn observe_is_quiet_when_orchestration_disabled_or_empty() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = test_config(dir.path());

    // Disabled → quiet regardless of data.
    cfg.orchestration.enabled = false;
    let obs = TinyPlaceProfile.observe(&cfg).await;
    assert!(!obs.has_changes);

    // Enabled but empty store → quiet.
    cfg.orchestration.enabled = true;
    let obs = TinyPlaceProfile.observe(&cfg).await;
    assert!(!obs.has_changes, "no unreviewed history → quiet");
}

#[tokio::test]
async fn observe_reflect_commit_emits_directive_then_advances_cursor() {
    let _serial = PROVIDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    seed_activity(&config, "1");

    let _guard = crate::openhuman::inference::provider::factory::test_provider_override::install(
        Arc::new(ScriptedProvider {
            reply: "STEERING_DIRECTIVE: prioritize the billing migration\n\
                    expires_after_cycles: 12"
                .to_string(),
        }),
    );

    let profile = TinyPlaceProfile;

    // observe → changed window, tainted, with a commit cursor token.
    let obs = profile.observe(&config).await;
    assert!(obs.has_changes);
    assert!(obs.has_external_content);
    assert!(
        obs.commit_token.is_some(),
        "carries the review cursor token"
    );

    // reflect → a directive is synthesized + persisted.
    let reflection = profile.reflect(&config, &obs, "").await.unwrap();
    let directive_id = match reflection {
        Reflection::Steered { directive_id } => directive_id,
        other => panic!("expected Steered, got {other:?}"),
    };
    assert!(directive_id > 0);
    store::with_connection(&config.workspace_dir, |conn| {
        let cur = store::current_steering_directive(conn, 0)?.expect("current directive");
        assert_eq!(cur.text, "prioritize the billing migration");
        Ok(())
    })
    .unwrap();

    // Before commit, the cursor is still empty (the split moved advance here).
    let before = store::with_connection(&config.workspace_dir, store::review_cursor).unwrap();
    assert!(before.is_empty(), "cursor not advanced until commit");

    // commit → cursor advances to the observed window.
    profile.commit(&config, &obs).await;
    let after = store::with_connection(&config.workspace_dir, store::review_cursor).unwrap();
    assert!(!after.is_empty(), "commit advanced the review cursor");

    // A re-observe over the same (now-reviewed) data is quiet — idempotent.
    let requiet = profile.observe(&config).await;
    assert!(!requiet.has_changes, "re-tick with no new data is quiet");
}

#[tokio::test]
async fn clean_none_is_idle_not_error() {
    let _serial = PROVIDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    seed_activity(&config, "1");

    let _guard = crate::openhuman::inference::provider::factory::test_provider_override::install(
        Arc::new(ScriptedProvider {
            reply: "NONE".to_string(),
        }),
    );

    let profile = TinyPlaceProfile;
    let obs = profile.observe(&config).await;
    assert!(obs.has_changes);
    // A clean NONE is an idle result — not an Err (so the cursor still advances).
    let reflection = profile.reflect(&config, &obs, "").await.unwrap();
    assert!(matches!(reflection, Reflection::Idle));
}

#[test]
fn steering_reflect_path_imports_no_agent_or_channel_symbols() {
    // Isolation invariant (stage 6): the tinyplace reflect path is a tool-free
    // provider chat — it must construct no Agent and reference no outbound
    // channel/send-message symbols. Source-scan the profile module itself.
    const SRC: &str = include_str!("tinyplace.rs");
    for forbidden in [
        "Agent::from_config",
        "agent_tools",
        "send_message",
        "run_single",
        "spawn_async_subagent",
    ] {
        assert!(
            !SRC.contains(forbidden),
            "tinyplace profile must not reference `{forbidden}` (isolation invariant)"
        );
    }
}
