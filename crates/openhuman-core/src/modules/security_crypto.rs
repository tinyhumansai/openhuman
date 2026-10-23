//! Host adapter for the TinySecurity confidential password, keyring and pairing
//! members (contract 1.16).
//!
//! Only the bus contract is linked. Every call is confidential, bound by the
//! module to the verified `trusted_host`, and fails closed: any transport,
//! attestation or decode failure is an error, never a fallback to host crypto.
//! Messages never carry secret material: errors are fixed strings.

use tinysecurity_bus::{
    names, CallerContext, KeyringAccess2, KeyringBackend2, KeyringConsentRequest2,
    KeyringConsentResponse2, KeyringOperation2, KeyringRequest2, KeyringResponse2,
    PairingOperation2, PairingRequest2, PairingResponse2, PasswordOperation2, PasswordPayload2,
    PasswordRequest2, PasswordResponse2,
};

use crate::security::keyring_consent::types::PolicyDecision;

const LOG_PREFIX: &str = "[security-crypto]";
#[cfg(test)]
pub(crate) static KEYRING_CONSENT_TEST_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());
/// Argon2id derivation (64 MiB, 3 passes) can queue behind two other
/// derivations and a cold start, so password calls outlive the 2 s policy budget.
const PASSWORD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Legacy `EncryptedPayload` shape accepted by `encryption.*` RPC callers.
pub(crate) type LegacyPayload = PasswordPayload2;

/// Seals `plaintext` under `password` through `PasswordCrypt2`. A supplied
/// `salt` selects the legacy separate-salt layout.
pub(crate) async fn password_encrypt(
    proxy: &tinybus::Proxy,
    password: String,
    plaintext: Vec<u8>,
    salt: Option<Vec<u8>>,
) -> Result<LegacyPayload, String> {
    let response = password_call(
        proxy,
        PasswordOperation2::Encrypt {
            password,
            plaintext,
            salt,
        },
    )
    .await?;
    response
        .payload
        .ok_or_else(|| "TinySecurity PasswordCrypt2 returned no payload".to_string())
}

/// Opens a legacy payload through `PasswordCrypt2`.
pub(crate) async fn password_decrypt(
    proxy: &tinybus::Proxy,
    password: String,
    payload: LegacyPayload,
) -> Result<Vec<u8>, String> {
    Ok(
        password_call(proxy, PasswordOperation2::Decrypt { password, payload })
            .await?
            .bytes,
    )
}

async fn password_call(
    proxy: &tinybus::Proxy,
    operation: PasswordOperation2,
) -> Result<PasswordResponse2, String> {
    log::debug!("{LOG_PREFIX} PasswordCrypt2 call");
    proxy
        .clone()
        .with_timeout(PASSWORD_TIMEOUT)
        .call_confidential(
            names::methods::PASSWORD_CRYPT2,
            (PasswordRequest2 { operation },),
        )
        .await
        .map_err(|_| "TinySecurity PasswordCrypt2 failed".to_string())
}

/// Reads a named secret; `None` only when the backend confirms absence.
pub(crate) async fn keyring_get(
    proxy: &tinybus::Proxy,
    caller: CallerContext,
    purpose: &str,
    name: &str,
) -> Result<Option<Vec<u8>>, String> {
    let response = keyring_call(
        proxy,
        caller,
        purpose,
        KeyringOperation2::Get {
            name: name.to_owned(),
        },
    )
    .await?;
    Ok(response.found.then_some(response.bytes))
}

/// Stores a named secret.
pub(crate) async fn keyring_set(
    proxy: &tinybus::Proxy,
    caller: CallerContext,
    purpose: &str,
    name: &str,
    value: Vec<u8>,
) -> Result<(), String> {
    keyring_call(
        proxy,
        caller,
        purpose,
        KeyringOperation2::Set {
            name: name.to_owned(),
            value,
        },
    )
    .await
    .map(|_| ())
}

/// Removes a named secret; returns whether one existed.
pub(crate) async fn keyring_delete(
    proxy: &tinybus::Proxy,
    caller: CallerContext,
    purpose: &str,
    name: &str,
) -> Result<bool, String> {
    Ok(keyring_call(
        proxy,
        caller,
        purpose,
        KeyringOperation2::Delete {
            name: name.to_owned(),
        },
    )
    .await?
    .found)
}

