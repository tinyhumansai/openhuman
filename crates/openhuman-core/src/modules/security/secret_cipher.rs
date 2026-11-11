//! Audited host orchestration for TinySecurity's routine config cipher bus.

use zeroize::{Zeroize, Zeroizing};

use super::{cipher_consumer::Consumer, Client, SecurityError};
use crate::security::approval::native::SecretScope;
use tinysecurity_bus::{
    EvaluateSecretCipherRequest2, ExecuteSecretCipherRequest2, SecretCipherKind2,
    SecretCipherRequest2, SecretCipherResponse2,
};

const OPERATION_TTL_MS: u32 = 30_000;
const MASTER_GRANT_TOOL: &str = "native_secret_grant";
const MASTER_GRANT_SUMMARY: &str = "Unlock configuration secrets";

/// Ask through the installed approval gate, then consume TinySecurity's exact
/// HumanOnce grant to obtain a short-lived process-private User master handle.
///
/// `config` decides who answers the grant: a person, or this host for the
/// operator when `[secrets] unattended_unlock` holds
/// ([`super::unattended_unlock`]).
pub(crate) async fn resolve_user_master(
    config: &crate::config::Config,
    client: &Client,
    scope: &SecretScope,
    registration_id: &str,
) -> Result<String, SecurityError> {
    resolve_user_master_inner(config, client, scope, registration_id, None).await
}

/// Resolve the module's User master using one explicitly supplied existing key.
/// The callback can answer one target read and denies every Set during this
/// invocation, preserving read-only inspection semantics.
pub(crate) async fn resolve_user_master_with_existing_key(
    config: &crate::config::Config,
    client: &Client,
    scope: &SecretScope,
    registration_id: &str,
    backend: &super::secret_backend::SecretBackend,
    encoded_key: &str,
) -> Result<String, SecurityError> {
    resolve_user_master_inner(
        config,
        client,
        scope,
        registration_id,
        Some((backend, encoded_key)),
    )
    .await
}

async fn resolve_user_master_inner(
    config: &crate::config::Config,
    client: &Client,
    scope: &SecretScope,
    registration_id: &str,
    existing_key: Option<(&super::secret_backend::SecretBackend, &str)>,
) -> Result<String, SecurityError> {
    #[cfg(any(test, feature = "e2e-test-support", feature = "test-keyring-consent"))]
    let answered = super::test_host::answer_master_grant(MASTER_GRANT_TOOL, registration_id).await;
    #[cfg(not(any(test, feature = "e2e-test-support", feature = "test-keyring-consent")))]
    let answered = None;
    let outcome = match answered {
        Some(outcome) => outcome,
        None => {
            use super::unattended_unlock::{answer_master_grant, mode_for, UnlockMode};
            let gate = crate::security::approval::ApprovalGate::try_global()
                .ok_or_else(|| SecurityError::Unavailable("secret approval unavailable".into()))?;
            match mode_for(config) {
                UnlockMode::Unattended => {
                    answer_master_grant(
                        &gate,
                        MASTER_GRANT_TOOL,
                        MASTER_GRANT_SUMMARY,
                        registration_id,
                    )
                    .await
                }
                mode => {
                    if mode == UnlockMode::IgnoredOnSaas {
                        tracing::warn!(
                            "[security::unattended_unlock] secrets.unattended_unlock is set but \
                             this is a SaaS core; the user must approve over WebChat, and an unlock \
                             with no WebChat route is refused"
                        );
                    }
                    if mode == UnlockMode::IgnoredWithoutOperatorKey {
                        tracing::warn!(
                            "[security::unattended_unlock] secrets.unattended_unlock is set but the \
                             keyring master key was not supplied by the operator \
                             (OPENHUMAN_KEYRING_MASTER_KEY / OPENHUMAN_KEYRING_MASTER_KEY_FILE on a \
                             file or encrypted_file keyring); asking a person instead"
                        );
                    }
                    gate.intercept_forced(
                        MASTER_GRANT_TOOL,
                        MASTER_GRANT_SUMMARY,
                        serde_json::json!({"registration_id": registration_id}),
                    )
                    .await
                }
            }
        }
    };
    if let crate::security::approval::GateOutcome::Deny { reason } = outcome {
        crate::security::approval::native::forget_secret_grant(registration_id);
        tracing::warn!(
            reason = %reason,
            "[security::secret_cipher] secret master grant refused"
        );
        return Err(master_grant_refusal(&reason));
    }
    let (request, request_id) =
        crate::security::approval::native::take_secret_grant(registration_id).ok_or_else(|| {
            SecurityError::Unavailable("secret master approval unavailable".into())
        })?;
    let _ordered = scope.lock().await;
    let _existing_read = existing_key
        .map(|(backend, encoded)| {
            backend
                .arm_existing_master_read(encoded)
                .map_err(|_| SecurityError::Unavailable("existing secret key unavailable".into()))
        })
        .transpose()?;
    let generation = client.policy_info().await?.generation;
    let audit = scope
        .correlation(generation)
        .await
        .map_err(SecurityError::Unavailable)?;
    let mut execute = tinysecurity_bus::ExecuteSecretRequest2 {
        request,
        request_id,
        audit,
    };
    match client.execute_secret2(&mut execute).await? {
        tinysecurity_bus::SecretResponse2::MasterHandle {
            handle,
            purpose: tinysecurity_bus::MasterPurpose2::ConfigUserMaster,
            generation: returned_generation,
        } if returned_generation == generation => Ok(handle),
        _ => Err(SecurityError::Unavailable(
            "secret master resolution refused".into(),
        )),
    }
}

