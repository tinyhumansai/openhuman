use super::*;

fn config_in(dir: &tempfile::TempDir) -> Config {
    Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    }
}

/// The RPC takes free text from a settings toggle, so `"GitHub"`,
/// `" github "` and `"github"` have to reach one row.
#[test]
fn kv_key_normalises_the_toolkit() {
    assert_eq!(kv_key(" GitHub "), "github");
    assert_eq!(kv_key("SLACK"), "slack");
    assert_eq!(kv_key("gmail"), "gmail");
    assert_eq!(kv_key("   "), "", "an all-whitespace toolkit has no row");
}

/// The stored JSON is the three boolean fields.
#[test]
fn stored_value_is_the_three_boolean_fields() {
    let value = serde_json::to_value(UserScopePref {
        read: true,
        write: false,
        admin: true,
    })
    .expect("UserScopePref serialises");
    assert_eq!(
        value,
        serde_json::json!({ "read": true, "write": false, "admin": true })
    );
}

/// The default a failed or absent read falls back to: productive, not
/// permissive-with-admin.
#[test]
fn default_pref_is_read_write_without_admin() {
    let pref = UserScopePref::default();
    assert!(pref.read);
    assert!(pref.write);
    assert!(!pref.admin, "admin must stay opt-in");
}

#[tokio::test]
async fn save_then_load_round_trips_per_toolkit() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    let pref = UserScopePref {
        read: true,
        write: false,
        admin: true,
    };
    save(&config, "GitHub", pref).await.unwrap();

    let loaded = load_or_default(&config, " github ").await;
    assert_eq!(
        serde_json::to_value(loaded).unwrap(),
        serde_json::to_value(pref).unwrap()
    );
    let other = load_or_default(&config, "slack").await;
    assert!(other.read && other.write && !other.admin);
}

#[tokio::test]
async fn save_rejects_an_empty_toolkit() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    assert!(save(&config, "  ", UserScopePref::default()).await.is_err());
}

#[tokio::test]
async fn unreadable_store_fails_open_to_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    let path = file_store::path(&config, USER_SCOPES_FILE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"not json").unwrap();

    let pref = load_or_default(&config, "github").await;
    assert!(pref.read && pref.write && !pref.admin);
    assert!(save(&config, "github", UserScopePref::default())
        .await
        .is_err());
}
