use super::*;

use std::sync::atomic::Ordering;

use serde_json::json;

use crate::skills::catalog::test_fixtures::{hermes_item, Fixture};

#[tokio::test]
async fn installs_a_catalog_entry_once_and_announces_it() {
    use crate::core::events::DomainEvent;
    use tinybus::TryRecvError;

    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS
        .get()
        .expect("event bus should be initialized")
        .receiver();

    let fixture = Fixture::start(vec![hermes_item("zz-registry-install", "built-in")]).await;
    let registry = fixture.registry();
    let workspace = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    let first = install_from_catalog_in(
        &registry,
        workspace.path(),
        Some(home.path()),
        "zz-registry-install",
        ScanAcknowledgement::Absent,
    )
    .await
    .expect("install")
    .installed()
    .expect("a clean document installs");
    assert_eq!(first.new_skills, ["zz-registry-install"]);
    assert!(first.stdout.contains("Installed to"), "{}", first.stdout);
    assert!(home
        .path()
        .join(".openhuman/skills/zz-registry-install/SKILL.md")
        .exists());

    let mut announced = false;
    loop {
        match rx.try_recv() {
            Ok(DomainEvent::WorkflowsChanged { reason }) if reason == "install" => {
                announced = true;
                break;
            }
            Ok(_) | Err(TryRecvError::Lagged(_)) => continue,
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
        }
    }
    assert!(announced, "a catalog install publishes WorkflowsChanged");

    let second = install_from_catalog_in(
        &registry,
        workspace.path(),
        Some(home.path()),
        "zz-registry-install",
        ScanAcknowledgement::Absent,
    )
    .await
    .expect("a repeat install succeeds")
    .installed()
    .expect("a clean document installs");
    assert!(second.new_skills.is_empty());
    assert!(
        second.stdout.contains("already installed"),
        "{}",
        second.stdout
    );
}

#[tokio::test]
async fn a_portal_entry_fails_fast_with_its_source_page() {
    let fixture = Fixture::start(vec![json!({
        "name": "code-audit",
        "description": "x",
        "category": "other",
        "source": "LobeHub",
        "identifier": "lobehub/code-audit",
        "sourceUrl": "https://lobehub.com/agent/code-audit"
    })])
    .await;
    let home = tempfile::tempdir().unwrap();
    let error = install_from_catalog_in(
        &fixture.registry_without_download_base(),
        home.path(),
        Some(home.path()),
        "lobehub/code-audit",
        ScanAcknowledgement::Absent,
    )
    .await
    .expect_err("no SKILL.md to fetch");
    assert_eq!(error.kind(), Some(RegistryErrorKind::NoDirectDownload));
    let message = error.to_string();
    assert!(
        message.starts_with("SKILL_REGISTRY_NO_DIRECT_DOWNLOAD: "),
        "{message}"
    );
    assert!(
        message.contains("https://lobehub.com/agent/code-audit"),
        "{message}"
    );
}

#[tokio::test]
async fn an_unknown_id_names_real_ids() {
    let fixture = Fixture::start(vec![hermes_item("git-helper", "built-in")]).await;
    let home = tempfile::tempdir().unwrap();
    let error = install_from_catalog_in(
        &fixture.registry(),
        home.path(),
        Some(home.path()),
        "git-helpr",
        ScanAcknowledgement::Absent,
    )
    .await
    .expect_err("unknown id");
    assert_eq!(error.kind(), Some(RegistryErrorKind::NotFound));
    assert!(error.to_string().contains("git-helper"), "{error}");
}

#[tokio::test]
async fn a_throttled_document_host_reports_rate_limiting() {
    let fixture = Fixture::start(vec![hermes_item("slow-skill", "built-in")]).await;
    fixture.document_status.store(429, Ordering::SeqCst);
    let home = tempfile::tempdir().unwrap();
    let error = install_from_catalog_in(
        &fixture.registry(),
        home.path(),
        Some(home.path()),
        "slow-skill",
        ScanAcknowledgement::Absent,
    )
    .await
    .expect_err("throttled");
    assert_eq!(error.kind(), Some(RegistryErrorKind::RateLimited));
    let message = error.to_string();
    assert!(
        message.starts_with("SKILL_REGISTRY_RATE_LIMITED: rate limited"),
        "{message}"
    );
    assert!(message.contains("42s"), "{message}");
    assert_eq!(
        fixture.document_hits.load(Ordering::SeqCst),
        1,
        "a throttled host is not hit again at once"
    );
}

