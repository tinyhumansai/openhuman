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
