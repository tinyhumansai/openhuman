use super::*;
use std::sync::Arc;

fn offline_config(root: &std::path::Path) -> Arc<Config> {
    let mut config = Config::default();
    config.action_dir = root.into();
    config.workspace_dir = root.join("internal");
    config.config_path = config.workspace_dir.join("config.toml");
    config.modules.enabled = false;
    config.modules.allow_download = false;
    Arc::new(config)
}

/// Writes an upload at `relative` under `root` with `readout` cached beside
/// it, as a provider fallback writes it.
async fn upload_with_readout(root: &std::path::Path, relative: &str, readout: &str) {
    let path = root.join(relative);
    let derived = path.parent().unwrap().join(".openhuman-derived");
    tokio::fs::create_dir_all(&derived).await.unwrap();
    tokio::fs::write(&path, b"%PDF-1.4 original bytes")
        .await
        .unwrap();
    tokio::fs::write(derived.join("readout.txt"), format!("key\n{readout}"))
        .await
        .unwrap();
}

fn scope_at(root: &std::path::Path) -> AttachmentAccessScope {
    AttachmentAccessScope {
        workspace: Some(root.into()),
        ..AttachmentAccessScope::default()
    }
}

#[tokio::test]
async fn cached_readout_chars_counts_characters_after_the_key() {
    let temp = tempfile::tempdir().unwrap();
    let relative = "uploads/thread/id/notes.pdf";
    upload_with_readout(temp.path(), relative, "ಕನ್ನಡ notes").await;
    let config = offline_config(temp.path());

    let chars = cached_readout_chars(&config, relative, &scope_at(temp.path())).await;

    assert_eq!(chars, Some("ಕನ್ನಡ notes".chars().count() as u64));
}

#[tokio::test]
async fn cached_readout_chars_is_none_without_a_readout() {
    let temp = tempfile::tempdir().unwrap();
    let relative = "uploads/thread/id/brief.pdf";
    let path = temp.path().join(relative);
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&path, b"%PDF-1.4").await.unwrap();
    let config = offline_config(temp.path());

    assert_eq!(
        cached_readout_chars(&config, relative, &scope_at(temp.path())).await,
        None
    );
}

#[tokio::test]
async fn cached_readout_chars_ignores_files_outside_uploads() {
    let temp = tempfile::tempdir().unwrap();
    let relative = "notes/thread/id/brief.pdf";
    upload_with_readout(temp.path(), relative, "outside the uploads root").await;
    let config = offline_config(temp.path());

    assert_eq!(
        cached_readout_chars(&config, relative, &scope_at(temp.path())).await,
        None
    );
}

#[cfg(unix)]
#[tokio::test]
async fn cached_readout_chars_refuses_a_symlinked_readout() {
    let temp = tempfile::tempdir().unwrap();
    let relative = "uploads/thread/id/brief.pdf";
    upload_with_readout(temp.path(), relative, "real").await;
    let derived = temp.path().join("uploads/thread/id/.openhuman-derived");
    let elsewhere = temp.path().join("elsewhere.txt");
    tokio::fs::write(&elsewhere, "key\nsomething else")
        .await
        .unwrap();
    tokio::fs::remove_file(derived.join("readout.txt"))
        .await
        .unwrap();
    std::os::unix::fs::symlink(&elsewhere, derived.join("readout.txt")).unwrap();
    let config = offline_config(temp.path());

    assert_eq!(
        cached_readout_chars(&config, relative, &scope_at(temp.path())).await,
        None
    );
}