fn installed_skill(home: &std::path::Path, slug: &str) -> std::path::PathBuf {
    home.join(".openhuman/skills").join(slug).join("SKILL.md")
}

#[tokio::test]
async fn a_scan_block_is_retried_once_and_a_clean_retry_installs() {
    let fixture = Fixture::start(vec![hermes_item("flaky-scan", "built-in")]).await;
    fixture.blocked_documents.store(1, Ordering::SeqCst);
    let home = tempfile::tempdir().unwrap();

    let outcome = install_from_catalog_in(
        &fixture.registry(),
        home.path(),
        Some(home.path()),
        "flaky-scan",
        ScanAcknowledgement::Absent,
    )
    .await
    .expect("install");

    assert_eq!(outcome.status(), "installed");
    assert_eq!(fixture.document_hits.load(Ordering::SeqCst), 2);
    let written = std::fs::read_to_string(installed_skill(home.path(), "flaky-scan")).unwrap();
    assert!(
        !written.contains('\u{200b}'),
        "the clean retry is what lands"
    );
}

#[tokio::test]
async fn a_document_that_still_blocks_is_not_installed() {
    let fixture = Fixture::start(vec![hermes_item("poisoned", "built-in")]).await;
    fixture
        .blocked_documents
        .store(usize::MAX, Ordering::SeqCst);
    let home = tempfile::tempdir().unwrap();

    let outcome = install_from_catalog_in(
        &fixture.registry(),
        home.path(),
        Some(home.path()),
        "poisoned",
        ScanAcknowledgement::Absent,
    )
    .await
    .expect("a scan block is an outcome, not an error");

    let SkillInstallOutcome::ScanBlocked(blocked) = outcome else {
        panic!("expected scan_blocked, got {outcome:?}");
    };
    assert_eq!(fixture.document_hits.load(Ordering::SeqCst), 2, "one retry");
    assert_eq!(blocked.target, "poisoned");
    assert_eq!(blocked.slug, "poisoned");
    assert!(blocked.findings.iter().any(|finding| finding.check
        == tinyskills::ScanCheck::InvisibleCodePoints
        && finding.verdict == tinyskills::Verdict::Block));
    assert!(
        blocked.message.contains("not installed"),
        "{}",
        blocked.message
    );
    assert!(!installed_skill(home.path(), "poisoned").exists());
}

async fn install_entry(
    registry: &SkillRegistry,
    home: &std::path::Path,
    entry_id: &str,
    acknowledgement: ScanAcknowledgement,
) -> SkillInstallOutcome {
    install_from_catalog_in(registry, home, Some(home), entry_id, acknowledgement)
        .await
        .expect("a scan block is an outcome, not an error")
}

fn blocked(outcome: SkillInstallOutcome) -> crate::skills::ops_install::ScanBlockedOutcome {
    match outcome {
        SkillInstallOutcome::ScanBlocked(blocked) => blocked,
        other => panic!("expected scan_blocked, got {other:?}"),
    }
}

fn by_user(digest: &str) -> ScanAcknowledgement {
    ScanAcknowledgement::ByUser {
        digest: digest.to_owned(),
    }
}

#[tokio::test]
async fn an_acknowledged_install_writes_the_blocked_document_the_user_saw() {
    let fixture = Fixture::start(vec![hermes_item("acknowledged", "built-in")]).await;
    fixture
        .blocked_documents
        .store(usize::MAX, Ordering::SeqCst);
    let registry = fixture.registry();
    let home = tempfile::tempdir().unwrap();

    let seen = blocked(
        install_entry(
            &registry,
            home.path(),
            "acknowledged",
            ScanAcknowledgement::Absent,
        )
        .await,
    );
    assert_eq!(fixture.document_hits.load(Ordering::SeqCst), 2);

    let outcome = install_entry(
        &registry,
        home.path(),
        "acknowledged",
        by_user(&seen.digest),
    )
    .await
    .installed()
    .expect("the user acknowledged this document");

    assert_eq!(outcome.new_skills, ["acknowledged"]);
    assert_eq!(
        fixture.document_hits.load(Ordering::SeqCst),
        3,
        "a matching acknowledged document installs without a refetch"
    );
    let written = std::fs::read_to_string(installed_skill(home.path(), "acknowledged")).unwrap();
    assert!(written.contains('\u{200b}'));
}

