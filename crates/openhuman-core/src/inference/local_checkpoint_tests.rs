use super::*;
use crate::config::rpc::LocalAiSettingsPatch;

/// One profile's config on disk; every step loads it fresh, as the RPC does.
struct Profile {
    dir: tempfile::TempDir,
}

impl Profile {
    /// A profile with OMLX connected and keyed: the state a failed switch
    /// must keep.
    async fn with_omlx() -> Self {
        let profile = Self::empty().await;
        profile
            .save(
                connect("omlx", "http://127.0.0.1:8080/v1", "omlx-key"),
                false,
            )
            .await;
        profile
    }

    async fn empty() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = dir.path().join("workspace");
        cfg.config_path = dir.path().join("config.toml");
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
        cfg.save().await.expect("initial save");
        Self { dir }
    }

    async fn load(&self) -> Result<Config, String> {
        Config::load_from_config_path_without_env(
            &self.dir.path().join("config.toml"),
            &self.dir.path().join("workspace"),
        )
        .await
        .map_err(|e| e.to_string())
    }

    async fn local_ai(&self) -> LocalAiConfig {
        self.load().await.expect("load").local_ai
    }

    /// Saves `patch`; returns the checkpoint id when `checkpoint` is set.
    async fn save(&self, patch: LocalAiSettingsPatch, checkpoint: bool) -> Option<String> {
        let outcome = apply(|| self.load(), patch, checkpoint)
            .await
            .expect("save");
        outcome.value["checkpoint_id"].as_str().map(str::to_string)
    }

    async fn attempt(&self, patch: LocalAiSettingsPatch) -> String {
        self.save(patch, true).await.expect("checkpoint id")
    }

    async fn rollback(&self, id: &str) -> bool {
        rollback(|| self.load(), id).await.expect("rollback")
    }
}

fn connect(provider: &str, base_url: &str, api_key: &str) -> LocalAiSettingsPatch {
    LocalAiSettingsPatch {
        runtime_enabled: Some(true),
        opt_in_confirmed: Some(true),
        provider: Some(provider.into()),
        base_url: Some(Some(base_url.into())),
        api_key: Some(api_key.into()),
        ..LocalAiSettingsPatch::default()
    }
}

fn lm_studio(key: &str) -> LocalAiSettingsPatch {
    connect("lm_studio", "http://127.0.0.1:1234/v1", key)
}

#[tokio::test]
async fn a_failed_attempt_restores_the_previous_runtime_and_key() {
    let profile = Profile::with_omlx().await;

    let id = profile.attempt(lm_studio("wrong-key")).await;
    assert_eq!(profile.local_ai().await.provider, "lm_studio");

    assert!(profile.rollback(&id).await);
    let restored = profile.local_ai().await;
    assert_eq!(restored.provider, "omlx");
    assert_eq!(
        restored.base_url.as_deref(),
        Some("http://127.0.0.1:8080/v1")
    );
    assert_eq!(restored.api_key.as_deref(), Some("omlx-key"));
    assert!(
        !profile.rollback(&id).await,
        "a checkpoint is restored once"
    );
}

#[tokio::test]
async fn a_failed_first_connection_turns_the_runtime_back_off() {
    let profile = Profile::empty().await;
    let before = profile.local_ai().await;

    let id = profile
        .attempt(connect("ollama", "http://127.0.0.1:11434", ""))
        .await;
    assert!(profile.rollback(&id).await);
    let after = profile.local_ai().await;
    assert_eq!(after.runtime_enabled, before.runtime_enabled);
    assert_eq!(after.base_url, None);
}

#[tokio::test]
async fn an_older_attempts_rollback_does_not_undo_a_newer_save() {
    let profile = Profile::with_omlx().await;

    let older = profile
        .attempt(connect("ollama", "http://127.0.0.1:11434", ""))
        .await;
    let newer = profile.attempt(lm_studio("lm-key")).await;

    assert!(!profile.rollback(&older).await);
    assert_eq!(
        profile.local_ai().await.provider,
        "lm_studio",
        "the newer save stands"
    );
    assert!(profile.rollback(&newer).await);
}

/// Saving again, even the very same values, means the attempt's settings
/// were chosen again: its rollback must not restore over them.
#[tokio::test]
async fn a_later_save_drops_the_checkpoint_even_with_the_same_values() {
    let profile = Profile::with_omlx().await;

    let id = profile.attempt(lm_studio("lm-key")).await;
    profile.save(lm_studio("lm-key"), false).await;

    assert!(!profile.rollback(&id).await);
    assert_eq!(profile.local_ai().await.provider, "lm_studio");
}

#[tokio::test]
async fn a_rollback_after_the_section_changed_elsewhere_changes_nothing() {
    let profile = Profile::with_omlx().await;

    let id = profile.attempt(lm_studio("lm-key")).await;
    // A writer outside `update_local_settings` (no checkpoint bookkeeping).
    let mut cfg = profile.load().await.expect("load");
    config_rpc::apply_local_ai_settings(
        &mut cfg,
        LocalAiSettingsPatch {
            chat_model_id: Some("picked-later".into()),
            ..LocalAiSettingsPatch::default()
        },
    )
    .await
    .expect("unrelated save");

    assert!(!profile.rollback(&id).await);
    let current = profile.local_ai().await;
    assert_eq!(current.provider, "lm_studio");
    assert_eq!(current.chat_model_id, "picked-later");
}

#[tokio::test]
async fn a_checkpoint_only_restores_into_its_own_config() {
    let a = Profile::with_omlx().await;
    let b = Profile::with_omlx().await;

    let id = a.attempt(lm_studio("lm-key")).await;

    assert!(!b.rollback(&id).await);
    assert_eq!(b.local_ai().await.provider, "omlx");
    assert!(a.rollback(&id).await);
}
