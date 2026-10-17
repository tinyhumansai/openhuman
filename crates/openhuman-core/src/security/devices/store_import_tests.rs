use super::*;
use crate::security::devices::store as live;
use crate::storage::import::{run_with, Outcome};
use tempfile::TempDir;

fn workspace() -> (TempDir, Config) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    (dir, config)
}

/// Writes a row the way the retired store did.
fn seed(config: &Config, channel: &str, label: &str, pubkey: &str, revoked: bool, seen: bool) {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO paired_devices \
             (channel_id, label, device_pubkey, core_session_token_hash, \
              shared_secret_encrypted, created_at, last_seen_at, revoked) \
             VALUES (?1, ?2, ?3, 'hash', NULL, '2026-01-01T00:00:00+00:00', ?4, ?5)",
            rusqlite::params![
                channel,
                label,
                pubkey,
                seen.then_some("2026-02-02T00:00:00+00:00"),
                i64::from(revoked)
            ],
        )?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn legacy_devices_are_imported_once_and_the_file_is_left_alone() {
    let (_dir, config) = workspace();
    seed(&config, "chan-a", "Phone", "pk-a", false, true);
    seed(&config, "chan-b", "Tablet", "pk-b", false, false);
    seed(&config, "chan-c", "Old laptop", "pk-c", true, false);
    let db = db_path(&config);
    let before = std::fs::read(&db).unwrap();

    let backend = crate::storage::local::backend_for(&config.workspace_dir).unwrap();
    let reports = run_with(&backend, &config, &[super::super::import_source()]);
    assert!(matches!(
        reports[0].outcome,
        Ok(Outcome::Imported {
            found: 3,
            written: 3
        })
    ));

    let listed = live::list_devices(&config).unwrap();
    let labels: Vec<_> = listed.iter().map(|d| d.label.as_str()).collect();
    assert_eq!(labels, ["Phone", "Tablet"]);
    let phone = live::get_device(&config, "chan-a").unwrap().unwrap();
    assert_eq!(phone.device_pubkey, "pk-a");
    assert!(phone.last_seen_at.is_some(), "last_seen_at survives");
    let tablet = live::get_device(&config, "chan-b").unwrap().unwrap();
    assert!(tablet.last_seen_at.is_none());
    let old = live::get_device(&config, "chan-c").unwrap().unwrap();
    assert!(old.revoked, "a revoked device stays revoked");
    assert_eq!(std::fs::read(&db).unwrap(), before, "devices.db untouched");

    // A second run changes nothing, even after the store moved on.
    live::revoke_device(&config, "chan-b").unwrap();
    let again = run_with(&backend, &config, &[super::super::import_source()]);
    assert!(matches!(again[0].outcome, Ok(Outcome::AlreadyDone)));
    assert_eq!(live::list_devices(&config).unwrap().len(), 1);
}
