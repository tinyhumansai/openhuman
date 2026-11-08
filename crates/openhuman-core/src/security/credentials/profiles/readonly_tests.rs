use super::*;

fn config(tmp: &tempfile::TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

fn write_app_session_profile(config: &Config, token: &str) {
    let state_dir = config.config_path.parent().expect("config state directory");
    std::fs::create_dir_all(state_dir).expect("create profile fixture directory");
    let doc = serde_json::json!({
        "schema_version": 1,
        "active_profiles": {},
        "profiles": {
            "app-session:default": {
                "provider": "app-session",
                "profile_name": "default",
                "kind": "token",
                "token": token,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "metadata": {},
            }
        }
    });
    std::fs::write(
        state_dir.join(PROFILES_FILENAME),
        serde_json::to_vec(&doc).expect("serialize profile fixture"),
    )
    .expect("write profile fixture");
}

fn tree(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn visit(root: &std::path::Path, at: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(at).expect("read fixture tree") {
            let path = entry.expect("read fixture entry").path();
            out.push(path.strip_prefix(root).expect("relative path").to_path_buf());
            if path.is_dir() {
                visit(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}

#[tokio::test]
async fn plaintext_app_session_lookup_creates_no_storage() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = config(&tmp);
    write_app_session_profile(&config, "plaintext-session");
    let before = tree(tmp.path());

    let profile = load_profile(&config, "app-session", None)
        .await
        .expect("plaintext profile is readable without a store")
        .expect("profile present");

    assert_eq!(profile.token.as_deref(), Some("plaintext-session"));
    assert!(
        !config.workspace_dir.join("storage").exists(),
        "a read-only profile lookup must not create the storage store"
    );
    assert_eq!(tree(tmp.path()), before, "a read-only lookup must not write");
}
