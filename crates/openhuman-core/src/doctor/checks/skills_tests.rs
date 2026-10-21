use super::*;
use crate::core::runtime::{CoreContext, DomainSet};
use crate::core::slots::Extensions;
use crate::doctor::contract::FindingSeverity;
use crate::skills::preflight::PreflightProbes;
use std::path::Path;

fn config_in(root: &Path) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.workspace_dir = root.join("workspace");
    config.action_dir = root.join("actions");
    config.config_path = root.join("config.toml");
    config
}

fn scoped(config: crate::config::Config) -> Arc<CoreContext> {
    CoreContext::for_test_with_extensions(DomainSet::full(), None, Extensions::new()).derive_with(
        crate::core::runtime::ContextOverlay::new(config, DomainSet::full(), Default::default())
            .without_user_skill_roots(),
    )
}

#[tokio::test]
async fn github_preflight_is_not_registered_when_no_skill_requires_it() {
    let scratch = tempfile::tempdir().expect("create isolated skill fixture");
    let config = config_in(scratch.path());
    let skill_dir = config.workspace_dir.join("skills/plain-skill");
    std::fs::create_dir_all(&skill_dir).expect("create skill directory");
    std::fs::write(skill_dir.join("SKILL.md"), "A plain skill.\n").expect("write skill document");

    let context = scoped(config.clone());
    let checks = CoreContext::scope(context, async {
        for_context(&DoctorCtx::from_config(config.clone()))
    })
    .await;

    assert!(checks.iter().any(|check| check.id() == "skills/discovery"));
    assert!(checks
        .iter()
        .all(|check| check.id() != "skills/github-preflight"));
}

#[derive(Default)]
struct MissingGithubProbe;

#[async_trait]
impl PreflightProbes for MissingGithubProbe {
    async fn composio_toolkit_active(&self, _toolkit: &str) -> bool {
        false
    }

    async fn composio_identity(&self, _toolkit: &str) -> Option<String> {
        None
    }

    async fn git_version(&self) -> Result<(), String> {
        Ok(())
    }

    async fn git_user_name(&self) -> String {
        "private local name".to_owned()
    }

    async fn git_user_email(&self) -> String {
        "private@example.test".to_owned()
    }
}

