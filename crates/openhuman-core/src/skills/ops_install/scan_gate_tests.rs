use super::*;

use super::super::fetch::InstallWorkflowFromUrlOutcome;
use tinyskills::{ScanCheck, Verdict};

#[test]
fn host_outcome_serializes_with_a_status_tag() {
    let installed = SkillInstallOutcome::Installed(InstallWorkflowFromUrlOutcome {
        url: "u".into(),
        stdout: "Installed to /x".into(),
        stderr: String::new(),
        new_skills: vec!["x".into()],
    });
    let value = serde_json::to_value(&installed).unwrap();
    assert_eq!(value["status"], "installed");
    assert_eq!(value["new_skills"][0], "x");

    let blocked = SkillInstallOutcome::ScanBlocked(ScanBlockedOutcome {
        target: "x".into(),
        fetched_from: "https://example.com/SKILL.md".into(),
        slug: "x".into(),
        digest: "d".into(),
        findings: vec![ScanFindingSummary {
            check: ScanCheck::InvisibleCodePoints,
            verdict: Verdict::Block,
            field: "the document body".into(),
            message: "an invisible character in the document body".into(),
        }],
        message: "blocked".into(),
    });
    let value = serde_json::to_value(&blocked).unwrap();
    assert_eq!(value["status"], "scan_blocked");
    assert_eq!(value["digest"], "d");
}

const BLOCKED_SKILL: &str =
    "---\nname: url-poisoned\ndescription: A pasted skill.\n---\n\n# Steps\nRun\u{200b} it.\n";
const OTHER_BLOCKED_SKILL: &str =
    "---\nname: url-poisoned\ndescription: A pasted skill.\n---\n\n# Steps\nRun\u{200b} it twice.\n";
const CLEAN_SKILL: &str =
    "---\nname: url-poisoned\ndescription: A pasted skill.\n---\n\n# Steps\nRun it.\n";

async fn serve(responses: &[(&'static str, u64)]) -> wiremock::MockServer {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    for (body, times) in responses {
        Mock::given(method("GET"))
            .and(path("/SKILL.md"))
            .respond_with(ResponseTemplate::new(200).set_body_string(*body))
            .up_to_n_times(*times)
            .expect(*times)
            .mount(&server)
            .await;
    }
    server
}

async fn install_url(
    server: &wiremock::MockServer,
    home: &std::path::Path,
    acknowledgement: &ScanAcknowledgement,
) -> SkillInstallOutcome {
    crate::skills::ops_install::install_workflow_from_url_with_home(
        home,
        crate::skills::ops_install::InstallWorkflowFromUrlParams {
            url: format!("{}/SKILL.md", server.uri()),
            timeout_secs: Some(5),
        },
        Some(home),
        true,
        acknowledgement.clone(),
    )
    .await
    .expect("install")
}

fn url_skill(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".openhuman/skills/url-poisoned/SKILL.md")
}

#[tokio::test]
async fn a_pasted_url_whose_scan_blocks_is_retried_and_refused() {
    let server = serve(&[(BLOCKED_SKILL, 2)]).await;
    let home = tempfile::tempdir().unwrap();

    let outcome = install_url(&server, home.path(), &ScanAcknowledgement::Absent).await;

    let SkillInstallOutcome::ScanBlocked(blocked) = outcome else {
        panic!("expected scan_blocked, got {outcome:?}");
    };
    assert!(blocked.target.ends_with("/SKILL.md"), "{}", blocked.target);
    assert_eq!(blocked.slug, "url-poisoned");
    assert_eq!(blocked.findings[0].check, ScanCheck::InvisibleCodePoints);
    assert!(!url_skill(home.path()).exists());
    server.verify().await;
}

#[tokio::test]
async fn a_pasted_url_installs_when_the_retry_scans_clean() {
    let server = serve(&[(BLOCKED_SKILL, 1), (CLEAN_SKILL, 1)]).await;
    let home = tempfile::tempdir().unwrap();

    let outcome = install_url(&server, home.path(), &ScanAcknowledgement::Absent).await;

    assert_eq!(outcome.status(), "installed");
    let written = std::fs::read_to_string(url_skill(home.path())).unwrap();
    assert!(!written.contains('\u{200b}'));
    server.verify().await;
}

fn blocked_digest(outcome: SkillInstallOutcome) -> String {
    match outcome {
        SkillInstallOutcome::ScanBlocked(blocked) => {
            assert!(!blocked.digest.is_empty());
            blocked.digest
        }
        other => panic!("expected scan_blocked, got {other:?}"),
    }
}

fn acknowledge(digest: &str) -> ScanAcknowledgement {
    ScanAcknowledgement::ByUser {
        digest: digest.to_owned(),
    }
}

#[tokio::test]
async fn an_acknowledged_pasted_url_installs_the_document_the_user_saw() {
    let server = serve(&[(BLOCKED_SKILL, 3)]).await;
    let home = tempfile::tempdir().unwrap();

    let digest =
        blocked_digest(install_url(&server, home.path(), &ScanAcknowledgement::Absent).await);
    let outcome = install_url(&server, home.path(), &acknowledge(&digest)).await;

    assert_eq!(outcome.status(), "installed");
    assert!(std::fs::read_to_string(url_skill(home.path()))
        .unwrap()
        .contains('\u{200b}'));
    server.verify().await;
}

#[tokio::test]
async fn a_changed_document_is_rescanned_and_refused_under_a_stale_acknowledgement() {
    let server = serve(&[(BLOCKED_SKILL, 2), (OTHER_BLOCKED_SKILL, 2)]).await;
    let home = tempfile::tempdir().unwrap();

    let seen =
        blocked_digest(install_url(&server, home.path(), &ScanAcknowledgement::Absent).await);
    let fresh = blocked_digest(install_url(&server, home.path(), &acknowledge(&seen)).await);

    assert_ne!(fresh, seen, "the refusal carries the new document's digest");
    assert!(!url_skill(home.path()).exists());
    server.verify().await;
}

#[tokio::test]
async fn a_changed_document_that_scans_clean_installs_under_a_stale_acknowledgement() {
    let server = serve(&[(BLOCKED_SKILL, 2), (CLEAN_SKILL, 1)]).await;
    let home = tempfile::tempdir().unwrap();

    let seen =
        blocked_digest(install_url(&server, home.path(), &ScanAcknowledgement::Absent).await);
    let outcome = install_url(&server, home.path(), &acknowledge(&seen)).await;

    assert_eq!(outcome.status(), "installed");
    assert!(!std::fs::read_to_string(url_skill(home.path()))
        .unwrap()
        .contains('\u{200b}'));
    server.verify().await;
}
