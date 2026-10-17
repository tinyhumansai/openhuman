use super::*;
use crate::storage::layout::state::STATE_FILE;
use crate::storage::layout::test_fixtures::{fixture_docs, tree, v1_profile, write, Fixture};

fn opts() -> Options {
    Options::default()
}

async fn go(fx: &Fixture, options: &Options) -> Result<Outcome, LayoutError> {
    migrate(&fx.target, &fx.config, options).await
}

async fn migrated(fx: &Fixture) -> Box<Report> {
    match go(fx, &opts()).await.unwrap() {
        Outcome::Migrated(report) => report,
        other => panic!("expected a migration, got {other:?}"),
    }
}

fn archives(fx: &Fixture) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(fx.target.backups())
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    found.retain(|p| p.to_string_lossy().ends_with(".tar.zst"));
    found
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// The invariants of a finished migration of `v1_profile()`.
fn assert_migrated(fx: &Fixture) {
    let profile = &fx.target.profile_dir;
    let (ws, files) = (fx.workspace(), fx.target.files());
    assert_eq!(
        std::fs::read_to_string(profile.join(".layout"))
            .unwrap()
            .trim(),
        "2"
    );
    assert!(
        !exists(&profile.join(STATE_FILE)),
        "the progress file is removed last"
    );
    // the database came across
    assert_eq!(fixture_docs(&fx.target.storage_db()).len(), 3);
    let jobs = crate::storage::layout::test_fixtures::jobs_value(&fx.target.storage_db());
    assert_eq!(
        jobs,
        Some(serde_json::json!({"jobs": [1, 2, 3]})),
        "imported state is in the v2 database"
    );
    // user files moved
    assert_eq!(
        std::fs::read_to_string(files.join("skills/s1/SKILL.md")).unwrap(),
        "# skill one\n"
    );
    assert!(files.join("attachments/photo.jpg").is_file());
    assert!(files.join("artifacts/a1/plan.md").is_file());
    assert_eq!(
        std::fs::read_to_string(files.join("STYLE.md")).unwrap(),
        "be brief\n"
    );
    assert!(files.join("state/costs.jsonl").is_file());
    assert!(files.join("memory/legacy_claims/c.json").is_file());
    for gone in [
        "skills",
        "attachments",
        "artifacts",
        "STYLE.md",
        "SOUL.md",
        "state",
    ] {
        assert!(!exists(&ws.join(gone)), "{gone} left workspace/");
    }
    // archived originals deleted
    for gone in [
        "storage",
        "memory/jobs.json",
        "memory/conversations",
        "memory",
    ] {
        assert!(!exists(&ws.join(gone)), "{gone} was not deleted");
    }
    // unknown entries untouched
    assert_eq!(
        std::fs::read_to_string(ws.join("mystery/data.bin")).unwrap(),
        "???"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("notes.tmp")).unwrap(),
        "scratch"
    );
    // the archive is whole and holds what was moved and deleted
    let found = archives(fx);
    assert_eq!(found.len(), 1, "{found:?}");
    let manifest = archive::verify(&found[0]).unwrap();
    let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    for want in [
        "workspace/skills/s1/SKILL.md",
        "workspace/STYLE.md",
        "workspace/memory/jobs.json",
        "workspace/memory/conversations/threads.jsonl",
        "workspace/memory/legacy_claims/c.json",
        "workspace/storage/storage.db",
    ] {
        assert!(paths.contains(&want), "{want} not archived: {paths:?}");
    }
    assert!(!paths
        .iter()
        .any(|p| p.contains("mystery") || p.contains("notes.tmp")));
    // the config layer now reads it as v2
    let layout = crate::config::schema::ProfileLayout::resolve_at(profile.clone());
    assert_eq!(
        layout.version,
        crate::config::schema::profile_layout::LayoutVersion::V2
    );
    assert_eq!(layout.workspace_dir, files);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_reports_the_plan_and_writes_nothing() {
    let fx = v1_profile();
    let before = tree(fx.root());
    let Outcome::DryRun(plan) = go(
        &fx,
        &Options {
            dry_run: true,
            ..opts()
        },
    )
    .await
    .unwrap() else {
        panic!("expected a dry run");
    };
    assert_eq!(
        tree(fx.root()),
        before,
        "a dry run writes nothing, not even a lock file"
    );
    let paths =
        |entries: &[plan::Entry]| entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
    assert!(paths(&plan.move_entries).contains(&"skills".to_owned()));
    assert!(paths(&plan.delete_entries).contains(&"memory/jobs.json".to_owned()));
    assert_eq!(
        paths(&plan.unknown_entries),
        vec!["mystery".to_owned(), "notes.tmp".to_owned()]
    );
    assert_eq!(
        plan.archive.files,
        plan.move_totals.files + plan.delete_totals.files
    );
    assert!(plan.archive.bytes > 0 && plan.database_bytes.unwrap() > 0);
    assert_eq!(plan.archive_dir, fx.target.backups());
    assert!(!plan.resuming);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_run_migrates_the_profile() {
    let fx = v1_profile();
    let original = tree(&fx.workspace());
    let report = migrated(&fx).await;
    assert_eq!(report.steps_run, Step::ALL.to_vec());
    assert_eq!(
        report.unknown,
        vec!["mystery".to_owned(), "notes.tmp".to_owned()]
    );
    assert!(
        report.moved.contains(&"skills".to_owned())
            && report.deleted.contains(&"storage".to_owned())
    );
    assert!(report.database_rows.unwrap() > 0 && report.restart_required);
    assert_eq!(report.archive, archives(&fx)[0]);
    assert_migrated(&fx);
    // a profile's workspace never grows files the migration did not account for
    for (path, digest) in &original {
        let in_archive_or_left =
            path.starts_with("mystery") || path == "notes.tmp" || path.starts_with("storage");
        let moved = fx.target.files().join(path);
        if !in_archive_or_left && exists(&moved) {
            assert_eq!(
                &tree(&fx.target.files())[path],
                digest,
                "{path} changed in the move"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_run_after_a_finished_migration_is_a_noop() {
    let fx = v1_profile();
    migrated(&fx).await;
    let before = tree(fx.root());
    assert_eq!(
        go(&fx, &opts()).await.unwrap(),
        Outcome::Noop {
            reason: Noop::AlreadyV2
        }
    );
    assert_eq!(tree(fx.root()), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_at_any_step_resumes_to_the_same_result() {
    for crashed in Step::ALL {
        let fx = v1_profile();
        let error = go(
            &fx,
            &Options {
                crash_after: Some(crashed),
                ..opts()
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, LayoutError::InjectedCrash(s) if s == crashed),
            "{crashed:?}: {error}"
        );
        assert!(
            fx.target.profile_dir.join(STATE_FILE).is_file(),
            "{crashed:?}: a crash leaves the progress file for the resume"
        );
        // dry run during the interruption says it would resume
        if let Outcome::DryRun(plan) = go(
            &fx,
            &Options {
                dry_run: true,
                ..opts()
            },
        )
        .await
        .unwrap()
        {
            assert!(plan.resuming, "{crashed:?}");
        } else {
            panic!("dry run");
        }
        let report = migrated(&fx).await;
        assert_eq!(
            report.steps_run.first(),
            Some(&crashed),
            "{crashed:?}: the unrecorded step re-runs"
        );
        assert_migrated(&fx);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_crashes_still_converge() {
    let fx = v1_profile();
    for crashed in Step::ALL.iter().take(5) {
        let _ = go(
            &fx,
            &Options {
                crash_after: Some(*crashed),
                ..opts()
            },
        )
        .await;
    }
    migrated(&fx).await;
    assert_migrated(&fx);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_migrator_is_refused_while_one_holds_the_lock() {
    let fx = v1_profile();
    let held = Lock::acquire(&fx.target.profile_dir).unwrap();
    let before = tree(&fx.workspace());
    let error = go(&fx, &opts()).await.unwrap_err();
    assert!(matches!(error, LayoutError::Locked(_)), "{error}");
    assert!(error.to_string().starts_with("LAYOUT_LOCKED"));
    assert_eq!(
        tree(&fx.workspace()),
        before,
        "the refused migrator touched nothing"
    );
    // restore takes the same lock
    assert!(matches!(
        restore(&fx.target, Path::new("/nonexistent.tar.zst")).await,
        Err(LayoutError::Locked(_))
    ));
    drop(held);
    migrated(&fx).await;
    assert_migrated(&fx);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_explicit_url_runs_only_the_schema_migrations() {
    let fx = v1_profile();
    let before = tree(fx.root());
    let memory = Options {
        env_url: Some("memory".to_owned()),
        ..opts()
    };
    assert_eq!(
        go(&fx, &memory).await.unwrap(),
        Outcome::Noop {
            reason: Noop::ExplicitUrl
        }
    );
    assert_eq!(
        tree(fx.root()),
        before,
        "no layout change for an explicit backend"
    );

    // MongoDB: a dry run must not even try to connect
    let mongo = Options {
        env_url: Some("mongodb://u:p@db.invalid/oh".to_owned()),
        dry_run: true,
        ..opts()
    };
    assert_eq!(
        go(&fx, &mongo).await.unwrap(),
        Outcome::Noop {
            reason: Noop::ExplicitUrl
        }
    );

    // `[storage] url` counts the same as the environment variable
    let mut config = fx.config.clone();
    config.storage.url = Some("memory".to_owned());
    let out = migrate(&fx.target, &config, &opts()).await.unwrap();
    assert_eq!(
        out,
        Outcome::Noop {
            reason: Noop::ExplicitUrl
        }
    );
    assert_eq!(tree(fx.root()), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_or_marked_profile_is_already_v2() {
    let dir = tempfile::TempDir::new().unwrap();
    let fx = Fixture {
        target: Target::new(dir.path(), "fresh"),
        config: Config::default(),
        dir,
    };
    assert_eq!(
        go(&fx, &opts()).await.unwrap(),
        Outcome::Noop {
            reason: Noop::AlreadyV2
        }
    );
    assert!(!fx.target.profile_dir.exists(), "a no-op creates nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moved_entry_never_overwrites_a_file_already_in_files() {
    let fx = v1_profile();
    write(&fx.target.files().join("STYLE.md"), "already here");
    let error = go(&fx, &opts()).await.unwrap_err();
    assert!(matches!(error, LayoutError::Conflict(_)), "{error}");
    assert_eq!(
        std::fs::read_to_string(fx.target.files().join("STYLE.md")).unwrap(),
        "already here"
    );
    assert_eq!(
        std::fs::read_to_string(fx.workspace().join("STYLE.md")).unwrap(),
        "be brief\n"
    );
    assert!(
        fx.workspace().join("memory/jobs.json").is_file(),
        "nothing was deleted"
    );
    assert!(!fx.target.profile_dir.join(".layout").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn files_merge_into_an_existing_files_directory() {
    let fx = v1_profile();
    write(&fx.target.files().join("skills/other/SKILL.md"), "mine");
    migrated(&fx).await;
    assert!(fx.target.files().join("skills/other/SKILL.md").is_file());
    assert!(fx.target.files().join("skills/s1/SKILL.md").is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_returns_the_profile_to_layout_v1() {
    let fx = v1_profile();
    let original = tree(&fx.workspace());
    let report = migrated(&fx).await;

    let restored = restore(&fx.target, &report.archive).await.unwrap();
    assert!(restored.restart_required && restored.files > 0);
    assert!(
        restored.v2_database.as_ref().unwrap().is_file(),
        "the v2 database is set aside, not deleted"
    );
    assert!(!fx.target.storage_db().exists());
    assert!(!fx.target.profile_dir.join(".layout").exists());
    assert_eq!(
        crate::config::schema::profile_layout::LayoutVersion::detect(&fx.target.profile_dir),
        crate::config::schema::profile_layout::LayoutVersion::V1
    );
    let after = tree(&fx.workspace());
    for (path, digest) in &original {
        if path.starts_with("storage") {
            continue; // the database file is the checkpointed copy
        }
        assert_eq!(after.get(path), Some(digest), "{path} did not round-trip");
    }
    assert!(fx.workspace().join("storage/storage.db").is_file());
    for moved in ["skills", "STYLE.md", "state"] {
        assert!(
            !exists(&fx.target.files().join(moved)),
            "{moved} still in files/"
        );
    }
    // and the v1 database holds the data again
    assert_eq!(fixture_docs(&legacy::v1_db(&fx.target)).len(), 3);
    // the profile can be migrated again
    let again = migrated_again(&fx).await;
    assert!(again);
}

async fn migrated_again(fx: &Fixture) -> bool {
    // a second archive gets its own name: wait out the timestamp's resolution
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    matches!(go(fx, &opts()).await.unwrap(), Outcome::Migrated(_))
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_refuses_a_corrupt_archive_and_a_migration_in_progress() {
    let fx = v1_profile();
    let report = migrated(&fx).await;
    let mut bytes = std::fs::read(&report.archive).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    let bad = report.archive.with_file_name("bad.tar.zst");
    std::fs::write(&bad, bytes).unwrap();
    let before = tree(&fx.target.profile_dir);
    assert!(restore(&fx.target, &bad).await.is_err());
    assert_eq!(
        tree(&fx.target.profile_dir),
        before,
        "a failed restore changes nothing"
    );

    let busy = v1_profile();
    let _ = go(
        &busy,
        &Options {
            crash_after: Some(Step::Import),
            ..opts()
        },
    )
    .await;
    let error = restore(&busy.target, &report.archive)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("in progress"), "{error}");
}

#[test]
fn a_live_core_on_the_v1_database_is_detected() {
    let target = Target::new(Path::new("/r"), "u1");
    let url = legacy::v1_url(&target);
    assert_eq!(
        live_conflict(&target, Some(url)),
        Some("sqlite:/r/users/u1/workspace/storage".to_owned())
    );
    assert_eq!(
        live_conflict(&target, Some("sqlite:/elsewhere".to_owned())),
        None
    );
    assert_eq!(live_conflict(&target, None), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_migrate_is_off_by_default_and_runs_when_asked() {
    let fx = v1_profile();
    assert!(!fx.config.storage.auto_migrate_layout);
    assert!(!auto_migrate(&fx.config).await);
    assert!(!fx.target.profile_dir.join(".layout").exists());
    let mut on = fx.config.clone();
    on.storage.auto_migrate_layout = true;
    assert!(auto_migrate(&on).await);
    assert_migrated(&fx);
    assert!(!auto_migrate(&on).await, "already v2");
}

#[test]
fn deleting_refuses_a_path_that_climbs_out_of_its_base() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("files");
    std::fs::create_dir_all(&base).unwrap();
    let outside = dir.path().join("keep.txt");
    std::fs::write(&outside, b"keep").unwrap();

    let climbing = plan::join(&base, "../keep.txt");
    assert!(matches!(
        remove_within(&base, &climbing),
        Err(LayoutError::Verify(_))
    ));
    assert!(
        outside.is_file(),
        "a `..` entry must not delete outside the base"
    );
}
