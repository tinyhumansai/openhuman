//! What a booted host provides for TinySecurity, for test processes that boot none.
//!
//! A host's boot loads TinySecurity, arms its synchronous adapters and installs
//! the approval gate, and a person answers the master-key prompt. A unit or
//! in-process test has none of that, so every fail-closed check refuses for a
//! reason unrelated to the test. This module supplies those pieces against the
//! real, checksum-pinned module: redaction, screening and ciphers are the
//! module's own answers. It is compiled only into unit-test, `e2e-test-support`
//! and `test-keyring-consent` builds, never a product build.
//!
//! It acts only when `OPENHUMAN_TEST_MODULES_STAGED=1` says the runner staged
//! the modules (so a test never downloads one) and
//! `OPENHUMAN_TEST_KEYRING_CONSENT` answers approve (the default in unit tests;
//! see `keyring_consent/test_responder.rs`). Otherwise the production path runs
//! and fails closed as it always does.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::config::Config;
use crate::core::runtime::turn_origin::AgentTurnOrigin;
use crate::core::runtime::DispatchCtx;
use crate::security::approval::{ApprovalDecision, ApprovalGate, GateOutcome};

const BOOT_TIMEOUT: Duration = Duration::from_secs(60);
const GRANT_TIMEOUT: Duration = Duration::from_secs(30);

static FIXTURE: OnceLock<Option<Config>> = OnceLock::new();
static FIXTURE_KEY: OnceLock<PathBuf> = OnceLock::new();
static GATE: OnceLock<Arc<ApprovalGate>> = OnceLock::new();

