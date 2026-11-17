//! SaaS doctor selection fixtures. This module is included by
//! `profiles::doctor` when the tenant resolver lands.

use super::{profile_provisioned_finding, resolve_with_loader, DoctorCaller, DoctorProfileState};
use crate::config::Config;
use crate::core::runtime::ComponentSet;
use crate::doctor::FindingSeverity;
use crate::doctor::{DoctorConfigSource, DoctorCtx};
use crate::doctor::{DoctorOptions, DoctorTier};
use crate::profiles::ProfileId;
use std::future::Ready;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadKind {
    Registry,
    Config,
    Credential,
    Lease,
}

#[derive(Default)]
struct ReadSpy(Mutex<Vec<(ReadKind, ProfileId)>>);

impl ReadSpy {
    fn record_selected_profile(&self, id: &ProfileId) {
        let mut reads = self.0.lock().unwrap();
        for kind in [
            ReadKind::Registry,
            ReadKind::Config,
            ReadKind::Credential,
            ReadKind::Lease,
        ] {
            reads.push((kind, id.clone()));
        }
    }

    fn snapshot(&self) -> Vec<(ReadKind, ProfileId)> {
        self.0.lock().unwrap().clone()
    }
}

fn spy_loader(spy: Arc<ReadSpy>) -> impl FnOnce(ProfileId) -> Ready<Result<ProfileId, String>> {
    move |id| {
        spy.record_selected_profile(&id);
        std::future::ready(Ok(id))
    }
}

fn profile(raw: &str) -> ProfileId {
    ProfileId::parse(raw).unwrap()
}

#[test]
fn selected_tenant_context_reports_tenant_source_and_profile() {
    let id = profile("alice");
    let ctx = DoctorCtx::from_config_with_components(Config::default(), ComponentSet::all())
        .with_profile(id.as_str(), DoctorConfigSource::Tenant);

    assert_eq!(ctx.profile(), Some(id.as_str()));
    assert_eq!(ctx.config_source(), DoctorConfigSource::Tenant);
}

#[tokio::test]
async fn a_tenant_foreign_profile_is_rejected_before_any_tenant_read() {
    let alice = profile("alice");
    let bob = profile("bob");
    let reads = Arc::new(ReadSpy::default());

    let result = resolve_with_loader(
        Some(DoctorCaller::Tenant(alice)),
        Some(bob.as_str()),
        spy_loader(Arc::clone(&reads)),
    )
    .await;

    assert!(result.is_err(), "a tenant cannot inspect another profile");
    assert!(
        reads.snapshot().is_empty(),
        "authorization must precede registry, config, credential, and lease reads"
    );
}

#[tokio::test]
async fn an_unscoped_saas_call_cannot_become_an_operator_profile_read() {
    let bob = profile("bob");
    let reads = Arc::new(ReadSpy::default());

    let result =
        resolve_with_loader(None, Some(bob.as_str()), spy_loader(Arc::clone(&reads))).await;

    assert!(result.is_err(), "missing task scope must fail closed");
    assert!(reads.snapshot().is_empty());
}

#[tokio::test]
async fn a_tenant_without_a_requested_id_loads_only_its_own_profile() {
    let alice = profile("alice");
    let bob = profile("bob");
    let reads = Arc::new(ReadSpy::default());

    let result = resolve_with_loader(
        Some(DoctorCaller::Tenant(alice.clone())),
        None,
        spy_loader(Arc::clone(&reads)),
    )
    .await
    .unwrap();

    assert_eq!(result, Some(alice.clone()));
    assert_eq!(
        reads.snapshot(),
        [
            (ReadKind::Registry, alice.clone()),
            (ReadKind::Config, alice.clone()),
            (ReadKind::Credential, alice.clone()),
            (ReadKind::Lease, alice),
        ]
    );
    assert!(!reads.snapshot().iter().any(|(_, id)| id == &bob));
}

#[tokio::test]
async fn a_tenant_may_explicitly_select_its_own_profile() {
    let alice = profile("alice");
    let reads = Arc::new(ReadSpy::default());

    let result = resolve_with_loader(
        Some(DoctorCaller::Tenant(alice.clone())),
        Some(alice.as_str()),
        spy_loader(Arc::clone(&reads)),
    )
    .await
    .unwrap();

    assert_eq!(result, Some(alice.clone()));
    assert!(reads.snapshot().iter().all(|(_, id)| id == &alice));
}

#[tokio::test]
async fn an_operator_can_select_any_profile_after_scope_is_verified() {
    let bob = profile("bob");
    let reads = Arc::new(ReadSpy::default());

    let result = resolve_with_loader(
        Some(DoctorCaller::Operator),
        Some(bob.as_str()),
        spy_loader(Arc::clone(&reads)),
    )
    .await
    .unwrap();

    assert_eq!(result, Some(bob.clone()));
    assert_eq!(
        reads.snapshot(),
        [
            (ReadKind::Registry, bob.clone()),
            (ReadKind::Config, bob.clone()),
            (ReadKind::Credential, bob.clone()),
            (ReadKind::Lease, bob),
        ]
    );
}

