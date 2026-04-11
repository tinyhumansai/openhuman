//! Shared test utilities for stubbing the global native bus registry.
//!
//! The native event bus ([`super::native_request`]) is a process-wide
//! singleton. Any test that installs a stub handler must:
//!
//!   1. Acquire [`BUS_HANDLER_LOCK`] so concurrent dispatch tests don't
//!      clobber each other's registrations.
//!   2. Install the typed stub on the global registry.
//!   3. Restore the production handler on teardown — even if the test
//!      panics — so subsequent tests observe a clean registry.
//!
//! Historically every stub test open-coded all three steps, which was
//! error-prone: a panic between step 2 and step 3 left the registry in an
//! inconsistent state, and subsequent tests failed with confusing
//! "handler was called N times" assertions.
//!
//! This module wraps the pattern in an RAII [`MockBusGuard`]. The generic
//! [`mock_bus_stub`] helper installs a typed stub for any method name, and
//! domain-specific conveniences (such as
//! [`crate::openhuman::agent::bus::mock_agent_run_turn`]) compose on top of
//! it by providing a method name + a restore closure that re-registers the
//! production handler.
//!
//! Tests in **any** module of `openhuman_core` can `use
//! crate::core::event_bus::testing::{mock_bus_stub, MockBusGuard,
//! BUS_HANDLER_LOCK};` — this module is not gated on `#[cfg(test)]` at the
//! module level so that `pub` items remain referenceable from integration
//! tests as well as unit tests.
//!
//! # Example
//!
//! ```ignore
//! use crate::core::event_bus::testing::mock_bus_stub;
//!
//! // Install a stub for a hypothetical `billing.charge` method with a
//! // custom restore closure. The restore fn runs when the guard drops.
//! let _guard = mock_bus_stub::<BillingChargeRequest, BillingChargeResponse, _, _, _>(
//!     "billing.charge",
//!     |req| async move {
//!         assert_eq!(req.amount_cents, 500);
//!         Ok(BillingChargeResponse { charge_id: "stub".into() })
//!     },
//!     || register_billing_handlers(),
//! )
//! .await;
//!
//! // ... drive the code under test ...
//! // Guard drops here → `register_billing_handlers()` runs automatically.
//! ```

use std::future::Future;

use tokio::sync::{Mutex as TokioMutex, MutexGuard as TokioMutexGuard};

use super::native_request::register_native_global;

/// Process-wide exclusion lock for tests that install mock bus handlers.
///
/// Acquired by [`mock_bus_stub`] for the lifetime of the returned
/// [`MockBusGuard`], and also by helpers such as
/// [`crate::openhuman::agent::bus::use_real_agent_handler`] that need the
/// real agent handler installed without racing against a stub-installing
/// test. Any test that touches global native-bus registration state
/// should acquire this lock first.
///
/// Tests that only *publish* broadcast events or that construct an
/// isolated [`super::NativeRegistry`] via `NativeRegistry::new()` do NOT
/// need this lock.
pub static BUS_HANDLER_LOCK: TokioMutex<()> = TokioMutex::const_new(());

/// RAII guard for a scoped mock bus session.
///
/// Holds [`BUS_HANDLER_LOCK`] for its entire lifetime and — on drop —
/// runs the caller-supplied `restore` closure so the production handler
/// for the stubbed method is re-registered on the global native registry.
///
/// Construction is private outside this module: tests acquire a guard by
/// calling [`mock_bus_stub`] (or a domain-specific convenience that
/// composes on top of it), which guarantees every guard is paired with
/// exactly one stub installation and that callers cannot forget to
/// restore production handlers.
pub struct MockBusGuard {
    // Held for the guard's lifetime; dropped implicitly after the Drop
    // impl's body runs.
    _lock: TokioMutexGuard<'static, ()>,
    // Option so Drop can move the closure out and call it. Always `Some`
    // until Drop runs.
    restore: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for MockBusGuard {
    fn drop(&mut self) {
        if let Some(restore) = self.restore.take() {
            // The restore closure may itself call `register_native_global`,
            // which is sync and cheap. If a restore closure ever needs to
            // perform async work, this would need to be reworked — but we
            // intentionally keep the surface synchronous so Drop never
            // blocks on an executor that might not exist.
            restore();
        }
    }
}

/// Install a typed stub for `method` on the global native bus, returning a
/// guard that holds [`BUS_HANDLER_LOCK`] and runs `restore` on drop.
///
/// This is the workhorse for every test that needs to intercept a native
/// bus request/response pair across module boundaries. Domain-specific
/// conveniences (e.g.
/// [`crate::openhuman::agent::bus::mock_agent_run_turn`]) should compose
/// on top of this helper by supplying the right method name and a
/// `restore` closure that calls the domain's production registration
/// function.
///
/// The `handler` closure receives the fully-typed request and must return
/// a `Result<Resp, String>` future. Any assertions made inside the closure
/// will run on the dispatching task; panics surface as the test failure
/// they represent.
///
/// # Type parameters
///
/// * `Req` — the request payload type (any `Send + 'static`).
/// * `Resp` — the response payload type (any `Send + 'static`).
/// * `F` — the handler closure type.
/// * `Fut` — the future returned by the handler.
/// * `R` — the restore closure type — called once when the guard drops.
pub async fn mock_bus_stub<Req, Resp, F, Fut, R>(
    method: &'static str,
    handler: F,
    restore: R,
) -> MockBusGuard
where
    Req: Send + 'static,
    Resp: Send + 'static,
    F: Fn(Req) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Resp, String>> + Send + 'static,
    R: FnOnce() + Send + 'static,
{
    let lock = BUS_HANDLER_LOCK.lock().await;
    register_native_global::<Req, Resp, F, Fut>(method, handler);
    MockBusGuard {
        _lock: lock,
        restore: Some(Box::new(restore)),
    }
}