#[tokio::test]
async fn a_stale_acknowledgement_does_not_install_a_changed_document() {
    let fixture = Fixture::start(vec![hermes_item("swapped", "built-in")]).await;
    fixture
        .blocked_documents
        .store(usize::MAX, Ordering::SeqCst);
    let registry = fixture.registry();
    let home = tempfile::tempdir().unwrap();

    let seen = blocked(
        install_entry(
            &registry,
            home.path(),
            "swapped",
            ScanAcknowledgement::Absent,
        )
        .await,
    );
    fixture.blocked_variant.store(1, Ordering::SeqCst);

    let fresh =
        blocked(install_entry(&registry, home.path(), "swapped", by_user(&seen.digest)).await);

    assert_ne!(
        fresh.digest, seen.digest,
        "the refusal names the new document"
    );
    assert!(fresh
        .findings
        .iter()
        .any(|finding| finding.verdict == tinyskills::Verdict::Block));
    assert_eq!(
        fixture.document_hits.load(Ordering::SeqCst),
        4,
        "the changed document is scanned twice like any unacknowledged one"
    );
    assert!(!installed_skill(home.path(), "swapped").exists());

    let installed = install_entry(&registry, home.path(), "swapped", by_user(&fresh.digest)).await;
    assert_eq!(installed.status(), "installed");
}

#[tokio::test]
async fn a_stale_acknowledgement_installs_a_changed_document_that_scans_clean() {
    let fixture = Fixture::start(vec![hermes_item("cleaned", "built-in")]).await;
    fixture.blocked_documents.store(2, Ordering::SeqCst);
    let registry = fixture.registry();
    let home = tempfile::tempdir().unwrap();

    let seen = blocked(
        install_entry(
            &registry,
            home.path(),
            "cleaned",
            ScanAcknowledgement::Absent,
        )
        .await,
    );
    let outcome = install_entry(&registry, home.path(), "cleaned", by_user(&seen.digest)).await;

    assert_eq!(outcome.status(), "installed");
    let written = std::fs::read_to_string(installed_skill(home.path(), "cleaned")).unwrap();
    assert!(!written.contains('\u{200b}'));
}

#[tokio::test]
async fn a_failed_document_fetch_is_retried_once() {
    let fixture = Fixture::start(vec![hermes_item("down", "built-in")]).await;
    fixture.document_status.store(503, Ordering::SeqCst);
    let home = tempfile::tempdir().unwrap();

    let error = install_from_catalog_in(
        &fixture.registry(),
        home.path(),
        Some(home.path()),
        "down",
        ScanAcknowledgement::Absent,
    )
    .await
    .expect_err("the host stays down");

    assert_eq!(error.kind(), Some(RegistryErrorKind::Unavailable));
    assert_eq!(fixture.document_hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_install_retry_does_not_refetch_a_catalog_in_cooldown() {
    let fixture = Fixture::start(vec![hermes_item("cold", "built-in")]).await;
    fixture.catalog_status.store(503, Ordering::SeqCst);
    let home = tempfile::tempdir().unwrap();

    let error = install_from_catalog_in(
        &fixture.registry(),
        home.path(),
        Some(home.path()),
        "cold",
        ScanAcknowledgement::Absent,
    )
    .await
    .expect_err("no catalog to locate the entry in");

    assert_eq!(error.kind(), Some(RegistryErrorKind::Unavailable));
    assert_eq!(
        fixture.catalog_hits.load(Ordering::SeqCst),
        1,
        "the retry answers from the cooldown instead of refetching"
    );
    assert_eq!(fixture.document_hits.load(Ordering::SeqCst), 0);
}
