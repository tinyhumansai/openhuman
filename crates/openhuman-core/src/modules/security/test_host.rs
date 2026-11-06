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

fn active() -> bool {
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
                Ok(Ok(())) => Some(config),
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
    let config = super::text::armed_config().ok()?;
    let gate = ApprovalGate::try_global()
        .unwrap_or_else(|| Arc::clone(GATE.get_or_init(|| ApprovalGate::for_test_host(config))));
    crate::core::bus::init().await.ok()?;
    let dispatch = DispatchCtx::ambient().with_turn_origin(AgentTurnOrigin::WebChat {
        thread_id: "test-host".into(),
        client_id: "test-host".into(),
        request_id: None,
    });
    eprintln!("DBG grant start");
    let asking = Arc::clone(&gate);
    let args = serde_json::json!({"registration_id": registration_id});
    let mut asked = dispatch.spawn(async move {
        asking
            .intercept_forced(tool, "Unlock configuration secrets", args)
            .await
    });
    let answered = tokio::time::timeout(GRANT_TIMEOUT, async {
        loop {
            tokio::select! {
                outcome = &mut asked => { eprintln!("DBG asked done {:?}", outcome.as_ref().map(|o| format!("{o:?}"))); break outcome.ok() },
                () = tokio::time::sleep(Duration::from_millis(5)) => {
                    let rows = gate.list_pending().unwrap_or_default();
                    eprintln!("DBG pending {}", rows.len());
                    for pending in rows {
                        if pending.tool_name == tool {
                            let d = gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce);
                            eprintln!("DBG decided {:?}", d.is_ok());
                        }
                    }
                }
            }
        }
    })
    .await;
    answered.ok().flatten()
}

/// Give a policy the answers the async host gate caches before a tool runs, for
/// a test that calls a synchronous adapter with no host gate in front of it.
pub(crate) fn prime_policy(policy: &crate::security::SecurityPolicy) {
    use crate::security::policy::CommandClass;

    if !active() || policy.budget_snapshot.lock().map_or(true, |seen| seen.is_some()) {
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
