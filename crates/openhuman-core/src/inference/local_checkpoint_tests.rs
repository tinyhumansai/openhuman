use super::*;
use crate::config::rpc::LocalAiSettingsPatch;

fn tmp_config(tmp: &tempfile::TempDir) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");
    cfg.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    cfg
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

fn checkpoint_id(outcome: &Outcome<Value>) -> String {
    outcome.value["checkpoint_id"]
        .as_str()
        .expect("checkpoint id in the snapshot")
        .to_string()
}

/// A saved OMLX runtime with its key, the state a failed switch must keep.
async fn connected_omlx(tmp: &tempfile::TempDir) -> Config {
    let mut cfg = tmp_config(tmp);
    config_rpc::apply_local_ai_settings(
        &mut cfg,
        connect("omlx", "http://127.0.0.1:8080/v1", "omlx-key"),
    )
    .await
    .expect("connect omlx");
    cfg
}

#[tokio::test]
async fn a_failed_attempt_restores_the_previous_runtime_and_key() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = connected_omlx(&tmp).await;

    let outcome = apply_with_checkpoint(
        &mut cfg,
        connect("lm_studio", "http://127.0.0.1:1234/v1", "wrong-key"),
    )
    .await
    .expect("attempt");
    assert_eq!(cfg.local_ai.provider, "lm_studio");

    assert!(rollback(&mut cfg, &checkpoint_id(&outcome))
        .await
        .expect("rollback"));
    assert_eq!(cfg.local_ai.provider, "omlx");
    assert_eq!(
        cfg.local_ai.base_url.as_deref(),
        Some("http://127.0.0.1:8080/v1")
    );
    assert_eq!(cfg.local_ai.api_key.as_deref(), Some("omlx-key"));

    let on_disk: Config = toml::from_str(&std::fs::read_to_string(&cfg.config_path).unwrap())
        .expect("saved config parses");
    assert_eq!(on_disk.local_ai.provider, "omlx", "the restore is saved");
}

#[tokio::test]
async fn a_failed_first_connection_turns_the_runtime_back_off() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let was_enabled = cfg.local_ai.runtime_enabled;

    let outcome = apply_with_checkpoint(&mut cfg, connect("ollama", "http://127.0.0.1:11434", ""))
        .await
        .expect("attempt");
    assert!(rollback(&mut cfg, &checkpoint_id(&outcome))
        .await
        .expect("rollback"));
    assert_eq!(cfg.local_ai.runtime_enabled, was_enabled);
    assert_eq!(cfg.local_ai.base_url, None);
}

#[tokio::test]
async fn an_older_attempts_rollback_does_not_undo_a_newer_save() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = connected_omlx(&tmp).await;

    let older = apply_with_checkpoint(&mut cfg, connect("ollama", "http://127.0.0.1:11434", ""))
        .await
        .expect("older attempt");
    let newer = apply_with_checkpoint(
        &mut cfg,
        connect("lm_studio", "http://127.0.0.1:1234/v1", "lm-key"),
    )
    .await
    .expect("newer attempt");

    assert!(!rollback(&mut cfg, &checkpoint_id(&older))
        .await
        .expect("older rollback"));
    assert_eq!(cfg.local_ai.provider, "lm_studio", "the newer save stands");
    assert!(rollback(&mut cfg, &checkpoint_id(&newer))
        .await
        .expect("newer rollback"));
}

#[tokio::test]
async fn a_rollback_after_another_save_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = connected_omlx(&tmp).await;

    let outcome = apply_with_checkpoint(
        &mut cfg,
        connect("lm_studio", "http://127.0.0.1:1234/v1", "lm-key"),
    )
    .await
    .expect("attempt");
    config_rpc::apply_local_ai_settings(
        &mut cfg,
        LocalAiSettingsPatch {
            chat_model_id: Some("picked-later".into()),
            ..LocalAiSettingsPatch::default()
        },
    )
    .await
    .expect("unrelated save");

    assert!(!rollback(&mut cfg, &checkpoint_id(&outcome))
        .await
        .expect("rollback"));
    assert_eq!(cfg.local_ai.provider, "lm_studio");
    assert_eq!(cfg.local_ai.chat_model_id, "picked-later");
}

#[tokio::test]
async fn a_checkpoint_only_restores_into_its_own_config() {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let mut a = connected_omlx(&a_dir).await;
    let mut b = connected_omlx(&b_dir).await;

    let outcome = apply_with_checkpoint(
        &mut a,
        connect("lm_studio", "http://127.0.0.1:1234/v1", "lm-key"),
    )
    .await
    .expect("attempt on a");

    assert!(!rollback(&mut b, &checkpoint_id(&outcome))
        .await
        .expect("rollback on b"));
    assert_eq!(b.local_ai.provider, "omlx");
    assert!(rollback(&mut a, &checkpoint_id(&outcome))
        .await
        .expect("rollback on a"));
}
