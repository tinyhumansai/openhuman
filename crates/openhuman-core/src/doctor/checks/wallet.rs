//! Read-only inspection of an existing wallet state snapshot.

use std::sync::Arc;

use crate::doctor::{ComponentId, DoctorCheck, DoctorTier, Finding};
use async_trait::async_trait;

#[cfg(feature = "web3")]
use crate::config::Config;

#[cfg(feature = "web3")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalletSnapshotFailure {
    Malformed,
    Unavailable,
}
#[cfg(feature = "web3")]
type SnapshotFailure = WalletSnapshotFailure;

#[cfg(feature = "web3")]
#[derive(Debug, Clone, Copy)]
struct WalletSnapshot {
    configured: bool,
}

#[cfg(feature = "web3")]
trait WalletSnapshotProbe: Send + Sync {
    fn read_existing(
        &self,
        config: &Config,
    ) -> Result<Option<WalletSnapshot>, WalletSnapshotFailure>;
}

#[cfg(feature = "web3")]
struct FileSnapshot;

#[cfg(feature = "web3")]
impl WalletSnapshotProbe for FileSnapshot {
    fn read_existing(
        &self,
        config: &Config,
    ) -> Result<Option<WalletSnapshot>, WalletSnapshotFailure> {
        use std::io::Read;

        const MAX_SNAPSHOT_BYTES: u64 = 1024 * 1024;
        let path = config.workspace_dir.join("state").join("wallet-state.json");
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(SnapshotFailure::Unavailable),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SnapshotFailure::Unavailable);
        }
        if metadata.len() > MAX_SNAPSHOT_BYTES {
            return Err(SnapshotFailure::Malformed);
        }
        let mut contents = Vec::with_capacity(metadata.len() as usize);
        std::fs::File::open(path)
            .map_err(|_| SnapshotFailure::Unavailable)?
            .take(MAX_SNAPSHOT_BYTES + 1)
            .read_to_end(&mut contents)
            .map_err(|_| SnapshotFailure::Unavailable)?;
        if contents.len() as u64 > MAX_SNAPSHOT_BYTES {
            return Err(SnapshotFailure::Malformed);
        }
        let inspection = crate::web3::wallet::inspect_stored_wallet_state(&contents)
            .map_err(|_| SnapshotFailure::Malformed)?;
        let configured = inspection.consent_granted && inspection.account_count > 0;
        Ok(Some(WalletSnapshot { configured }))
    }
}

pub(super) fn for_context() -> Vec<Arc<dyn DoctorCheck>> {
    vec![Arc::new(WalletStatusCheck {
        #[cfg(feature = "web3")]
        probe: Arc::new(FileSnapshot),
    })]
}

struct WalletStatusCheck {
    #[cfg(feature = "web3")]
    probe: Arc<dyn WalletSnapshotProbe>,
}

impl WalletStatusCheck {
    #[cfg(all(test, feature = "web3"))]
    fn with_probe(probe: Arc<dyn WalletSnapshotProbe>) -> Self {
        Self { probe }
    }
}

#[async_trait]
impl DoctorCheck for WalletStatusCheck {
    fn id(&self) -> &str {
        "wallet/status"
    }
    fn category(&self) -> &str {
        "wallet"
    }
    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }
    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Wallet)
    }

    async fn detect(&self, ctx: &crate::doctor::DoctorCtx) -> Vec<Finding> {
        #[cfg(not(feature = "web3"))]
        let finding = Finding::skipped(
            self.id(),
            "Wallet support is not included in this build.",
            "Use a build with the wallet component enabled to inspect wallet state.",
        );
        #[cfg(feature = "web3")]
        let finding = match self.probe.read_existing(ctx.config()) {
            Ok(Some(WalletSnapshot { configured: true })) => {
                Finding::pass(self.id(), "An existing wallet snapshot is configured.")
            }
            Ok(Some(WalletSnapshot { configured: false })) | Ok(None) => Finding::skipped(
                self.id(),
                "No configured wallet snapshot exists in the selected workspace.",
                "Set up a wallet through the normal wallet flow, then rerun doctor.",
            ),
            Err(SnapshotFailure::Malformed) => Finding::error(
                self.id(),
                "The selected wallet snapshot is malformed.",
                "Review wallet settings before retrying; doctor leaves the snapshot untouched.",
            ),
            Err(SnapshotFailure::Unavailable) => Finding::error(
                self.id(),
                "The selected wallet snapshot could not be read.",
                "Check local workspace permissions, then rerun doctor.",
            ),
        };
        #[cfg(not(feature = "web3"))]
        let _ = ctx;
        vec![finding]
    }
}

#[cfg(test)]
#[path = "wallet_tests.rs"]
mod tests;
