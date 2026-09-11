use super::*;

// ── config.toml.bak holds the PREVIOUS config (#6205 review) ────────────
//
// Found in review, not by us: the backup was copied from the temp file, so
// `.bak` became a duplicate of the config that had just been written and the
// previous one was gone. Recovery had nothing older to fall back to, and the
// failed-rename path copied those new bytes over the live config while
// reporting `Err`.

/// After a save, `config.toml.bak` must hold what `config.toml` held *before*
/// it — not a second copy of what was just written.
///
/// `load_or_init`'s corruption recovery reads this file to get back a working
/// config. A `.bak` that mirrors the live file cannot do that, and the mismatch
/// is invisible until the day it is needed.
#[tokio::test]
async fn a_save_backs_up_the_previous_config_not_the_new_one() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = load_or_init_for_workspace(tmp.path()).await;
    config.default_model = Some("first-generation-model".to_string());
    config.save().await.expect("first save");

    // The config as it stands on disk immediately before the save under test.
    let previous = tokio::fs::read_to_string(&config.config_path)
        .await
        .unwrap();
    assert!(
        previous.contains("first-generation-model")
            && !previous.contains("second-generation-model"),
        "precondition: disk holds the first generation and not the second"
    );

    config.default_model = Some("second-generation-model".to_string());
    config.save().await.expect("second save");

    let backup_path = config.config_path.with_file_name("config.toml.bak");
    let backed_up = tokio::fs::read_to_string(&backup_path)
        .await
        .expect("a save over an existing config must leave a .bak");
    let live = tokio::fs::read_to_string(&config.config_path)
        .await
        .unwrap();

    assert!(
        live.contains("second-generation-model"),
        "the live config must hold the new contents"
    );
    assert!(
        !backed_up.contains("second-generation-model"),
        "the backup must hold the PREVIOUS config, not a copy of the new one"
    );
    // Not asserted byte-for-byte: the backup is re-serialized through the same
    // secret encryption the live config gets, so that replacing a legacy
    // plaintext config cannot leave the plaintext behind in `.bak`
    // (`config_secrets_encrypted_on_save_decrypted_on_load` pins that). What
    // must hold is that it *describes the previous config* and that recovery,
    // which parses this file, can still load it.
    assert!(
        backed_up.contains("first-generation-model"),
        "the backup must hold the previous config's contents, got:\n{backed_up}"
    );
    let recovered: crate::openhuman::config::Config = toml::from_str(&backed_up)
        .expect("the backup must parse as a config, since recovery loads it");
    assert_eq!(
        recovered.default_model.as_deref(),
        Some("first-generation-model"),
        "the backup must describe the config that was replaced"
    );
}

/// A first-ever write has no existing config to preserve, and that must not be
/// an error — nor may it invent a `.bak` seeded with the new contents.
#[tokio::test]
async fn a_first_ever_save_succeeds_without_writing_a_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let config = load_or_init_for_workspace(tmp.path()).await;

    assert!(
        tokio::fs::try_exists(&config.config_path).await.unwrap(),
        "the initial save must have written the config"
    );
    let backup_path = config.config_path.with_file_name("config.toml.bak");
    assert!(
        !tokio::fs::try_exists(&backup_path).await.unwrap(),
        "a first-ever write has nothing to back up, so no .bak should exist"
    );
}

/// A backup of an already-at-rest config must be the file itself, byte for
/// byte — including anything this build does not model.
///
/// `Config` ignores unknown keys, so a round trip through parse-and-serialize
/// silently drops them. A rollback, a downgrade or a bad migration is exactly
/// when a backup is read, and exactly when the dropped keys would be wanted, so
/// the verbatim copy is the default and only a secret upgrade displaces it.
#[tokio::test]
async fn a_backup_of_an_at_rest_config_is_byte_for_byte() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = load_or_init_for_workspace(tmp.path()).await;

    // Something a future build wrote and this one has never heard of.
    let doctored = format!(
        "{}\nkey_from_a_build_we_do_not_know = \"preserve-me\"\n",
        tokio::fs::read_to_string(&config.config_path)
            .await
            .unwrap()
    );
    tokio::fs::write(&config.config_path, &doctored)
        .await
        .unwrap();

    config.default_model = Some("post-doctoring-model".to_string());
    config.save().await.expect("save over the doctored config");

    let backup_path = config.config_path.with_file_name("config.toml.bak");
    let backed_up = tokio::fs::read_to_string(&backup_path).await.unwrap();

    assert_eq!(
        backed_up, doctored,
        "an at-rest config must be backed up verbatim, not re-serialized"
    );
    assert!(
        backed_up.contains("key_from_a_build_we_do_not_know"),
        "a key this build does not model must survive into the backup"
    );
    assert!(
        !tokio::fs::read_to_string(&config.config_path)
            .await
            .unwrap()
            .contains("key_from_a_build_we_do_not_know"),
        "precondition: the live config does drop it, which is why the backup must not"
    );
}

/// The narrow exception: when the save is upgrading secrets in place, the
/// backup must hold the upgraded form, not the pre-upgrade bytes.
///
/// `load_or_init` triggers exactly this save when it force-migrates a legacy
/// `enc:` secret, for the stated purpose of making the insecure ciphertext stop
/// living on disk. A verbatim `.bak` would keep it there permanently.
#[tokio::test]
async fn a_backup_never_preserves_a_secret_the_save_is_upgrading() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    let config_path = tmp.path().join("config.toml");
    let secret = "plaintext-token-that-must-not-survive";

    std::fs::write(
        &config_path,
        format!("[channels_config.telegram]\nbot_token = \"{secret}\"\nallowed_users = []\n"),
    )
    .unwrap();

    let mut cfg = Config {
        config_path: config_path.clone(),
        workspace_dir,
        ..Default::default()
    };
    cfg.channels_config.telegram = Some(TelegramConfig {
        bot_token: secret.to_string(),
        chat_id: None,
        allowed_users: vec![],
        stream_mode: StreamMode::Off,
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    cfg.save().await.unwrap();

    let backup_path = config_path.with_file_name("config.toml.bak");
    let backed_up = tokio::fs::read_to_string(&backup_path).await.unwrap();
    assert!(
        !backed_up.contains(secret),
        "the pre-upgrade plaintext secret must not survive in .bak, got:\n{backed_up}"
    );
}