#[tokio::test]
async fn required_github_preflight_uses_fake_probes_and_redacts_error_details() {
    let scratch = tempfile::tempdir().expect("create isolated skill fixture");
    let config = config_in(scratch.path());
    let skill_dir = config.workspace_dir.join("skills/required-skill");
    std::fs::create_dir_all(&skill_dir).expect("create skill directory");
    std::fs::write(skill_dir.join("WORKFLOW.md"), "A workflow.\n")
        .expect("write workflow document");
    let manifest = skill_dir.join("workflow.toml");
    std::fs::write(
        &manifest,
        "id = \"required-skill\"\nwhen_to_use = \"Use the GitHub API\"\n\
         [github]\nrequired = true\nidentity_match = \"strict\"\n",
    )
    .expect("write GitHub gate config");

    let context = scoped(config.clone());
    let (required, checks) = CoreContext::scope(context, async {
        (
            required_github_preflight(&config),
            for_context(&DoctorCtx::from_config(config.clone())),
        )
    })
    .await;
    let required = required.expect("the workflow explicitly requires GitHub");
    assert_eq!(required.path, manifest);
    assert!(checks
        .iter()
        .any(|check| check.id() == "skills/github-preflight"));

    let check = GithubPreflightCheck {
        required: required.config,
        manifest_path: required.path,
        probes: Some(Arc::new(MissingGithubProbe)),
    };
    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .remove(0);

    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(finding
        .message
        .contains("GitHub integration is not connected"));
    assert!(!finding.message.contains("private"));
    assert!(!finding.message.contains("example.test"));
    assert_eq!(
        finding.config_path.as_deref(),
        Some(manifest.to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn production_github_preflight_skips_expired_profile_without_mutating_credentials_or_loading_connectors(
) {
    let scratch = tempfile::tempdir().expect("create isolated preflight fixture");
    let config = config_in(scratch.path());
    let _env = crate::config::test_env::EnvVarGuard::locked_async()
        .await
        .with("HOME", scratch.path())
        .with("OPENHUMAN_WORKSPACE", config.workspace_dir.as_os_str())
        .without("OPENHUMAN_ACTION_DIR")
        .without("COMPOSIO_API_KEY")
        .with("OPENHUMAN_KEYRING_BACKEND", "file");

    let skill_dir = config.workspace_dir.join("skills/required-skill");
    std::fs::create_dir_all(&skill_dir).expect("create isolated skill directory");
    std::fs::write(
        skill_dir.join("WORKFLOW.md"),
        "A required GitHub workflow.\n",
    )
    .expect("write valid workflow document");
    let manifest = skill_dir.join("workflow.toml");
    std::fs::write(
        &manifest,
        "id = \"required-skill\"\nwhen_to_use = \"Use GitHub\"\n\
         [github]\nrequired = true\nidentity_match = \"none\"\n",
    )
    .expect("write required GitHub manifest");
    let profiles = scratch.path().join("auth-profiles.json");
    let raw_profile = serde_json::json!({
        "schema_version": 1,
        "active_profiles": {},
        "profiles": {
            "app-session:default": {
                "provider": "app-session",
                "profile_name": "default",
                "kind": "token",
                "token": "expired-doctor-test-secret",
                "metadata": { "session_expires_at": "2000-01-01T00:00:00Z" }
            }
        }
    })
    .to_string();
    std::fs::write(&profiles, &raw_profile).expect("seed isolated expired profile");

    let loaded =
        crate::security::credentials::session_support::load_app_session_profile_read_only(&config)
            .expect("read-only profile fixture should parse");
    assert!(matches!(
        crate::security::credentials::session_support::classify_session_token(
            loaded.as_ref(),
            chrono::Utc::now()
        ),
        crate::security::credentials::session_support::SessionTokenCheck::Expired
    ));
    let connector_state_before = crate::modules::ops::state_of("tinyconnectors");

    let context = scoped(config.clone());
    let findings = CoreContext::scope(context, async {
        let required = required_github_preflight(&config).expect("manifest requires GitHub");
        GithubPreflightCheck {
            required: required.config,
            manifest_path: required.path,
            probes: None,
        }
        .detect(&DoctorCtx::from_config(config.clone()))
        .await
    })
    .await;

    assert_eq!(std::fs::read_to_string(&profiles).unwrap(), raw_profile);
    assert!(!scratch.path().join("auth-profiles.lock").exists());
    assert_eq!(
        crate::modules::ops::state_of("tinyconnectors"),
        connector_state_before,
        "doctor must not start or otherwise transition the connector module"
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, FindingSeverity::Skipped);
    assert!(findings[0].message.contains("unavailable"));
    assert!(!scratch.path().join("modules").exists());
    assert!(!findings[0].message.contains("expired-doctor-test-secret"));
}

#[tokio::test]
async fn production_github_preflight_skips_malformed_profile_without_quarantining_or_loading_connectors(
) {
    let scratch = tempfile::tempdir().expect("create isolated preflight fixture");
    let config = config_in(scratch.path());
    let _env = crate::config::test_env::EnvVarGuard::locked_async()
        .await
        .with("HOME", scratch.path())
        .with("OPENHUMAN_WORKSPACE", config.workspace_dir.as_os_str())
        .without("OPENHUMAN_ACTION_DIR")
        .without("COMPOSIO_API_KEY")
        .with("OPENHUMAN_KEYRING_BACKEND", "file");

    let skill_dir = config.workspace_dir.join("skills/required-skill");
    std::fs::create_dir_all(&skill_dir).expect("create isolated skill directory");
    std::fs::write(
        skill_dir.join("WORKFLOW.md"),
        "A required GitHub workflow.\n",
    )
    .expect("write workflow document");
    let manifest = skill_dir.join("workflow.toml");
    std::fs::write(
        &manifest,
        "id = \"required-skill\"\nwhen_to_use = \"Use GitHub\"\n\
         [github]\nrequired = true\nidentity_match = \"none\"\n",
    )
    .expect("write required GitHub manifest");
    let profiles = scratch.path().join("auth-profiles.json");
    let malformed = b"{ malformed profile store";
    std::fs::write(&profiles, malformed).expect("seed malformed isolated profile store");

    let context = scoped(config.clone());
    let findings = CoreContext::scope(context, async {
        let required = required_github_preflight(&config).expect("manifest requires GitHub");
        GithubPreflightCheck {
            required: required.config,
            manifest_path: required.path,
            probes: None,
        }
        .detect(&DoctorCtx::from_config(config.clone()))
        .await
    })
    .await;

    assert_eq!(std::fs::read(&profiles).unwrap(), malformed);
    assert!(!scratch.path().join("auth-profiles.lock").exists());
    assert!(!scratch
        .path()
        .read_dir()
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains("corrupt")));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, FindingSeverity::Skipped);
    assert!(findings[0].message.contains("unavailable"));
    assert!(!scratch.path().join("modules").exists());
}

#[tokio::test]
async fn malformed_skill_manifest_reports_only_a_warning_and_its_path() {
    let scratch = tempfile::tempdir().expect("create isolated skill fixture");
    let config = config_in(scratch.path());
    let skill_dir = config.workspace_dir.join("skills/broken-skill");
    std::fs::create_dir_all(&skill_dir).expect("create skill directory");
    let manifest = skill_dir.join("SKILL.md");
    std::fs::write(&manifest, "---\nname: [broken\n---\nA test skill.\n")
        .expect("write malformed skill manifest");

    let context = scoped(config.clone());
    let findings = CoreContext::scope(context, async {
        SkillDiscoveryCheck
            .detect(&DoctorCtx::from_config(config))
            .await
    })
    .await;

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, FindingSeverity::Warn);
    assert_eq!(
        findings[0].config_path.as_deref(),
        Some(manifest.to_string_lossy().as_ref())
    );
    assert!(!findings[0].message.contains("broken"));
}