async fn keyring_call(
    proxy: &tinybus::Proxy,
    caller: CallerContext,
    purpose: &str,
    operation: KeyringOperation2,
) -> Result<KeyringResponse2, String> {
    log::debug!("{LOG_PREFIX} KeyringAccess2 call");
    proxy
        .call_confidential(
            names::methods::KEYRING_ACCESS2,
            (KeyringRequest2 {
                caller,
                purpose: purpose.to_owned(),
                operation,
            },),
        )
        .await
        .map_err(|_| "TinySecurity KeyringAccess2 failed".to_string())
}

/// One pairing operation on an account registered at module init.
pub(crate) async fn pairing_call(
    proxy: &tinybus::Proxy,
    account: &str,
    operation: PairingOperation2,
) -> Result<PairingResponse2, String> {
    log::debug!("{LOG_PREFIX} PairingControl2 call");
    proxy
        .call_confidential(
            names::methods::PAIRING_CONTROL2,
            (PairingRequest2 {
                account: account.to_owned(),
                operation,
            },),
        )
        .await
        .map_err(|_| "TinySecurity PairingControl2 failed".to_string())
}

fn backend_kind(backend: &KeyringBackend2) -> &'static str {
    match backend {
        KeyringBackend2::Os => "os",
        KeyringBackend2::LinuxKernel => "linux_kernel",
        KeyringBackend2::SecretService => "secret_service",
        KeyringBackend2::EncryptedFile { .. } => "encrypted_file",
    }
}

/// What consent a request needs, from the backend it will touch.
///
/// A platform credential store is the user's own keychain: the platform guards
/// it and the user chose it by installing the product. The encrypted-file store
/// is the local fallback the user must explicitly agree to, so only the
/// recorded answer counts, never whether the OS keyring happens to be
/// available. No configured backend refuses.
pub(crate) fn consent_decision(
    backend: Option<&KeyringBackend2>,
    recorded: impl FnOnce() -> PolicyDecision,
) -> PolicyDecision {
    match backend {
        None => PolicyDecision::Declined,
        Some(KeyringBackend2::EncryptedFile { .. }) => recorded(),
        Some(
            KeyringBackend2::Os | KeyringBackend2::LinuxKernel | KeyringBackend2::SecretService,
        ) => PolicyDecision::Proceed,
    }
}

/// Decides one `KeyringConsent2.Consent` request. Consent UX stays host-side:
/// when the encrypted-file store is requested and no answer was recorded, the
/// existing consent-required event is published and this request is denied.
pub(crate) fn keyring_consent_response(
    request: &KeyringConsentRequest2,
    decision: PolicyDecision,
) -> KeyringConsentResponse2 {
    let allowed = matches!(decision, PolicyDecision::Proceed);
    log::debug!(
        "{LOG_PREFIX} keyring consent access={} allowed={allowed}",
        match request.access {
            KeyringAccess2::Read => "read",
            KeyringAccess2::Write => "write",
            KeyringAccess2::Delete => "delete",
        }
    );
    KeyringConsentResponse2 { allowed }
}

/// The `Consent` callback body: the requested backend and the recorded consent.
///
/// `backend` is the one the module's keyring was configured with, bound to the
/// served callback object itself (never a process global), so each callback
/// answers for exactly the module configuration it was installed for. `None`
/// means no keyring is configured and every request is refused.
pub(crate) fn handle_keyring_consent(
    backend: Option<&KeyringBackend2>,
    request: &KeyringConsentRequest2,
) -> KeyringConsentResponse2 {
    log::debug!(
        "{LOG_PREFIX} keyring consent backend={}",
        backend.map_or("none", backend_kind)
    );
    keyring_consent_response(
        request,
        consent_decision(
            backend,
            crate::security::keyring_consent::policy::recorded_consent_decision,
        ),
    )
}

#[cfg(test)]
#[path = "security_crypto_tests.rs"]
mod tests;
