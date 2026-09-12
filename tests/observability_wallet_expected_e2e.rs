//! End-to-end coverage for the wallet-not-configured reporting decision
//! (#5805, fixed by #5811).
//!
//! An unconfigured wallet is the default state of an **optional** feature, so
//! the condition must never reach Sentry as an error. #5805 measured 55 error
//! events in 72 minutes from one ordinary local session, while a genuine
//! turn-killing failure in the same session emitted nothing — severity
//! inverted in both directions at once.
//!
//! The part that regressed is specifically the **context-wrapped** form.
//! `jsonrpc.rs` already demoted the bare sentinel via an exact-equality
//! predicate, but `hosted/orchestration/schemas.rs` lifts the wallet error
//! into an RPC failure with
//!
//! ```text
//! .map_err(|e| format!("self_identity key_status: {e}"))?
//! ```
//!
//! and exact equality stops matching the moment any caller adds context.
//! `format!("{context}: {e}")` appears ~800 times in `src/`, so this is the
//! common shape, not an exotic one.
//!
//! These tests drive the real reporting entry point
//! [`report_error_or_expected`] and assert on what it actually emits, rather
//! than only on the classifier's return value: the classification is a means,
//! and the observable contract is "this does not page".

use std::io;
use std::sync::{Arc, Mutex};

use openhuman_core::core::observability::{
    expected_error_kind, report_error_or_expected, ExpectedErrorKind,
};
use openhuman_core::openhuman::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE;

/// The exact wrapper `hosted/orchestration/schemas.rs` applies, reproduced from
/// the log line quoted in #5805:
///
/// ```text
/// ERR report_error [observability] rpc.invoke_method failed:
///     self_identity key_status: wallet is not configured; run wallet setup first
/// ```
fn wrapped_like_issue_5805() -> String {
    format!("self_identity key_status: {WALLET_NOT_CONFIGURED_MESSAGE}")
}

// ---------------------------------------------------------------------------
// tracing capture
// ---------------------------------------------------------------------------

