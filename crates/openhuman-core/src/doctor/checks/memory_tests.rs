use super::*;
use crate::core::runtime::ComponentSet;
use crate::core::runtime::{CoreContext, DomainSet};
use crate::core::slots::Extensions;
use crate::doctor::contract::{DoctorConfigSource, DoctorCtx, FindingSeverity};
use crate::memory::engine::{Binding, EngineHealth, DISABLED_ENGINE};
use crate::memory::engine_registry::{
    register_memory_engine_in, register_memory_health_probe_in, EngineEntry, EngineFuture,
    ReadOnlyHealthProbe,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn context(engine: &str) -> DoctorCtx {
    let mut config = crate::config::Config::default();
    config.memory.engine = engine.to_owned();
    DoctorCtx::from_config_with_components(config, ComponentSet::all())
        .with_profile("selected-b", DoctorConfigSource::Profile)
}

#[tokio::test]
async fn binding_uses_only_the_selected_config() {
    let finding = BindingCheck
        .detect(&context(crate::memory::engine::CORTEXDB_ENGINE))
        .await;
    assert_eq!(finding[0].check_id, "memory/binding");
    assert_eq!(finding[0].severity, FindingSeverity::Pass);
}

#[tokio::test]
async fn binding_reports_disabled_memory_without_resolving_an_engine() {
    let finding = BindingCheck.detect(&context(DISABLED_ENGINE)).await;
    assert_eq!(finding[0].severity, FindingSeverity::Skipped);
}

#[tokio::test]
async fn health_is_skipped_when_memory_is_intentionally_disabled() {
    let finding = HealthCheck.detect(&context(DISABLED_ENGINE)).await;
    assert_eq!(finding[0].severity, FindingSeverity::Skipped);
}

#[tokio::test]
async fn selected_health_uses_the_read_only_probe_instead_of_the_cached_engine_builder() {
    let fixture = tempfile::tempdir().expect("memory health fixture");
    let engine_id = "doctor-read-only-health-test";
    let builder_calls = Arc::new(AtomicUsize::new(0));
    let builder_calls_in_closure = Arc::clone(&builder_calls);
    let extensions = Extensions::new();
    register_memory_engine_in(
        &extensions,
        engine_id,
        EngineEntry {
            build: Arc::new(move |_| {
                builder_calls_in_closure.fetch_add(1, Ordering::SeqCst);
                Binding::Off {
                    engine: Some(engine_id.to_owned()),
                    endpoint: None,
                    reason: "normal cached builder must not run in doctor".to_owned(),
                }
            }),
            free_background: None,
        },
    );
    let probe_calls = Arc::new(AtomicUsize::new(0));
    let probe_calls_in_closure = Arc::clone(&probe_calls);
    let probe: ReadOnlyHealthProbe = Arc::new(move |_| {
        let calls = Arc::clone(&probe_calls_in_closure);
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            EngineHealth::Ok
        }) as EngineFuture<EngineHealth>
    });
    register_memory_health_probe_in(&extensions, engine_id, probe);

    let core = CoreContext::for_test_with_extensions(DomainSet::full(), None, extensions);
    let mut config = crate::config::Config::default();
    config.memory.engine = engine_id.to_owned();
    config.workspace_dir = fixture.path().join("workspace");
    let context = DoctorCtx::from_config_with_components(config, ComponentSet::all());
    let findings = CoreContext::scope(core, HealthCheck.detect(&context)).await;

    assert_eq!(builder_calls.load(Ordering::SeqCst), 0);
    assert_eq!(probe_calls.load(Ordering::SeqCst), 1);
    assert_eq!(findings[0].check_id, "memory/health");
    assert_eq!(findings[0].severity, FindingSeverity::Pass);
}

#[test]
fn health_reports_a_safe_engine_down_reason() {
    let finding = findings_for_health(EngineHealth::Down("engine is down".to_owned()));
    assert_eq!(finding[0].check_id, "memory/health");
    assert_eq!(finding[0].severity, FindingSeverity::Error);
    assert_eq!(finding[0].message, "engine is down");
}

#[test]
fn health_redacts_urls_and_credential_like_reasons() {
    let finding = findings_for_health(EngineHealth::Down(
        "request to https://user:secret@example.test failed".to_owned(),
    ));
    assert!(!finding[0].message.contains("secret"));
}

#[test]
fn health_withholds_bare_secrets_and_local_paths() {
    for private_reason in ["sk-live-secret-value", "/Users/private/person/data.db"] {
        let finding = findings_for_health(EngineHealth::Down(private_reason.to_owned()));
        assert!(!finding[0].message.contains(private_reason));
        assert!(finding[0].message.contains("details are withheld"));
    }
}

#[test]
fn hosted_memory_binding_requires_a_registered_host_adapter() {
    assert!(!binding_supported(
        crate::memory::engine::TINYHUMANS_ENGINE,
        false
    ));
    assert!(binding_supported(
        crate::memory::engine::TINYHUMANS_ENGINE,
        true
    ));
    assert!(binding_supported(
        crate::memory::engine::CORTEXDB_ENGINE,
        false
    ));
}