/// Why the secret master was not unlocked, from the gate's denial reason: a
/// person's refusal, an unanswered prompt, or the gate's own reason (its
/// agent-facing marker removed) when it never asked.
fn master_grant_refusal(reason: &str) -> SecurityError {
    const WHAT: &str = "the approval to unlock configuration secrets";
    let message = if crate::security::approval::is_unanswered_approval_reason(reason) {
        format!("{WHAT} was not answered in time")
    } else if reason.contains("User denied") {
        format!("{WHAT} was denied")
    } else {
        let reason = reason
            .trim_start()
            .strip_prefix(tinytools::status::POLICY_DENIED_MARKER)
            .unwrap_or(reason)
            .trim();
        format!("{WHAT} was refused: {reason}")
    };
    SecurityError::ApprovalRefused(message)
}

/// Successful result of the native routine config cipher.
pub(crate) enum Output {
    Ciphertext(String),
    Plaintext {
        value: Zeroizing<String>,
        migrated: bool,
        replacement_ciphertext: Option<String>,
    },
}

struct WipingRequest(SecretCipherRequest2);
impl Drop for WipingRequest {
    fn drop(&mut self) {
        self.0.input.zeroize();
    }
}

struct WipingEvaluate(EvaluateSecretCipherRequest2);
impl Drop for WipingEvaluate {
    fn drop(&mut self) {
        self.0.request.input.zeroize();
    }
}

struct WipingExecute(ExecuteSecretCipherRequest2);
impl Drop for WipingExecute {
    fn drop(&mut self) {
        self.0.request.input.zeroize();
    }
}

/// Perform the intent and execution phases on the registered subject's ordered
/// audit stream. Decrypted output is released only by the exact module-bound
/// consumer after the matching successful `ExecuteSecretCipher2` response.
pub(crate) async fn run(
    client: &Client,
    scope: &SecretScope,
    consumer: &Consumer,
    registration_id: &str,
    master_handle: &str,
    operation: SecretCipherKind2,
    input: &[u8],
) -> Result<Output, SecurityError> {
    let info = client.policy_info().await?;
    let _stream = scope.lock().await;
    let mut correlations = scope
        .correlations(info.generation, 4)
        .await
        .map_err(SecurityError::Unavailable)?
        .into_iter();
    let mut request = WipingRequest(SecretCipherRequest2 {
        registration_id: registration_id.to_owned(),
        master_handle: master_handle.to_owned(),
        call_id: uuid::Uuid::new_v4().simple().to_string(),
        operation,
        input: input.to_vec(),
        ttl_ms: OPERATION_TTL_MS,
    });
    let mut evaluate = WipingEvaluate(EvaluateSecretCipherRequest2 {
        request: request.0.clone(),
        audit: correlations
            .next()
            .ok_or_else(|| SecurityError::Unavailable("secret audit unavailable".into()))?,
    });
    let pending = client.evaluate_secret_cipher2(&mut evaluate.0).await?;
    let execute_request = std::mem::replace(
        &mut request.0,
        SecretCipherRequest2 {
            registration_id: String::new(),
            master_handle: String::new(),
            call_id: String::new(),
            operation,
            input: Vec::new(),
            ttl_ms: 0,
        },
    );
    let mut execute = WipingExecute(ExecuteSecretCipherRequest2 {
        request: execute_request,
        pending_id: pending.pending_id,
        audit: correlations
            .next()
            .ok_or_else(|| SecurityError::Unavailable("secret audit unavailable".into()))?,
        delivery_audit: correlations
            .next()
            .ok_or_else(|| SecurityError::Unavailable("secret audit unavailable".into()))?,
        completion_audit: correlations
            .next()
            .ok_or_else(|| SecurityError::Unavailable("secret audit unavailable".into()))?,
    });
    match client.execute_secret_cipher2(&mut execute.0).await? {
        SecretCipherResponse2::Ciphertext { ciphertext } => Ok(Output::Ciphertext(ciphertext)),
        SecretCipherResponse2::Delivered {
            identity,
            migrated,
            replacement_ciphertext,
        } => {
            let (plaintext, _replacement) = consumer.take(&identity).map_err(|_| {
                SecurityError::Unavailable("secret cipher output unavailable".into())
            })?;
            Ok(Output::Plaintext {
                value: plaintext,
                migrated,
                replacement_ciphertext,
            })
        }
    }
}

#[cfg(test)]
#[path = "secret_cipher_tests.rs"]
mod tests;
