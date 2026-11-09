use super::*;
use crate::doctor::contract::{DoctorCtx, FindingSeverity};
#[cfg(feature = "web3")]
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[cfg(feature = "web3")]
struct ReadOnlySnapshot {
    expected_workspace: std::path::PathBuf,
    snapshot: Result<Option<WalletSnapshot>, WalletSnapshotFailure>,
    reads: Arc<AtomicUsize>,
    keychain_queries: Arc<AtomicUsize>,
}

#[cfg(feature = "web3")]
impl WalletSnapshotProbe for ReadOnlySnapshot {
    fn read_existing(
        &self,
        config: &crate::config::Config,
    ) -> Result<Option<WalletSnapshot>, WalletSnapshotFailure> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        assert_eq!(config.workspace_dir, self.expected_workspace);
        self.snapshot
    }
}

fn config_in(root: &std::path::Path) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.workspace_dir = root.join("workspace");
    config.action_dir = root.join("actions");
    config.config_path = root.join("config.toml");
    config
}

#[cfg(feature = "web3")]
fn stored_wallet_json(address: &str) -> String {
    serde_json::json!({
        "consentGranted": true,
        "source": "generated",
        "mnemonicWordCount": 12,
        "encryptedMnemonic": "enc2:doctor-fake-encrypted-mnemonic",
        "accounts": [{
            "chain": "evm",
            "address": address,
            "derivationPath": "m/44'/60'/0'/0/0"
        }],
        "updatedAtMs": 7
    })
    .to_string()
}