#[cfg(test)]
thread_local! {
    static ABSENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Keeps this thread on the production path until dropped.
#[cfg(test)]
pub(crate) struct AbsentGuard;

#[cfg(test)]
impl Drop for AbsentGuard {
    fn drop(&mut self) {
        ABSENT.with(|absent| absent.set(false));
    }
}

/// For a test that pins what the host does when TinySecurity cannot answer: no
/// test host on this thread, so the module stays exactly as absent as it is in
/// a process that never booted one.
#[cfg(test)]
pub(crate) fn absent() -> AbsentGuard {
    ABSENT.with(|absent| absent.set(true));
    AbsentGuard
}

/// Fixtures driving the gate themselves, so the test host must not answer too.
#[cfg(test)]
static EXPLICIT_CONSENT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Leaves the master grant to the caller's own approval until dropped.
#[cfg(test)]
pub(crate) struct ExplicitConsentGuard;

#[cfg(test)]
impl Drop for ExplicitConsentGuard {
    fn drop(&mut self) {
        EXPLICIT_CONSENT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// See [`ExplicitConsentGuard`].
#[cfg(test)]
pub(crate) fn explicit_consent() -> ExplicitConsentGuard {
    EXPLICIT_CONSENT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    ExplicitConsentGuard
}

/// Whether this process has a test host, and it applies to this thread.
pub(super) fn active() -> bool {
    #[cfg(test)]
    if ABSENT.with(std::cell::Cell::get) {
        return false;
    }
    std::env::var("OPENHUMAN_TEST_MODULES_STAGED").as_deref() == Ok("1")
        && crate::security::keyring_consent::test_responder::configured_answer() == Some(true)
}

/// Whether `key_path` is the scope this module admitted for its fixture.
pub(super) fn is_fixture_scope(key_path: &Path) -> bool {
    FIXTURE_KEY.get().is_some_and(|fixture| fixture == key_path)
}

extern "C" fn remove_fixture() {
    if let Some(dir) = FIXTURE_KEY.get().and_then(|key| key.parent()) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn fixture_config() -> Option<Config> {
    let dir = std::env::temp_dir().join(format!("oh-security-host-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("workspace")).ok()?;
    let _ = FIXTURE_KEY.set(dir.join(".secret_key"));
    // SAFETY: registers a function with no captured state to run at exit.
    unsafe { libc::atexit(remove_fixture) };
    Some(Config {
        config_path: dir.join("config.toml"),
        workspace_dir: dir.join("workspace"),
        action_dir: dir.join("workspace"),
        ..Config::default()
    })
}

/// The configuration a caller with none of its own screens through.
///
/// TinySecurity serves one secret scope at a time, so once a runtime has
/// admitted the module (an `auth_store_session`, an embedder's boot) the
/// caller must present that scope: the module refuses the test host's private
/// fixture scope while another is live, and every screen would be held as
/// `screen.unavailable`. The fixture is for a process nothing else admitted
/// the module in.
pub(crate) fn screening_config() -> Option<Config> {
    if !active() {
        return None;
    }
    super::text::armed().or_else(boot)
}

/// The configuration the module is live under, for a caller whose own
/// configuration names a different secret scope.
///
/// A production caller always presents the host's own configuration. A fixture
/// that builds one of its own (a gate over a bare `Config`) would be refused
/// the module while the runtime's scope is live, and its fail-closed paths
/// (argument redaction, the audit write) would deny every call for a reason
/// unrelated to the test. `None` leaves the request as it is: no test host, no
/// scope admitted yet, or the same scope already.
#[cfg(not(test))]
pub(crate) fn live_scope_config(requested: &Config) -> Option<Config> {
    if !active() {
        return None;
    }
    let armed = super::text::armed()?;
    let key_path = |config: &Config| {
        config
            .config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".secret_key")
    };
    (key_path(&armed) != key_path(requested) && !is_fixture_scope(&key_path(&armed)))
        .then_some(armed)
}

/// Load TinySecurity once for this process, as a host's boot does, and return
/// the configuration it was admitted with. Blocks on the module bus, which is
/// never the caller's runtime, so a single-threaded test runtime is safe.
pub(crate) fn boot() -> Option<Config> {
    if !active() {
        return None;
    }
    FIXTURE
        .get_or_init(|| {
            let config = fixture_config()?;
            let admitted = config.clone();
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            crate::modules::runtime::spawn_on_bus(async move {
                let _ = tx.send(super::ensure_loaded(&admitted).await);
            })
            .ok()?;
            match rx.recv_timeout(BOOT_TIMEOUT) {
                Ok(Ok(())) => {
                    // A host's boot arms the synchronous bridge too. Without it every
                    // sync seam (log and output redaction, privacy and provider-host
                    // checks) answers "runtime unavailable" and fails closed, so a test
                    // asking the real module for a redaction got `[REDACTED]` back.
                    if !super::sync_seam::arm_current() {
                        log::warn!("[security][test-host] sync bridge did not arm");
                    }
                    Some(config)
                }
                outcome => {
                    log::warn!("[security][test-host] boot failed: {outcome:?}");
                    None
                }
            }
        })
        .clone()
}

/// Answer TinySecurity's HumanOnce master grant the way the person at a booted
/// host does: through the real gate, on a WebChat turn, with an ordinary
/// approve. `None` leaves the prompt to the production path.
pub(super) async fn answer_master_grant(
    tool: &'static str,
    registration_id: &str,
) -> Option<GateOutcome> {
    if !active() {
        return None;
    }
    #[cfg(test)]
    if EXPLICIT_CONSENT.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        return None;
    }
    let config = super::text::armed_config().ok()?;
    let gate = ApprovalGate::try_global()
        .unwrap_or_else(|| Arc::clone(GATE.get_or_init(|| ApprovalGate::for_test_host(config))));
    crate::core::bus::init().await.ok()?;
    let dispatch = DispatchCtx::ambient().with_turn_origin(AgentTurnOrigin::WebChat {
        thread_id: "test-host".into(),
        client_id: "test-host".into(),
        request_id: None,
    });
    let asking = Arc::clone(&gate);
    let args = serde_json::json!({"registration_id": registration_id});
    let mut asked = dispatch.spawn(async move {
        asking
            .intercept_forced(tool, "Unlock configuration secrets", args)
            .await
    });
    // Wall clock, not Tokio's: a test's paused clock would expire any timer here.
    let deadline = std::time::Instant::now() + GRANT_TIMEOUT;
    while std::time::Instant::now() < deadline {
        tokio::select! {
            outcome = &mut asked => return outcome.ok(),
            () = tokio::time::sleep(Duration::from_millis(5)) => {
                for pending in gate.list_pending().unwrap_or_default() {
                    if pending.tool_name != tool {
                        continue;
                    }
                    // `decide` blocks on the engine, whose work may need this
                    // very thread on a single-threaded test runtime.
                    let deciding = Arc::clone(&gate);
                    let _ = dispatch
                        .spawn_blocking(move || {
                            deciding.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
                        })
                        .await;
                }
            }
        }
    }
    asked.abort();
    None
}

/// Give a policy the answers the async host gate caches before a tool runs, for
/// a test that calls a synchronous adapter with no host gate in front of it.
pub(crate) fn prime_policy(policy: &crate::security::SecurityPolicy) {
    use crate::security::policy::CommandClass;

    if !active()
        || policy
            .budget_snapshot
            .lock()
            .map_or(true, |seen| seen.is_some())
    {
        return;
    }
    #[cfg(test)]
    if super::gap::budget_port_installed() {
        return;
    }
    if boot().is_none() {
        return;
    }
    let policy = policy.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let task = crate::modules::runtime::spawn_on_bus(async move {
        let budget = policy.peek_action_budget().await;
        if let Err(error) = &budget {
            log::warn!("[security][test-host] budget peek failed: {error}");
        }
        for class in [
            CommandClass::Read,
            CommandClass::Write,
            CommandClass::Network,
            CommandClass::Install,
            CommandClass::Destructive,
        ] {
            let _ = policy.command_class_gate(class).await;
        }
        let _ = tx.send(());
    });
    if task.is_ok() {
        let _ = rx.recv_timeout(BOOT_TIMEOUT);
    }
}
