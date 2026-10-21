use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::core::runtime::{CoreContext, DomainSet};
use crate::core::slots::Extensions;
use crate::memory::engine::{Binding, EngineHealth};
use crate::memory::engine_registry::{
    register_memory_engine_in, register_memory_health_probe_in, EngineEntry, EngineFuture,
    ReadOnlyHealthProbe,
};

#[tokio::test]
async fn a_legacy_memory_selector_does_not_resolve_the_normal_engine() {
    let fixture = tempfile::tempdir().expect("doctor config fixture");
    let engine_id = "doctor-legacy-selector-test";
    let normal_builds = Arc::new(AtomicUsize::new(0));
    let normal_builds_in_closure = Arc::clone(&normal_builds);
    let extensions = Extensions::new();
    register_memory_engine_in(
        &extensions,
        engine_id,
        EngineEntry {
            build: Arc::new(move |_| {
                normal_builds_in_closure.fetch_add(1, Ordering::SeqCst);
                Binding::Off {
                    engine: Some(engine_id.to_owned()),
                    endpoint: None,
                    reason: "test engine must not be resolved by doctor options".to_owned(),
                }
            }),
            free_background: None,
        },
    );
    let readonly_probes = Arc::new(AtomicUsize::new(0));
    let readonly_probes_in_closure = Arc::clone(&readonly_probes);
    let readonly_probe: ReadOnlyHealthProbe = Arc::new(move |_| {
        let probes = Arc::clone(&readonly_probes_in_closure);
        Box::pin(async move {
            probes.fetch_add(1, Ordering::SeqCst);
            EngineHealth::Ok
        }) as EngineFuture<EngineHealth>
    });
    register_memory_health_probe_in(&extensions, engine_id, readonly_probe);

    let core = CoreContext::for_test_with_extensions(DomainSet::full(), None, extensions);
    let mut config = Config::default();
    config.memory.engine = engine_id.to_owned();
    config.workspace_dir = fixture.path().join("workspace");
    config.action_dir = fixture.path().join("actions");
    config.config_path = fixture.path().join("config.toml");
    let context = DoctorCtx::from_config_with_components(config.clone(), ComponentSet::all());
    let options = DoctorOptions {
        tier: DoctorTier::Connect,
        only: vec!["memory/engine".to_owned()],
        ..DoctorOptions::default()
    };

    CoreContext::scope(core, report_with_context(options, context))
        .await
        .expect("a removed selector is reported in the doctor response");

    assert_eq!(normal_builds.load(Ordering::SeqCst), 0);
    assert_eq!(readonly_probes.load(Ordering::SeqCst), 0);
}