#[tokio::test]
async fn a_valid_but_unprovisioned_operator_target_reaches_the_profile_check() {
    let id = profile("not-provisioned");
    let reads = Arc::new(ReadSpy::default());

    let result = resolve_with_loader(Some(DoctorCaller::Operator), Some(id.as_str()), {
        let reads = Arc::clone(&reads);
        move |selected| {
            reads.record_selected_profile(&selected);
            std::future::ready(Ok(false))
        }
    })
    .await
    .unwrap();

    assert_eq!(
        result,
        Some(false),
        "the selected check reports provision status"
    );
    assert!(reads.snapshot().iter().all(|(_, selected)| selected == &id));
}

#[tokio::test]
async fn an_operator_without_a_selected_profile_does_not_load_a_tenant() {
    let reads = Arc::new(ReadSpy::default());

    let result = resolve_with_loader(
        Some(DoctorCaller::Operator),
        None,
        spy_loader(Arc::clone(&reads)),
    )
    .await
    .unwrap();

    assert!(
        result.is_none(),
        "the operator keeps its own normal config path"
    );
    assert!(reads.snapshot().is_empty());
}

#[tokio::test]
async fn malformed_profile_ids_are_rejected_before_tenant_reads() {
    let reads = Arc::new(ReadSpy::default());

    let result = resolve_with_loader(
        Some(DoctorCaller::Operator),
        Some("../bob"),
        spy_loader(Arc::clone(&reads)),
    )
    .await;

    assert!(result.is_err());
    assert!(reads.snapshot().is_empty());
}

#[test]
fn an_unprovisioned_profile_is_a_safe_profile_provisioned_error() {
    let finding = profile_provisioned_finding(DoctorProfileState::Unprovisioned);

    assert_eq!(finding.check_id, "profile/provisioned");
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(!finding.message.contains("secret"));
    assert!(!finding.message.contains("token"));
    assert!(!finding.fix_hint.contains("secret"));
}

#[tokio::test]
async fn dispatch_report_runs_registered_provisioned_check_without_creating_state() {
    const CHILD: &str = "OPENHUMAN_SAAS_DOCTOR_UNPROVISIONED_CHILD";
    let name = "dispatch_report_runs_registered_provisioned_check_without_creating_state";
    let module = module_path!().split_once("::").expect("crate module").1;
    let exact = format!("{module}::{name}");
    if std::env::var(CHILD).as_deref() != Ok(exact.as_str()) {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", &exact, "--nocapture", "--test-threads=1"])
            .env(CHILD, &exact)
            .output()
            .expect("run isolated SaaS profile doctor test");
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "isolated test {exact} failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    use crate::core::runtime::{mode, CoreContext, DomainSet, Mode, SaasConfig};
    use crate::storage::test_support::memory_backend;
    use crate::storage::{CollectionSpec, Scope, StorageBackend};

    mode::lock_mode(Mode::Saas).expect("isolated test process uses SaaS mode");
    let temp = tempfile::tempdir().unwrap();
    let backend: Arc<dyn StorageBackend> = memory_backend();
    let operator = CoreContext::for_test(DomainSet::saas(), None);
    let host = crate::profiles::ProfileHost::with_backend(
        SaasConfig::new(temp.path()),
        Arc::clone(&operator),
        Some(Arc::clone(&backend)),
    )
    .unwrap();
    let id = ProfileId::parse("not-provisioned").unwrap();
    let layout = host.layout_of(&id);
    let cluster = backend
        .for_scope(&Scope::new(crate::storage::lease::CLUSTER_SCOPE).unwrap())
        .unwrap();
    let registry_spec = CollectionSpec::new(crate::profiles::registry::REGISTRY_COLLECTION);
    let lease_spec = CollectionSpec::new(crate::storage::lease::LEASE_COLLECTION);
    assert!(cluster
        .documents()
        .validate_collection_spec(&registry_spec)
        .await
        .is_err());
    assert!(cluster
        .documents()
        .validate_collection_spec(&lease_spec)
        .await
        .is_err());
    crate::profiles::host::install(Arc::new(host));

    let options = DoctorOptions {
        tier: DoctorTier::Connect,
        only: vec!["profile/provisioned".into()],
        skip: Vec::new(),
        profile: Some(id.as_str().into()),
    };
    let dispatch = crate::core::runtime::DispatchCtx::new(Arc::clone(&operator));
    let outcome = CoreContext::scope(operator, async {
        crate::doctor::ops::doctor_report_for_dispatch(&dispatch, options).await
    })
    .await
    .unwrap();

    assert_eq!(outcome.value.profile.as_deref(), Some(id.as_str()));
    assert_eq!(outcome.value.config_source, DoctorConfigSource::Tenant);
    assert!(outcome
        .value
        .checks_run
        .iter()
        .any(|check| check == "profile/provisioned"));
    assert!(!outcome.value.ok);
    let finding = outcome
        .value
        .findings
        .iter()
        .find(|finding| finding.check_id == "profile/provisioned")
        .expect("registered profile/provisioned check result");
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(
        !layout.dir.exists(),
        "doctor must not create profile directories"
    );
    assert!(cluster
        .documents()
        .validate_collection_spec(&registry_spec)
        .await
        .is_err());
    assert!(cluster
        .documents()
        .validate_collection_spec(&lease_spec)
        .await
        .is_err());
}

#[test]
fn a_profile_held_by_another_node_is_reported_without_holder_details() {
    let finding = profile_provisioned_finding(DoctorProfileState::HeldElsewhere);

    assert_eq!(finding.check_id, "profile/provisioned");
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(!finding.message.contains("node-a"));
    assert!(!finding.message.contains("https://"));
    assert!(!finding.fix_hint.contains("token"));
}
