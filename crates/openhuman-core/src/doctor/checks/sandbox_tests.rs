use super::*;
use crate::doctor::contract::FindingSeverity;
use std::path::Path;

struct DockerAvailability(bool);

#[async_trait]
impl SandboxProbe for DockerAvailability {
    async fn docker_available(&self) -> Result<bool, ()> {
        Ok(self.0)
    }

    async fn landlock_status(&self) -> LandlockStatus {
        LandlockStatus::Unavailable
    }
}

struct LandlockAvailability(LandlockStatus);

#[async_trait]
impl SandboxProbe for LandlockAvailability {
    async fn docker_available(&self) -> Result<bool, ()> {
        Ok(false)
    }

    async fn landlock_status(&self) -> LandlockStatus {
        self.0
    }
}

fn config(root: &Path, backend: SandboxBackend) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.workspace_dir = root.join("workspace");
    config.action_dir = root.join("actions");
    config.config_path = root.join("config.toml");
    config.sandbox.backend = backend;
    config
}

#[tokio::test]
async fn configured_docker_without_a_daemon_is_an_error() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let check = SandboxBackendCheck::with_probe(Arc::new(DockerAvailability(false)));
    let finding = check
        .detect(&DoctorCtx::from_config(config(
            scratch.path(),
            SandboxBackend::Docker,
        )))
        .await
        .remove(0);

    assert_eq!(finding.check_id, "sandbox/backend");
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(finding.message.contains("Docker backend is unavailable"));
}

#[tokio::test]
async fn available_docker_backend_passes() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let check = SandboxBackendCheck::with_probe(Arc::new(DockerAvailability(true)));
    let finding = check
        .detect(&DoctorCtx::from_config(config(
            scratch.path(),
            SandboxBackend::Docker,
        )))
        .await
        .remove(0);

    assert_eq!(finding.severity, FindingSeverity::Pass);
}

#[tokio::test]
async fn no_probe_runs_when_sandboxing_is_disabled_in_config() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let check = SandboxBackendCheck::with_probe(Arc::new(DockerAvailability(false)));
    let mut config = config(scratch.path(), SandboxBackend::Docker);
    config.sandbox.enabled = Some(false);
    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .remove(0);

    assert_eq!(finding.severity, FindingSeverity::Skipped);
    assert!(finding.message.contains("disabled"));
}

#[tokio::test]
async fn an_existing_tinybox_landlock_status_is_mapped_to_the_configured_backend() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let check =
        SandboxBackendCheck::with_probe(Arc::new(LandlockAvailability(LandlockStatus::Available)));
    let finding = check
        .detect(&DoctorCtx::from_config(config(
            scratch.path(),
            SandboxBackend::Landlock,
        )))
        .await
        .remove(0);

    assert_eq!(finding.severity, FindingSeverity::Pass);
}

#[tokio::test]
async fn unsupported_and_unavailable_landlock_statuses_are_distinguished() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let unavailable = SandboxBackendCheck::with_probe(Arc::new(LandlockAvailability(
        LandlockStatus::Unavailable,
    )))
    .detect(&DoctorCtx::from_config(config(
        scratch.path(),
        SandboxBackend::Landlock,
    )))
    .await
    .remove(0);
    assert_eq!(unavailable.severity, FindingSeverity::Skipped);

    let unsupported = SandboxBackendCheck::with_probe(Arc::new(LandlockAvailability(
        LandlockStatus::Unsupported,
    )))
    .detect(&DoctorCtx::from_config(config(
        scratch.path(),
        SandboxBackend::Landlock,
    )))
    .await
    .remove(0);
    assert_eq!(unsupported.severity, FindingSeverity::Error);
}

#[cfg(all(target_os = "linux", feature = "modules"))]
#[test]
fn landlock_requires_a_matching_backend_and_the_untrusted_code_isolation_floor() {
    use crate::modules::tinybox::contract::JailStatus;

    let available_landlock = JailStatus {
        backend: "landlock".to_owned(),
        available: true,
        isolation: "kernel".to_owned(),
        suitable_for_untrusted_code: true,
        filesystem: "enforced".to_owned(),
        network: "enforced".to_owned(),
        subprocess: "enforced".to_owned(),
    };
    assert_eq!(
        landlock_status_from_module(&available_landlock),
        LandlockStatus::Available
    );

    let available_other_backend = JailStatus {
        backend: "seatbelt".to_owned(),
        ..available_landlock.clone()
    };
    assert_eq!(
        landlock_status_from_module(&available_other_backend),
        LandlockStatus::Unsupported,
        "another available jail does not satisfy an explicit Landlock selection"
    );

    let insufficient_landlock = JailStatus {
        suitable_for_untrusted_code: false,
        ..available_landlock
    };
    assert_eq!(
        landlock_status_from_module(&insufficient_landlock),
        LandlockStatus::Unsupported
    );
}

#[cfg(all(target_os = "linux", feature = "modules"))]
#[tokio::test]
async fn missing_tinybox_status_does_not_start_the_module_runtime() {
    if crate::util::module_test_process::run(
        module_path!(),
        "missing_tinybox_status_does_not_start_the_module_runtime",
    ) {
        return;
    }
    assert!(!crate::modules::runtime::is_started());

    let status = HostSandboxProbe.landlock_status().await;

    assert_eq!(status, LandlockStatus::Unavailable);
    assert!(!crate::modules::runtime::is_started());
    assert_eq!(
        crate::modules::ops::state_of(crate::modules::tinybox::MODULE_ID),
        crate::modules::types::ModuleState::Available
    );
}
