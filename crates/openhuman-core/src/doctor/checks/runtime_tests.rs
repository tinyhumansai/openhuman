use super::*;

use crate::doctor::{DoctorCheck, DoctorCtx, FindingSeverity};

#[cfg(unix)]
#[test]
fn environment_check_resolves_git_and_curl_from_path_without_running_them() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = tempfile::tempdir().expect("environment fixture");
    let bin = fixture.path().join("bin");
    std::fs::create_dir(&bin).expect("create fake bin directory");
    let marker = fixture.path().join("ran");
    for name in ["git", "curl"] {
        let path = bin.join(name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\nprintf x > '{}'\n", marker.display()),
        )
        .expect("write executable fixture that would leave a marker if run");
        let mut permissions = std::fs::metadata(&path)
            .expect("executable metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).expect("mark fixture executable");
    }

    let findings = environment_findings(
        Some(bin.as_os_str()),
        Some(std::ffi::OsStr::new("/bin/sh")),
        Some(std::ffi::OsStr::new("/isolated/home")),
        None,
    );

    assert!(findings.iter().any(|finding| {
        finding.check_id == "runtime/environment"
            && finding.severity == FindingSeverity::Pass
            && finding.config_path.as_deref() == Some("PATH/git")
    }));
    assert!(findings.iter().any(|finding| {
        finding.check_id == "runtime/environment"
            && finding.severity == FindingSeverity::Pass
            && finding.config_path.as_deref() == Some("PATH/curl")
    }));
    assert!(findings
        .iter()
        .all(|finding| !finding.message.contains("/isolated/home")));
    assert!(
        !marker.exists(),
        "static environment checks must not spawn tools"
    );
}

#[test]
fn environment_check_reports_missing_prerequisites_without_echoing_values() {
    let findings = environment_findings(None, None, None, None);

    assert!(findings.iter().any(|finding| {
        finding.config_path.as_deref() == Some("PATH/git")
            && finding.severity == FindingSeverity::Warn
    }));
    assert!(findings.iter().any(|finding| {
        finding.config_path.as_deref() == Some("PATH/curl")
            && finding.severity == FindingSeverity::Warn
    }));
    assert!(findings.iter().any(|finding| {
        finding.config_path.as_deref() == Some("SHELL") && finding.severity == FindingSeverity::Warn
    }));
    assert!(findings.iter().any(|finding| {
        finding.config_path.as_deref() == Some("HOME") && finding.severity == FindingSeverity::Error
    }));
    assert!(
        findings
            .iter()
            .all(|finding| !finding.message.contains("/Users/")
                && !finding.message.contains("secret"))
    );
}

#[cfg(unix)]
fn fake_cli(root: &std::path::Path, script: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = root.join("claude-agent-sdk");
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).expect("write isolated fake CLI");
    let mut permissions = std::fs::metadata(&path)
        .expect("fake CLI metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).expect("mark fake CLI executable");
    path
}

#[cfg(unix)]
fn sdk_context(binary: std::path::PathBuf) -> DoctorCtx {
    let mut config = crate::config::Config::default();
    config.claude_agent_sdk.enabled = true;
    config.claude_agent_sdk.binary = binary.to_string_lossy().into_owned();
    DoctorCtx::from_config(config)
}

#[cfg(unix)]
#[tokio::test]
async fn agent_sdk_probe_reports_availability_without_echoing_command_output() {
    let fixture = tempfile::tempdir().expect("SDK fixture");
    let binary = fake_cli(
        fixture.path(),
        "printf 'claude 1.2.3\\n'; printf 'private-token\\n' >&2",
    );

    let findings = AgentSdkCheck.detect(&sdk_context(binary)).await;

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].check_id, "runtime/agent-sdk");
    assert_eq!(findings[0].severity, FindingSeverity::Pass);
    assert!(!findings[0].message.contains("private-token"));
    assert!(!findings[0].message.contains("1.2.3"));
    assert!(!findings[0].fix_hint.trim().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn agent_sdk_probe_reports_failure_without_echoing_stderr() {
    let fixture = tempfile::tempdir().expect("SDK fixture");
    let binary = fake_cli(
        fixture.path(),
        "printf 'private-error-token\\n' >&2; exit 7",
    );

    let findings = AgentSdkCheck.detect(&sdk_context(binary)).await;

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, FindingSeverity::Warn);
    assert!(!findings[0].message.contains("private-error-token"));
    assert!(!findings[0].fix_hint.trim().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn disabled_agent_sdk_is_skipped_without_running_its_configured_binary() {
    let fixture = tempfile::tempdir().expect("SDK fixture");
    let marker = fixture.path().join("ran");
    let binary = fake_cli(
        fixture.path(),
        &format!("printf x > '{}'", marker.display()),
    );
    let mut config = crate::config::Config::default();
    config.claude_agent_sdk.enabled = false;
    config.claude_agent_sdk.binary = binary.to_string_lossy().into_owned();

    let findings = AgentSdkCheck.detect(&DoctorCtx::from_config(config)).await;

    assert_eq!(findings[0].severity, FindingSeverity::Skipped);
    assert!(
        !marker.exists(),
        "disabled SDK check must not spawn its CLI"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn static_tier_does_not_spawn_the_connect_tier_agent_sdk_cli() {
    use crate::doctor::{DoctorOptions, DoctorTier};

    let fixture = tempfile::tempdir().expect("SDK fixture");
    let marker = fixture.path().join("ran");
    let binary = fake_cli(
        fixture.path(),
        &format!("printf x > '{}'", marker.display()),
    );
    let report = crate::doctor::runner::run(
        &sdk_context(binary),
        DoctorOptions {
            tier: DoctorTier::Static,
            only: vec!["runtime/agent-sdk".to_owned()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("above-tier SDK check is represented as skipped");

    assert_eq!(report.checks_skipped, vec!["runtime/agent-sdk".to_owned()]);
    assert!(report.findings.iter().any(|finding| {
        finding.check_id == "runtime/agent-sdk" && finding.severity == FindingSeverity::Skipped
    }));
    assert!(!marker.exists(), "static tier must not spawn the SDK CLI");
}
