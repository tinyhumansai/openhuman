use super::*;
use crate::doctor::contract::{DoctorCtx, FindingSeverity};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct FixedVoiceStatus {
    snapshot: VoiceSnapshot,
    calls: Arc<AtomicUsize>,
    workspace: std::sync::Mutex<Option<std::path::PathBuf>>,
}

#[async_trait::async_trait]
impl VoiceStatusProbe for FixedVoiceStatus {
    async fn inspect(&self, config: &crate::config::Config) -> VoiceSnapshot {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.workspace.lock().unwrap() = Some(config.workspace_dir.clone());
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

#[tokio::test]
async fn stt_and_tts_configuration_findings_are_separate_and_sanitized() {
    let scratch = tempfile::tempdir().expect("create isolated fixture");
    let config = config_in(scratch.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(FixedVoiceStatus {
        snapshot: VoiceSnapshot {
            stt: Err(VoiceIssue::CredentialMissing),
            tts: Ok(()),
        },
        calls: Arc::clone(&calls),
        workspace: Default::default(),
    });
    let checks = checks_with_probe(probe);
    let ctx = DoctorCtx::from_config(config);
    let stt = checks[0].detect(&ctx).await.remove(0);
    let tts = checks[1].detect(&ctx).await.remove(0);
    assert_eq!(stt.check_id, "voice/stt");
    assert_eq!(stt.severity, FindingSeverity::Error);
    assert_eq!(tts.check_id, "voice/tts");
    assert_eq!(tts.severity, FindingSeverity::Pass);
    assert_eq!(
        stt.message,
        "The selected voice provider has no saved credential."
    );
    assert_eq!(
        stt.fix_hint,
        "Connect the selected speech provider or choose an available route."
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn voice_checks_inspect_the_selected_config_without_starting_audio_runtime() {
    let ambient = tempfile::tempdir().expect("create ambient fixture");
    let selected = tempfile::tempdir().expect("create selected fixture");
    let config_a = config_in(ambient.path());
    let config_b = config_in(selected.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(FixedVoiceStatus {
        snapshot: VoiceSnapshot {
            stt: Ok(()),
            tts: Ok(()),
        },
        calls: Arc::clone(&calls),
        workspace: Default::default(),
    });
    let checks = checks_with_probe(probe.clone());
    let ambient_context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config_a,
    );
    let findings = crate::core::runtime::CoreContext::scope(ambient_context, async {
        let ctx = DoctorCtx::from_config(config_b.clone());
        let mut findings = Vec::new();
        for check in &checks {
            findings.extend(check.detect(&ctx).await);
        }
        findings
    })
    .await;
    assert_eq!(findings.len(), 2);
    assert!(findings
        .iter()
        .all(|finding| finding.severity == FindingSeverity::Pass));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        *probe.workspace.lock().unwrap(),
        Some(config_b.workspace_dir)
    );
}
