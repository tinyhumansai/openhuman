//! Safe diagnostics for the configured secret backend.

use async_trait::async_trait;

use crate::doctor::contract::{DoctorCheck, DoctorCtx, DoctorTier, Finding};

pub(super) struct KeyringCheck;

#[async_trait]
impl DoctorCheck for KeyringCheck {
    fn id(&self) -> &str {
        "secrets/keyring"
    }

    fn category(&self) -> &str {
        "secrets"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    fn is_blocking(&self) -> bool {
        true
    }

    async fn detect(&self, _ctx: &DoctorCtx) -> Vec<Finding> {
        self.detect_blocking(_ctx).unwrap_or_default()
    }

    fn detect_blocking(&self, _ctx: &DoctorCtx) -> Option<Vec<Finding>> {
        use std::io::IsTerminal;

        let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
        let finding = keyring_with_probe(cfg!(target_os = "macos"), interactive, || {
            (
                crate::security::secret_storage::ops::backend_name(),
                crate::security::secret_storage::ops::is_available(),
            )
        });
        Some(vec![finding])
    }
}

fn keyring_with_probe(
    is_macos: bool,
    interactive: bool,
    probe: impl FnOnce() -> (String, bool),
) -> Finding {
    if !interactive {
        let message = if is_macos {
            "The macOS keyring probe was skipped because this session is non-interactive."
        } else {
            "The keyring probe was skipped because this session is non-interactive."
        };
        return Finding::skipped(
            "secrets/keyring",
            message,
            "Rerun doctor in an interactive terminal to check keyring availability.",
        );
    }
    let (backend, available) = probe();
    if available {
        Finding::pass(
            "secrets/keyring",
            format!("The configured {backend} secret backend is available."),
        )
    } else {
        Finding::error(
            "secrets/keyring",
            format!("The configured {backend} secret backend is unavailable."),
            "Check the selected secret backend and its access permissions.",
        )
    }
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
