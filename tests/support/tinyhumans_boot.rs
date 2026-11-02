//! Give an in-process core its TinyHumans backend transport.
//!
//! The core carries no backend client. Suites that spawn the
//! `openhuman-core` binary get the transport from `main.rs`
//! (`openhuman_rpc::host::cli`); suites that boot
//! the core in-process (`build_core_http_router`, direct `ops` calls against
//! the mock backend) must call [`boot`] first or every backend-touching call
//! answers `BACKEND_UNAVAILABLE:`. Idempotent and cheap: call it from any
//! fixture, as often as you like.
//!
//! Include with `#[path = "support/tinyhumans_boot.rs"] mod tinyhumans_boot;`
//! (or `../support/...` from `tests/raw_coverage/`).

#![allow(dead_code)]

use std::sync::Once;

static BOOT: Once = Once::new();

/// Install the SDK-backed backend transport once per test process.
pub fn boot() {
    BOOT.call_once(|| {
        openhuman_tinyhumans::register_hooks();
        openhuman_tinyhumans::install(openhuman_tinyhumans::InstallOptions::default())
            .expect("install the TinyHumans backend transport for tests");
    });
}

/// Whether [`boot`] has run in this process. A suite that asserts on a surface
/// the hosted layer adds (or must not add) uses this when it shares a process
/// with suites that boot the transport.
pub fn is_booted() -> bool {
    BOOT.is_completed()
}

/// Boot the process's default core context over `config`, once.
///
/// Modules reach host services (the storage bus) through the ambient dispatch
/// context, which in a headless process is the default context a real host
/// always builds. A test that calls domain ops directly has none unless an
/// earlier test in the same process happened to build one, so under per-test
/// processes (nextest) it must boot one itself.
pub async fn boot_default_context(config: openhuman_core::config::Config) {
    use openhuman_core::core::runtime::{CoreContext, DomainSet, TokenSource};
    if CoreContext::default_context().is_some() {
        return;
    }
    openhuman_core::core::bus::init()
        .await
        .expect("init the event bus");
    CoreContext::init_with_config(
        openhuman_core::core::types::HostKind::Library,
        &TokenSource::Fixed(std::sync::Arc::new("test-token".to_string())),
        DomainSet::full(),
        openhuman_core::tools::toolpacks::ToolGroups::none(),
        Some(config),
    )
    .await
    .expect("boot the default core context");
}