/// Collects formatted `tracing` output so a test can assert on the level and
/// fields the reporting path actually emitted.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("capture buffer poisoned")).into_owned()
    }
}

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("capture buffer poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Serializes every test in this file that touches process-global reporting
/// state.
///
/// `with_default` is thread-scoped, so the capture below looked independent —
/// but the `paging` module calls `sentry::init`, which binds a client to the
/// process-global Hub, and `sentry-tracing` is compiled in under
/// `crash-reporting`. With a client bound, a `tracing::error!` raised on
/// *another* thread can be consumed by the Sentry layer instead of reaching
/// this thread's fmt subscriber — so a capture here comes back EMPTY and the
/// assertion fails for a reason that has nothing to do with the behaviour
/// under test.
///
/// That is not hypothetical: it turned up as
/// `reporting_a_genuine_wallet_failure_still_emits_error` failing on
/// "Captured output:" with nothing after it, only in the full-suite lane and
/// only under `crash-reporting` (the gate `paging` is behind). `mod paging`
/// had a lock of its own, which serialized its two tests against each other
/// and against nothing else. One lock for the whole file is what actually
/// closes it.
static REPORTING_STATE_LOCK: Mutex<()> = Mutex::new(());

fn lock_reporting_state() -> std::sync::MutexGuard<'static, ()> {
    REPORTING_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Run `body` with a capturing subscriber installed and return what it logged.
///
/// Holds [`REPORTING_STATE_LOCK`] for the duration: the subscriber is
/// thread-local, but what it is able to observe is not.
fn capture_reporting<F: FnOnce()>(body: F) -> String {
    let _state = lock_reporting_state();
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();

    tracing::subscriber::with_default(subscriber, body);
    capture.contents()
}

// ---------------------------------------------------------------------------
// classification
// ---------------------------------------------------------------------------

/// The regression #5811 fixed: the wrapped form must classify as expected.
///
/// This is the assertion that fails if the `is_wallet_not_configured_message`
/// arm is removed from `expected_error_kind`.
#[test]
fn a_context_wrapped_wallet_state_is_classified_as_expected() {
    let wrapped = wrapped_like_issue_5805();

    assert_eq!(
        expected_error_kind(&wrapped),
        Some(ExpectedErrorKind::WalletNotConfigured),
        "the RPC layer wraps the wallet sentinel as `{{context}}: {{e}}` \
         (#5805); a classifier that only matches the bare message lets this \
         page. Message under test: {wrapped}"
    );
}

/// Wrapping must not have to be single-layer: nothing constrains how many
/// `format!("{context}: {e}")` hops an error takes before it is reported, and
/// a predicate that only tolerated one would regress the moment a caller
/// gained an intermediate layer.
#[test]
fn a_multiply_wrapped_wallet_state_is_still_classified_as_expected() {
    let nested = format!(
        "rpc.invoke_method failed: self_identity key_status: {WALLET_NOT_CONFIGURED_MESSAGE}"
    );

    assert_eq!(
        expected_error_kind(&nested),
        Some(ExpectedErrorKind::WalletNotConfigured),
        "demotion must survive arbitrary nesting depth, not just one wrapper"
    );
}

/// The bare sentinel — the shape a direct RPC produces — must stay demoted too.
#[test]
fn the_bare_wallet_sentinel_is_classified_as_expected() {
    assert_eq!(
        expected_error_kind(WALLET_NOT_CONFIGURED_MESSAGE),
        Some(ExpectedErrorKind::WalletNotConfigured),
    );
}

/// Guard against the matcher being too permissive.
///
/// The predicate is substring-based on purpose, which buys wrapper-tolerance at
/// the cost of blast radius. This pins the other side of that trade: a genuine
/// wallet failure — one that is a defect, not user-state — must still reach
/// Sentry. Without this, a future widening of the needle could silently demote
/// real failures and nothing would fail.
#[test]
fn a_genuine_wallet_failure_is_not_demoted() {
    for genuine in [
        "wallet signing failed: invalid nonce",
        "wallet keychain read failed: keyring access denied",
        "self_identity key_status: wallet is configured but the key is corrupt",
    ] {
        assert_eq!(
            expected_error_kind(genuine),
            None,
            "a real wallet defect must still page; demoting it would hide the \
             failures this classification exists to keep visible. Message: {genuine}"
        );
    }
}

// ---------------------------------------------------------------------------
// the observable reporting decision
// ---------------------------------------------------------------------------

/// The contract #5805 is about: this condition is logged as an expected state,
/// not as an error.
///
/// Asserted on the emitted record rather than on the classifier alone, because
/// "does not page" is the property the issue is about — a classification that
/// did not change what is emitted would fix nothing.
#[test]
fn reporting_a_wrapped_wallet_state_emits_info_and_not_error() {
    let wrapped = wrapped_like_issue_5805();

    let logged = capture_reporting(|| {
        report_error_or_expected(wrapped.as_str(), "rpc", "invoke_method", &[]);
    });

    assert!(
        logged.contains("INFO"),
        "an unconfigured wallet is expected user-state and must be recorded as \
         a breadcrumb at INFO. Captured output:\n{logged}"
    );
    assert!(
        !logged.contains("ERROR"),
        "the wrapped wallet state must not be reported at ERROR — that is the \
         55-events-in-72-minutes behaviour #5805 reported. Captured output:\n{logged}"
    );
    assert!(
        logged.contains("wallet_not_configured"),
        "the demotion should be attributable via its `kind` field so the \
         breadcrumb can still be correlated. Captured output:\n{logged}"
    );
}

/// A genuine wallet defect must still be reported at ERROR.
///
/// The contrast case for the test above: together they pin that the two are
/// routed differently, so a change that demoted everything would fail here
/// rather than passing both.
#[test]
fn reporting_a_genuine_wallet_failure_still_emits_error() {
    let logged = capture_reporting(|| {
        report_error_or_expected(
            "wallet signing failed: invalid nonce",
            "rpc",
            "invoke_method",
            &[],
        );
    });

    assert!(
        logged.contains("ERROR"),
        "a real wallet failure must still page. Captured output:\n{logged}"
    );
}

// ---------------------------------------------------------------------------
// the reporting decision, observed at Sentry
// ---------------------------------------------------------------------------

// The tracing assertions above pin the breadcrumb: its level, and the `kind`
// field that makes a demotion attributable. They do not pin *paging*, and this
// file's whole claim is that the wallet state "does not page".
//
// `report_error_message` reaches Sentry by calling `sentry::with_scope` /
// `capture_message` directly (`src/core/observability.rs`), not through a
// tracing layer, and no Sentry tracing layer is installed here. So a capture
// that started firing for the expected case — or one that stopped firing for a
// genuine failure — would leave the INFO/ERROR text untouched and every
// assertion above would still pass. These two tests close that gap by counting
// envelopes on a `TestTransport`, which is the same wiring
// `tests/observability_smoke.rs` uses.
//
// Gated, not `required-features`: the Sentry capture in `report_error_message`
// is `#[cfg(feature = "crash-reporting")]`, so with the gate off there is
// nothing to observe and both counts would be a vacuous 0. Putting
// `required-features` on the target instead would take the classification tests
// above down with it in every contributor build — the blunt instrument
// `Cargo.toml` warns about on `observability_smoke`. The e2e lane runs this
// target with the product feature set (`scripts/test-rust-e2e.sh` passes
// `product-features.sh`, which includes `crash-reporting`), so these run where
// it counts.
#[cfg(feature = "crash-reporting")]
mod paging {
    use super::*;
    /// Count the Sentry events one call to `report_error_or_expected` produces.
    ///
    /// The client is bound to a **private** hub that is current only inside
    /// [`sentry::Hub::run`]. `sentry::init` instead binds it on the hub every
    /// test thread's hub is copied from, so while a paging test held a client,
    /// a sibling test's `report_error_or_expected` on another thread landed in
    /// this transport too — `left: 2` for a genuine failure, deterministic
    /// under the product feature set, invisible under the contributor default
    /// set where this module does not compile.
    ///
    /// The private hub removes the *client* binding, but it does not remove the
    /// need to serialise. `sentry-tracing`'s layer sits in the global
    /// subscriber stack, so while a client is current on this thread a
    /// `tracing::error!` raised by `capture_reporting` on ANOTHER thread can be
    /// consumed by that layer instead of reaching its fmt subscriber — the
    /// capture then comes back empty and
    /// `reporting_a_genuine_wallet_failure_still_emits_error` fails for a
    /// reason unrelated to the behaviour under test. That is `main`'s
    /// `d0509bb17` finding, and it still holds here: an earlier revision of
    /// this merge dropped the guard on the reasoning that a private hub made it
    /// redundant, and CI reproduced exactly that failure. Both fixes are
    /// needed — the private hub for the paging count, the file-wide lock for
    /// the capture.
    fn captured_events_for(message: &str) -> usize {
        let _guard = lock_reporting_state();
        let transport = sentry::test::TestTransport::new();
        let transport_for_factory = transport.clone();
        let options = sentry::ClientOptions {
            dsn: Some(
                "https://public@sentry.example.com/1"
                    .parse()
                    .expect("dsn parses"),
            ),
            transport: Some(Arc::new(move |_opts: &sentry::ClientOptions| {
                transport_for_factory.clone() as Arc<dyn sentry::Transport>
            })),
            sample_rate: 1.0,
            ..sentry::ClientOptions::default()
        };
        let client = Arc::new(sentry::Client::from_config(sentry::apply_defaults(options)));
        let hub = Arc::new(sentry::Hub::new(
            Some(Arc::clone(&client)),
            Arc::new(sentry::Scope::default()),
        ));
        sentry::Hub::run(hub, || {
            report_error_or_expected(message, "rpc", "invoke_method", &[]);
        });
        client.flush(Some(std::time::Duration::from_secs(2)));
        transport.fetch_and_clear_envelopes().len()
    }

    /// The contract #5805 is about, stated as the thing a user notices:
    /// an unconfigured wallet sends Sentry nothing at all.
    #[test]
    fn a_wrapped_wallet_state_pages_nobody() {
        let wrapped = wrapped_like_issue_5805();

        assert_eq!(
            captured_events_for(&wrapped),
            0,
            "an unconfigured wallet is expected user-state; reaching Sentry at \
             all is the 55-events-in-72-minutes behaviour #5805 reported. \
             Message under test: {wrapped}"
        );
    }

    /// The contrast case. Without it, a change that demoted *everything* would
    /// satisfy the test above and this file would be pinning silence rather
    /// than a decision.
    #[test]
    fn a_genuine_wallet_failure_still_pages() {
        assert_eq!(
            captured_events_for("wallet signing failed: invalid nonce"),
            1,
            "a real wallet defect must still reach Sentry; demotion that \
             swallowed it would hide exactly the failures #5805 found were \
             already invisible"
        );
    }
}
