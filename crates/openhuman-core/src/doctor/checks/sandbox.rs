//! Bounded probes for explicitly configured sandbox backends.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::config::SandboxBackend;
use crate::doctor::contract::{ComponentId, DoctorCheck, DoctorCtx, DoctorTier, Finding};

#[async_trait]
trait SandboxProbe: Send + Sync {
    async fn docker_available(&self) -> Result<bool, ()>;
    async fn landlock_status(&self) -> LandlockStatus;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LandlockStatus {
    Available,
    Unsupported,
    Unavailable,
    Failed,
}

struct HostSandboxProbe;

#[async_trait]
impl SandboxProbe for HostSandboxProbe {
    async fn docker_available(&self) -> Result<bool, ()> {
        use std::process::Stdio;

        let mut command = tokio::process::Command::new("docker");
        command
            .args(["info", "--format", "{{.ServerVersion}}"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        match tokio::time::timeout(Duration::from_secs(5), command.status()).await {
            Ok(Ok(status)) => Ok(status.success()),
            Ok(Err(_)) | Err(_) => Err(()),
        }
    }

    async fn landlock_status(&self) -> LandlockStatus {
        #[cfg(all(target_os = "linux", feature = "modules"))]
        {
            use crate::modules::runtime::{call_if_ready, ExistingModuleCallError};
            use crate::modules::tinybox::contract::names::JAIL_STATUS;
            use crate::modules::tinybox::contract::JailStatus;

            return match call_if_ready::<_, JailStatus>(
                crate::modules::tinybox::MODULE_ID,
                JAIL_STATUS,
                (),
                Duration::from_secs(5),
            )
            .await
            {
                Ok(Some(status)) => landlock_status_from_module(&status),
                Ok(None) | Err(ExistingModuleCallError::Unavailable) => LandlockStatus::Unavailable,
                Err(ExistingModuleCallError::Failed) => LandlockStatus::Failed,
            };
        }
        #[cfg(all(target_os = "linux", not(feature = "modules")))]
        {
            LandlockStatus::Unavailable
        }
        #[cfg(not(target_os = "linux"))]
        {
            LandlockStatus::Unsupported
        }
    }
}

#[cfg(all(target_os = "linux", feature = "modules"))]
fn landlock_status_from_module(
    status: &crate::modules::tinybox::contract::JailStatus,
) -> LandlockStatus {
    if status.backend == "landlock" && status.available && status.suitable_for_untrusted_code {
        LandlockStatus::Available
    } else {
        LandlockStatus::Unsupported
    }
}

pub(super) fn for_context(_ctx: &DoctorCtx) -> Vec<Arc<dyn DoctorCheck>> {
    vec![Arc::new(SandboxBackendCheck {
        probe: Arc::new(HostSandboxProbe),
    })]
}

struct SandboxBackendCheck {
    probe: Arc<dyn SandboxProbe>,
}

impl SandboxBackendCheck {
    #[cfg(test)]
    fn with_probe(probe: Arc<dyn SandboxProbe>) -> Self {
        Self { probe }
    }

    async fn finding(&self, ctx: &DoctorCtx) -> Finding {
        if !ctx.config().components.sandbox || ctx.config().sandbox.enabled == Some(false) {
            return Finding::skipped(
                "sandbox/backend",
                "Sandboxing is disabled in the selected configuration.",
                "Enable sandboxing in the selected configuration to check its backend.",
            );
        }
        match &ctx.config().sandbox.backend {
            SandboxBackend::Auto => Finding::pass(
                "sandbox/backend",
                "The sandbox backend is selected automatically by the runtime.",
            ),
            SandboxBackend::None => Finding::skipped(
                "sandbox/backend",
                "No sandbox backend is configured.",
                "Select a sandbox backend to check its availability.",
            ),
            SandboxBackend::Docker => match self.probe.docker_available().await {
                Ok(true) => Finding::pass(
                    "sandbox/backend",
                    "The configured Docker backend is available.",
                ),
                Ok(false) | Err(()) => Finding::error(
                    "sandbox/backend",
                    "The configured Docker backend is unavailable.",
                    "Start Docker and confirm that the current user can reach its daemon.",
                ),
            },
            SandboxBackend::Landlock => match self.probe.landlock_status().await {
                LandlockStatus::Available => Finding::pass(
                    "sandbox/backend",
                    "The configured Landlock backend is available.",
                ),
                LandlockStatus::Unsupported => Finding::error(
                    "sandbox/backend",
                    "The configured Landlock backend is unavailable on this host.",
                    "Use a Linux kernel and TinyBox build with Landlock support, or select another backend.",
                ),
                LandlockStatus::Unavailable => Finding::skipped(
                    "sandbox/backend",
                    "TinyBox is not already available to report its native jail capability.",
                    "Open the sandbox runtime, then rerun doctor to inspect its backend.",
                ),
                LandlockStatus::Failed => Finding::error(
                    "sandbox/backend",
                    "The existing TinyBox runtime did not return native jail status.",
                    "Review the sandbox runtime and rerun doctor.",
                ),
            },
            SandboxBackend::Firejail | SandboxBackend::Bubblewrap => Finding::error(
                "sandbox/backend",
                "The configured sandbox backend is not supported by this build.",
                "Select Docker, Landlock, automatic selection, or no backend.",
            ),
        }
    }
}

#[async_trait]
impl DoctorCheck for SandboxBackendCheck {
    fn id(&self) -> &str {
        "sandbox/backend"
    }

    fn category(&self) -> &str {
        "sandbox"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Sandbox)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        vec![self.finding(ctx).await]
    }
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
