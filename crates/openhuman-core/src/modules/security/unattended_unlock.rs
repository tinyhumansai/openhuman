//! Operator-authorized unattended unlock of the config secret master.
//!
//! Every config load or save that touches an encrypted field needs the User
//! master, and TinySecurity releases it only against a one-time `HumanOnce`
//! grant ("Unlock configuration secrets"). On a desktop a person answers it. A
//! headless `openhuman-core serve` host has nobody to ask, so the grant parks
//! until it expires and every encrypted field becomes unreadable.
//!
//! An operator can opt such a host out of the prompt with
//! `[secrets] unattended_unlock = true` (or
//! `OPENHUMAN_SECRETS_UNATTENDED_UNLOCK=1`). The setting is honoured only when
//! the operator also supplied the keyring master key
//! (`OPENHUMAN_KEYRING_MASTER_KEY` / `OPENHUMAN_KEYRING_MASTER_KEY_FILE`) to a
//! `file` or `encrypted_file` keyring: whoever can read that key can already
//! decrypt the keyring, so a person approving each unlock adds nothing there.
//! The host then answers the grant itself, through the same gate and the same
//! one-time TinySecurity grant a person's approval produces; nothing about the
//! grant, its audit row or the cipher changes. The decision rests on where the
//! key came from, not on what kind of host this is: a desktop with its usual
//! key (OS keychain, or one the app minted itself) never takes this path,
//! whatever the config says; any host whose operator exports a valid key and
//! sets the option does, which is the operator's explicit choice.

use std::collections::HashSet;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::core::runtime::turn_origin::AgentTurnOrigin;
use crate::core::runtime::DispatchCtx;
use crate::security::approval::{ApprovalDecision, ApprovalGate, GateOutcome};
use crate::security::secret_storage::store::BackendKind;

const LOG: &str = "[security::unattended_unlock]";
/// How often the parked grant is looked for. The park is the host's own and
/// appears within a few engine round trips.
const POLL: Duration = Duration::from_millis(10);

/// How this host answers the config secret master grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnlockMode {
    /// A person approves it (the default, and every host on its own key).
    Interactive,
    /// The host approves it for the operator.
    Unattended,
    /// `unattended_unlock` is on but the keyring key is not the operator's,
    /// so the setting is ignored and a person approves it.
    IgnoredWithoutOperatorKey,
    /// `unattended_unlock` is on but this is a SaaS core, whose gate refuses
    /// a forced approval without an authenticated WebChat route before it can
    /// park; the setting is ignored and the user approves it in chat.
    IgnoredOnSaas,
}

/// The pure decision: the setting, whether the operator supplied the keyring
/// master key, and the keyring backend in use.
pub(crate) fn unlock_mode(
    unattended_unlock: bool,
    saas: bool,
    operator_key_supplied: bool,
    backend: BackendKind,
) -> UnlockMode {
    if !unattended_unlock {
        return UnlockMode::Interactive;
    }
    if saas {
        return UnlockMode::IgnoredOnSaas;
    }
    let file_keyring = matches!(backend, BackendKind::File | BackendKind::EncryptedFile);
    if operator_key_supplied && file_keyring {
        UnlockMode::Unattended
    } else {
        UnlockMode::IgnoredWithoutOperatorKey
    }
}

/// [`unlock_mode`] for `config` on this process.
///
/// The setting is read with the environment override applied, because some
/// callers hold a configuration read straight from the file.
pub(crate) fn mode_for(config: &Config) -> UnlockMode {
    if !config.secrets.effective_unattended_unlock() {
        return UnlockMode::Interactive;
    }
    #[cfg(test)]
    if ASSUME_OPERATOR_KEY.load(Ordering::SeqCst) > 0 {
        return UnlockMode::Unattended;
    }
    // A key variable that is set but unusable (missing file, malformed key)
    // is not an operator key: the file backend never validates it at startup.
    if crate::core::runtime::is_saas() {
        return UnlockMode::IgnoredOnSaas;
    }
    unlock_mode(
        true,
        false,
        crate::security::secret_storage::encrypted_file_backend::key_source::operator_master_key_supplied(),
        crate::security::secret_storage::store::effective_backend_kind(),
    )
}

/// Test seam: treat the keyring key as operator-supplied while held, so a test
/// need not change the process-wide keyring backend. It matters only to a
/// config that also sets `unattended_unlock`.
#[cfg(test)]
static ASSUME_OPERATOR_KEY: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) struct AssumeOperatorKeyGuard;