#[cfg(not(feature = "web3"))]
#[tokio::test]
async fn feature_off_wallet_facade_skips_without_reading_wallet_state() {
    let scratch = tempfile::tempdir().expect("create isolated wallet fixture");
    let config = config_in(scratch.path());
    let state_path = config.workspace_dir.join("state").join("wallet-state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).expect("create state fixture");
    let original = b"wallet snapshot remains untouched";
    std::fs::write(&state_path, original).expect("write state fixture");
    let check = WalletStatusCheck {};
    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .remove(0);
    assert_eq!(finding.severity, FindingSeverity::Skipped);
    assert_eq!(std::fs::read(&state_path).unwrap(), original);
}

#[cfg(feature = "web3")]
#[tokio::test]
async fn a_valid_wallet_snapshot_is_inspected_without_keychain_access_or_rewrite() {
    let scratch = tempfile::tempdir().expect("create isolated wallet fixture");
    let config = config_in(scratch.path());
    let state_path = config.workspace_dir.join("state").join("wallet-state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).expect("create fixture state directory");
    let original = stored_wallet_json("0xdoctor-fixture-address");
    std::fs::write(&state_path, &original).expect("write fixture wallet state");

    let reads = Arc::new(AtomicUsize::new(0));
    let keychain_queries = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(ReadOnlySnapshot {
        expected_workspace: config.workspace_dir.clone(),
        snapshot: Ok(Some(WalletSnapshot { configured: true })),
        reads: Arc::clone(&reads),
        keychain_queries: Arc::clone(&keychain_queries),
    });
    let check = WalletStatusCheck::with_probe(probe.clone());

    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .into_iter()
        .next()
        .expect("existing wallet snapshot produces a finding");

    assert_eq!(finding.check_id, "wallet/status");
    assert_eq!(finding.severity, FindingSeverity::Pass);
    assert!(!finding.message.contains("0xdoctor-fixture-address"));
    assert!(!finding.message.contains("doctor-fake-encrypted-mnemonic"));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(keychain_queries.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_to_string(&state_path).unwrap(), original);
}

#[cfg(feature = "web3")]
#[tokio::test]
async fn the_default_snapshot_reader_only_reads_selected_workspace_bytes() {
    let ambient = tempfile::tempdir().expect("create ambient wallet fixture");
    let selected = tempfile::tempdir().expect("create selected wallet fixture");
    let config_a = config_in(ambient.path());
    let config_b = config_in(selected.path());
    let state_a = config_a
        .workspace_dir
        .join("state")
        .join("wallet-state.json");
    let state_b = config_b
        .workspace_dir
        .join("state")
        .join("wallet-state.json");
    std::fs::create_dir_all(state_a.parent().unwrap()).expect("create ambient state");
    std::fs::create_dir_all(state_b.parent().unwrap()).expect("create selected state");
    std::fs::write(&state_a, stored_wallet_json("0xambient-address")).unwrap();
    let selected_bytes = stored_wallet_json("0xselected-address");
    std::fs::write(&state_b, &selected_bytes).unwrap();
    let check = WalletStatusCheck {
        probe: Arc::new(FileSnapshot),
    };
    let ambient_context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config_a,
    );
    let finding = crate::core::runtime::CoreContext::scope(ambient_context, async {
        check
            .detect(&DoctorCtx::from_config(config_b))
            .await
            .remove(0)
    })
    .await;

    assert_eq!(finding.severity, FindingSeverity::Pass);
    assert!(!finding.message.contains("0xselected-address"));
    assert_eq!(std::fs::read_to_string(&state_b).unwrap(), selected_bytes);
    assert_eq!(
        std::fs::read_dir(state_b.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[cfg(feature = "web3")]
#[tokio::test]
async fn oversized_wallet_snapshot_is_rejected_without_rewriting_it() {
    let scratch = tempfile::tempdir().expect("create isolated wallet fixture");
    let config = config_in(scratch.path());
    let state_path = config.workspace_dir.join("state").join("wallet-state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let original = vec![b' '; 1024 * 1024 + 1];
    std::fs::write(&state_path, &original).unwrap();

    let check = WalletStatusCheck {
        probe: Arc::new(FileSnapshot),
    };
    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .remove(0);
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert_eq!(std::fs::read(&state_path).unwrap(), original);
    assert_eq!(
        std::fs::read_dir(state_path.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[cfg(feature = "web3")]
#[tokio::test]
async fn malformed_wallet_account_records_are_rejected_without_rewriting_state() {
    let scratch = tempfile::tempdir().expect("create isolated wallet fixture");
    let config = config_in(scratch.path());
    let state_path = config.workspace_dir.join("state").join("wallet-state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let original = serde_json::json!({
        "consentGranted": true,
        "source": "generated",
        "mnemonicWordCount": 12,
        "encryptedMnemonic": "enc2:doctor-fake-encrypted-mnemonic",
        "accounts": [null],
        "updatedAtMs": 7
    })
    .to_string();
    std::fs::write(&state_path, &original).unwrap();

    let check = WalletStatusCheck {
        probe: Arc::new(FileSnapshot),
    };
    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .remove(0);

    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(!finding.message.contains("doctor-fake-encrypted-mnemonic"));
    assert_eq!(std::fs::read_to_string(&state_path).unwrap(), original);
}

#[cfg(feature = "web3")]
#[tokio::test]
async fn malformed_selected_wallet_state_is_reported_without_quarantine_or_deletion() {
    let scratch = tempfile::tempdir().expect("create isolated wallet fixture");
    let config = config_in(scratch.path());
    let state_path = config.workspace_dir.join("state").join("wallet-state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).expect("create fixture state directory");
    let original = "{broken but private}\n";
    std::fs::write(&state_path, original).expect("write malformed fixture state");

    let reads = Arc::new(AtomicUsize::new(0));
    let keychain_queries = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(ReadOnlySnapshot {
        expected_workspace: config.workspace_dir.clone(),
        snapshot: Err(WalletSnapshotFailure::Malformed),
        reads,
        keychain_queries: Arc::clone(&keychain_queries),
    });
    let check = WalletStatusCheck::with_probe(probe);

    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .into_iter()
        .next()
        .expect("malformed state produces a safe finding");

    assert_eq!(finding.check_id, "wallet/status");
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(!finding.message.contains("broken but private"));
    assert_eq!(keychain_queries.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_to_string(&state_path).unwrap(), original);
    assert_eq!(
        std::fs::read_dir(state_path.parent().unwrap())
            .unwrap()
            .count(),
        1,
        "the read-only check must not create a quarantine copy"
    );
}

#[cfg(feature = "web3")]
#[tokio::test]
async fn ambient_wallet_state_is_not_used_for_selected_b_workspace() {
    let ambient = tempfile::tempdir().expect("create ambient wallet fixture");
    let selected = tempfile::tempdir().expect("create selected wallet fixture");
    let config_a = config_in(ambient.path());
    let config_b = config_in(selected.path());
    let path_a = config_a
        .workspace_dir
        .join("state")
        .join("wallet-state.json");
    std::fs::create_dir_all(path_a.parent().unwrap()).expect("create ambient state directory");
    std::fs::write(&path_a, stored_wallet_json("0xambient-a-address"))
        .expect("write ambient wallet state");

    let reads = Arc::new(AtomicUsize::new(0));
    let keychain_queries = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(ReadOnlySnapshot {
        expected_workspace: config_b.workspace_dir.clone(),
        snapshot: Ok(None),
        reads: Arc::clone(&reads),
        keychain_queries: Arc::clone(&keychain_queries),
    });
    let check = WalletStatusCheck::with_probe(probe);
    let ambient_context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config_a,
    );
    let finding = crate::core::runtime::CoreContext::scope(ambient_context, async {
        check
            .detect(&DoctorCtx::from_config(config_b.clone()))
            .await
            .into_iter()
            .next()
            .expect("selected-B empty state is represented")
    })
    .await;

    assert_eq!(finding.check_id, "wallet/status");
    assert_eq!(finding.severity, FindingSeverity::Skipped);
    assert!(!finding.message.contains("0xambient-a-address"));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(keychain_queries.load(Ordering::SeqCst), 0);
}