#[cfg(test)]
impl Drop for AssumeOperatorKeyGuard {
    fn drop(&mut self) {
        ASSUME_OPERATOR_KEY.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn assume_operator_key() -> AssumeOperatorKeyGuard {
    ASSUME_OPERATOR_KEY.fetch_add(1, Ordering::SeqCst);
    AssumeOperatorKeyGuard
}

/// Ask for the grant through `gate` and approve it once on the operator's
/// behalf.
///
/// The request is raised outside whatever turn triggered the config load (a
/// channel or scheduled turn would otherwise be refused before the grant is
/// even created), so it parks exactly like a Settings save with no chat route,
/// and this host decides that park with an ordinary one-time approval. Only
/// the park this call raised is answered: the gate stamps it with a fresh
/// correlation id, and any other pending grant is left for its own owner. The
/// outcome is the gate's: a failed engine or an expired park still denies.
/// Dropping the returned future (a config-load timeout, a cancelled caller)
/// aborts the request, so the gate's cancel guard withdraws its park.
pub(crate) async fn answer_master_grant(
    gate: &Arc<ApprovalGate>,
    tool: &'static str,
    summary: &'static str,
    registration_id: &str,
) -> GateOutcome {
    tracing::info!(
        tool,
        "{LOG} unattended unlock: answering the config secret grant for the operator \
         (secrets.unattended_unlock with an operator-supplied keyring master key)"
    );
    let dispatch = DispatchCtx::ambient().with_turn_origin(AgentTurnOrigin::Unknown);
    let asking = Arc::clone(gate);
    let args = serde_json::json!({ "registration_id": registration_id });
    let correlation = format!("unattended-unlock-{}", uuid::Uuid::new_v4().simple());
    let asked_as = correlation.clone();
    let mut asked = tokio_util::task::AbortOnDropHandle::new(dispatch.spawn(async move {
        asking
            .intercept_forced_correlated(tool, summary, args, &asked_as)
            .await
    }));
    let mut decided = HashSet::new();
    loop {
        tokio::select! {
            outcome = &mut asked => {
                return match outcome {
                    Ok(outcome) => {
                        match &outcome {
                            GateOutcome::Allow => tracing::info!(
                                tool,
                                "{LOG} unattended unlock: config secret grant approved"
                            ),
                            GateOutcome::Deny { reason } => tracing::warn!(
                                tool,
                                %reason,
                                "{LOG} unattended unlock: config secret grant refused"
                            ),
                        }
                        outcome
                    }
                    Err(error) => {
                        tracing::warn!(
                            tool,
                            %error,
                            "{LOG} unattended unlock: the grant request did not finish"
                        );
                        GateOutcome::Deny {
                            reason: format!(
                                "{} unattended secret unlock did not finish",
                                tinytools::status::POLICY_DENIED_MARKER
                            ),
                        }
                    }
                };
            }
            () = tokio::time::sleep(POLL) => {
                for pending in gate.list_pending().unwrap_or_default() {
                    if !is_our_park(&pending, tool, &correlation)
                        || !decided.insert(pending.request_id.clone())
                    {
                        continue;
                    }
                    tracing::info!(
                        tool,
                        request_id = %pending.request_id,
                        "{LOG} unattended unlock: approving the parked grant once"
                    );
                    // `decide` blocks on the engine, which may need this very
                    // worker on a small runtime.
                    let deciding = Arc::clone(gate);
                    let request_id = pending.request_id.clone();
                    let result = dispatch
                        .spawn_blocking(move || {
                            deciding.decide(&request_id, ApprovalDecision::ApproveOnce)
                        })
                        .await;
                    match result {
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => tracing::warn!(
                            tool,
                            request_id = %pending.request_id,
                            %error,
                            "{LOG} unattended unlock: the approval was not applied"
                        ),
                        Err(error) => tracing::warn!(
                            tool,
                            request_id = %pending.request_id,
                            %error,
                            "{LOG} unattended unlock: the approval task failed"
                        ),
                    }
                }
            }
        }
    }
}

/// Whether `pending` is the park this call raised: the grant's tool, stamped
/// with this call's correlation id. Nothing else is answered, so a grant
/// another configuration raised (which may not allow unattended unlock) always
/// waits for its own person.
fn is_our_park(
    pending: &crate::security::approval::PendingApproval,
    tool: &str,
    correlation: &str,
) -> bool {
    pending.tool_name == tool && pending.tool_call_id.as_deref() == Some(correlation)
}

#[cfg(test)]
#[path = "unattended_unlock_tests.rs"]
mod tests;
